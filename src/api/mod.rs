//! CivitAI REST client.
//!
//! One async function (`fetch_popular`) does the work for the
//! `/api/v1/images` endpoint, with exponential backoff on 429 / 5xx
//! responses (1s → 2s → 4s, up to 3 retries, max 4 attempts total —
//! see PLAN.md §2).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::sleep;
use tracing::debug;

use crate::cli::{NsfwLevel, browsing_level_bitmask};

const API_BASE: &str = "https://civitai.com/api/v1/images";
const USER_AGENT: &str = concat!("civistash/", env!("CARGO_PKG_VERSION"));

// ── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ApiError {
	#[error("HTTP {status} after {attempts} attempt(s): {body}")]
	HttpStatus {
		status: u16,
		attempts: u32,
		body: String,
	},

	#[error("rate-limited after {attempts} attempt(s)")]
	RateLimited { attempts: u32 },

	/// Short, user-facing description of a transport-level failure
	/// (timeout, connection refused, decode failure, …). We deliberately
	/// do not store the full `reqwest::Error` here because its
	/// `Display` nests the URL three levels deep, producing an
	/// unreadable error line. The full chain is still available via
	/// `RUST_LOG=civistash=trace`.
	#[error("transport error: {0}")]
	Transport(String),

	#[error("failed to parse API response: {0}")]
	Parse(#[from] serde_json::Error),
}

/// Route any `reqwest::Error` that escapes via `?` (e.g. from
/// `resp.json().await?`) through the same user-friendly description
/// we use for transport errors during the retry loop.
impl From<reqwest::Error> for ApiError {
	fn from(e: reqwest::Error) -> Self {
		ApiError::Transport(describe_transport_error(e))
	}
}

/// Collapse a `reqwest::Error` to a single-line, user-readable
/// description. Drops the URL (already implicit from the CLI context)
/// and the nested error chain.
///
/// Takes ownership because `Error::without_url` consumes `self`. The
/// caller has the `reqwest::Error` in hand either way.
fn describe_transport_error(e: reqwest::Error) -> String {
	if e.is_timeout() {
		"request timed out".to_string()
	} else if e.is_connect() {
		"connection failed".to_string()
	} else if e.is_decode() {
		"response body could not be decoded".to_string()
	} else if e.is_redirect() {
		"too many redirects".to_string()
	} else if e.is_request() {
		"request could not be sent".to_string()
	} else if e.is_body() {
		"request body could not be built".to_string()
	} else if e.is_builder() {
		"request builder error".to_string()
	} else {
		// Fallback — still a single line, URL stripped.
		e.without_url().to_string()
	}
}

// ── Public types ───────────────────────────────────────────────────────────

/// Per-image statistics block in the API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageStats {
	pub cry_count: Option<u32>,
	pub laugh_count: Option<u32>,
	pub like_count: Option<u32>,
	pub dislike_count: Option<u32>,
	pub heart_count: Option<u32>,
	pub comment_count: Option<u32>,
	#[serde(flatten)]
	pub extra: serde_json::Value,
}

/// A single image object in the response. All API fields not modelled
/// explicitly are captured in `extra` so the sidecar is a complete
/// verbatim copy of the upstream JSON.
///
/// A handful of fields (`nsfw_level`, `username`, `base_model`) are
/// captured as raw `serde_json::Value` because CivitAI has been seen
/// returning them as different JSON types depending on the image
/// (e.g. `username` is a string for normal users but an integer ID
/// for deleted/anonymous accounts; `nsfw_level` is sometimes a
/// string like `"None"`, sometimes an integer). The original type
/// is preserved in the sidecar so no information is lost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
	pub id: i64,
	pub url: String,
	pub hash: Option<String>,
	pub width: Option<u32>,
	pub height: Option<u32>,
	#[serde(rename = "type")]
	pub media_type: Option<String>,
	pub nsfw: Option<bool>,
	pub nsfw_level: Option<serde_json::Value>,
	pub browsing_level: Option<i32>,
	pub created_at: Option<String>,
	pub post_id: Option<i64>,
	pub username: Option<serde_json::Value>,
	pub base_model: Option<serde_json::Value>,
	#[serde(default)]
	pub model_version_ids: Vec<i64>,
	pub stats: Option<ImageStats>,
	pub meta: Option<serde_json::Value>,

	/// Any fields not explicitly modelled land here. The sidecar
	/// writer flattens this back out, so the on-disk JSON looks
	/// identical to the API response.
	#[serde(flatten)]
	pub extra: serde_json::Value,
}

impl Image {
	/// True if this entry should be filtered out of the cycle. The
	/// `allow_all` flag toggles whether any non-`image` media type
	/// (videos, audio, unknown, …) is kept. With `allow_all = false`
	/// (the default), only `media_type = "image"` (or absent) is
	/// kept.
	pub fn is_filtered(&self, allow_all: bool) -> bool {
		match self.media_type.as_deref() {
			Some("image") | None => false,
			_ => !allow_all,
		}
	}
}

