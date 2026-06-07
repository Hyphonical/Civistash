//! Per-cycle orchestrator.
//!
//! This is the reusable unit called by both one-shot and daemon
//! modes (PLAN.md §4 + §6.1). It is intentionally ignorant of the
//! outer loop and signal handling — the daemon passes an optional
//! `Arc<AtomicBool>` shutdown flag that the cycle checks between
//! images, so a SIGTERM during a cycle lets the current image finish
//! and its sidecar land on disk before returning.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use owo_colors::OwoColorize;
use tracing::{info, warn};

use crate::api::{self, Image};
use crate::cli::Cli;
use crate::download::{self, DownloadOutcome};
use crate::storage::{self, extension_from_url};
use crate::ui::{
	self, BRICK, CycleStats, DIM, EMBER, FOREST, SYM_FAIL, SYM_FILTER, SYM_OK, SYM_PROGRESS,
	SYM_SKIP, humanize_bytes,
};

/// Run one fetch cycle. Returns the per-cycle stats; the caller is
/// responsible for printing the summary.
///
/// `shutdown`, if supplied, is checked between images. When it flips
/// to `true` the cycle finishes the current image, writes its
/// sidecar, and returns. The in-flight HTTP request is not aborted —
/// the plan's success criteria #5 requires the current image to
/// finish before exit.
pub async fn run_cycle(
	client: &reqwest::Client,
	cli: &Cli,
	shutdown: Option<&Arc<AtomicBool>>,
) -> Result<CycleStats> {
	let start = Instant::now();
	let mut stats = CycleStats::default();

	info!(
		limit = cli.limit,
		period = %cli.period,
		sort = %cli.sort,
		video = cli.video,
		"fetching popular images"
	);

	let items = match api::fetch_popular(
		client,
		cli.token.as_deref(),
		cli.period,
		cli.sort,
		&cli.nsfw_level,
		cli.limit,
	)
	.await
	{
		Ok(items) => items,
		Err(e) => {
			warn!(error = %e, "API fetch failed");
			return Err(e.into());
		}
	};

	if items.is_empty() {
		info!("API returned 0 images");
		stats.duration = start.elapsed();
		return Ok(stats);
	}

	let base = &cli.output_dir;
	let date = chrono::Utc::now().date_naive();
	let partition = storage::partition_dir(base, date);
	storage::ensure_dir(&partition)?;

	for image in &items {
		if let Some(flag) = shutdown
			&& flag.load(Ordering::Relaxed)
		{
			ui::status(
				ui::SYM_DIVIDER,
				DIM,
				"shutdown requested, finishing current image",
			);
			break;
		}

		// (a) Filter non-image media. Videos are kept only if the
		// user opted in with --video; audio and other types are
		// always filtered in v0.1.0.
		if image.is_filtered(cli.video) {
			stats.filtered += 1;
			let name = format!("{}.{}", image.id, extension_from_url(&image.url));
			let ty = image.media_type.as_deref().unwrap_or("unknown");
			ui::status(
				SYM_FILTER,
				DIM,
				format!("Filtered    {:<14}  (type={})", name, ty),
			);
			continue;
		}

		// (b) Dedup by CivitAI image ID.
		if storage::id_exists_anywhere(base, image.id) {
			stats.skipped += 1;
			let name = format!("{}.{}", image.id, extension_from_url(&image.url));
			ui::status(
				SYM_SKIP,
				DIM,
				format!("Skipped     {:<14}  (already present)", name),
			);
			continue;
		}

		// (c) Announce, then download.
		let name = format!("{}.{}", image.id, extension_from_url(&image.url));
		ui::status(
			SYM_PROGRESS,
			EMBER,
			format!("Downloading {:<14}  {}", name, format_meta(image)),
		);

		// (d) Download + (e) sidecar.
		match download::download_image(client, image, &partition).await {
			DownloadOutcome::Ok(final_path) => {
				match storage::write_sidecar(base, date, image, &final_path, &image.url) {
					Ok(_sidecar) => {
						stats.downloaded += 1;
						let rel = final_path
							.strip_prefix(base)
							.unwrap_or(&final_path)
							.to_string_lossy()
							.into_owned();
						let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
						ui::status(
							SYM_OK,
							FOREST,
							format!(
								"Downloaded  {:<14}  → {} ({})",
								name,
								rel.style(EMBER),
								humanize_bytes(size).bold()
							),
						);
					}
					Err(e) => {
						stats.failed += 1;
						ui::status(
							SYM_FAIL,
							BRICK,
							format!("Sidecar     {:<14}  ({})", name, e),
						);
					}
				}
			}
			DownloadOutcome::Skipped => {
				stats.skipped += 1;
				ui::status(
					SYM_SKIP,
					DIM,
					format!("Skipped     {:<14}  (already present)", name),
				);
			}
			DownloadOutcome::Failed(reason) => {
				stats.failed += 1;
				ui::status(
					SYM_FAIL,
					BRICK,
					format!("Failed      {:<14}  ({})", name, reason),
				);
			}
		}
	}

	stats.duration = start.elapsed();
	Ok(stats)
}

/// Compact metadata shown in the "Downloading …" line. Kept terse
/// so it doesn't push the next column off-screen.
fn format_meta(image: &Image) -> String {
	match (image.width, image.height) {
		(Some(w), Some(h)) => format!("{}×{}", w, h),
		_ => String::new(),
	}
}