/// The full `/api/v1/images` response.
#[derive(Debug, Deserialize)]
pub struct ImagesResponse {
	pub items: Vec<Image>,
	/// Pagination block. The `nextCursor` field (a string) is the
	/// opaque cursor for the next page; an absent or empty value
	/// signals "no more pages".
	#[serde(default)]
	pub metadata: serde_json::Value,
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Maximum items CivitAI's `/api/v1/images` returns in a single
/// request. When the user asks for more, the public `fetch_popular`
/// transparently pages with `cursor`.
const PER_PAGE_MAX: u32 = 200;

/// Fetch the top *N* popular images from CivitAI.
///
/// `limit` is the **total** number of images desired. If it exceeds
/// `PER_PAGE_MAX`, this function makes multiple requests chained by
/// the `cursor` query parameter until either the limit is reached
/// or the API signals no more pages (empty/absent `nextCursor`).
///
/// Each individual page request honours the same 1 s / 2 s / 4 s
/// backoff (3 retries) on 429, 5xx, and transport errors as the
/// original single-request design. A failed page aborts the whole
/// fetch — we do not return partial results across pages.
pub async fn fetch_popular(
	client: &reqwest::Client,
	ca_token: Option<&str>,
	period: crate::cli::Period,
	sort: crate::cli::SortOrder,
	nsfw_level: &[NsfwLevel],
	limit: u32,
) -> Result<Vec<Image>, ApiError> {
	if limit == 0 {
		return Ok(Vec::new());
	}

	let per_page = limit.min(PER_PAGE_MAX);
	let mut all_items: Vec<Image> = Vec::with_capacity(limit as usize);
	let mut cursor: Option<String> = None;

	loop {
		let (page_items, next_cursor) = fetch_one_page(
			client,
			ca_token,
			period,
			sort,
			nsfw_level,
			per_page,
			cursor.as_deref(),
		)
		.await?;

		// Don't over-collect on the last page.
		let remaining = (limit as usize).saturating_sub(all_items.len());
		all_items.extend(page_items.into_iter().take(remaining));

		debug!(
			total = all_items.len(),
			limit,
			has_next_page = next_cursor.is_some(),
			"fetched one page"
		);

		// Stop if we have enough, or the API has no more pages.
		if all_items.len() >= limit as usize {
			break;
		}
		match next_cursor {
			Some(c) if !c.is_empty() => cursor = Some(c),
			_ => break,
		}
	}

	Ok(all_items)
}

/// Single request to `/api/v1/images` with the given `cursor` (or
/// `None` for the first page). Returns the parsed items and the
/// API-supplied cursor for the *next* page (if any).
async fn fetch_one_page(
	client: &reqwest::Client,
	ca_token: Option<&str>,
	period: crate::cli::Period,
	sort: crate::cli::SortOrder,
	nsfw_level: &[NsfwLevel],
	per_page: u32,
	cursor: Option<&str>,
) -> Result<(Vec<Image>, Option<String>), ApiError> {
	let mut url = reqwest::Url::parse(API_BASE).expect("API_BASE is a hardcoded constant");
	{
		let mut qp = url.query_pairs_mut();
		qp.append_pair("sort", &sort.to_string());
		qp.append_pair("period", &period.to_string());
		qp.append_pair("limit", &per_page.to_string());
		// PLAN.md §8.2 — always request generation metadata.
		qp.append_pair("withMeta", "true");
		if !nsfw_level.is_empty() {
			qp.append_pair(
				"browsingLevel",
				&browsing_level_bitmask(nsfw_level).to_string(),
			);
		}
		if let Some(c) = cursor {
			qp.append_pair("cursor", c);
		}
	}

	let backoff = [
		Duration::from_secs(1),
		Duration::from_secs(2),
		Duration::from_secs(4),
	];
	let mut last_status: Option<(u16, String)> = None;
	let mut rate_limited = false;

	for attempt in 0..=backoff.len() {
		let mut req = client
			.get(url.as_str())
			.header(reqwest::header::USER_AGENT, USER_AGENT)
			.header(reqwest::header::ACCEPT, "application/json");
		if let Some(t) = ca_token {
			req = req.bearer_auth(t);
		}

		// Send + handle transport errors. Plan §2 says "Honour HTTP 429
		// and 5xx responses" but a transport-level timeout is morally
		// the same as a 5xx from the client's perspective, so it gets
		// the same retry treatment.
		let resp = match req.send().await {
			Ok(r) => r,
			Err(e) => {
				let desc = describe_transport_error(e);
				debug!(
					attempt = attempt + 1,
					error = %desc,
					"transport error, evaluating backoff"
				);
				if attempt < backoff.len() {
					sleep(backoff[attempt]).await;
					continue;
				}
				return Err(ApiError::Transport(desc));
			}
		};
		let status = resp.status();

		if status.is_success() {
			let parsed: ImagesResponse = resp.json().await?;
			let next_cursor = extract_next_cursor(&parsed.metadata);
			return Ok((parsed.items, next_cursor));
		}

		// Capture the body (truncated) for diagnostics, but never let a
		// body read loop forever.
		let body = resp
			.text()
			.await
			.unwrap_or_default()
			.chars()
			.take(512)
			.collect::<String>();

		debug!(
			attempt = attempt + 1,
			status = status.as_u16(),
			"request failed; evaluating backoff"
		);

		let is_retryable = status.as_u16() == 429 || status.is_server_error();
		last_status = Some((status.as_u16(), body));
		if status.as_u16() == 429 {
			rate_limited = true;
		}

		if !is_retryable || attempt == backoff.len() {
			break;
		}

		sleep(backoff[attempt]).await;
	}

	// All retries exhausted.
	let (status, body) = last_status.expect("loop ran at least once on non-success");
	if rate_limited {
		Err(ApiError::RateLimited {
			attempts: (backoff.len() + 1) as u32,
		})
	} else {
		Err(ApiError::HttpStatus {
			status,
			attempts: (backoff.len() + 1) as u32,
			body,
		})
	}
}

/// Extract the `nextCursor` field from the response metadata, or
/// `None` if absent / empty. The API uses an empty string (rather
/// than `null`) to signal "no more pages" on the last page, so we
/// filter both.
fn extract_next_cursor(metadata: &serde_json::Value) -> Option<String> {
	metadata
		.get("nextCursor")
		.and_then(|v| v.as_str())
		.filter(|s| !s.is_empty())
		.map(String::from)
}
