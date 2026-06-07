//! Per-cycle orchestrator. The daemon passes an optional shutdown
//! flag that the cycle checks between images, so a signal received
//! during a cycle lets the current image finish before returning.

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

/// Run one fetch cycle and return its per-image counters.
///
/// When `shutdown` is `Some` and its flag is set, the cycle finishes
/// the current image (writing its sidecar) and returns. The in-flight
/// HTTP request is not aborted.
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
		all_types = cli.all_types,
		"fetching popular images"
	);

	let items = api::fetch_popular(
		client,
		cli.ca_token.as_deref(),
		cli.period,
		cli.sort,
		&cli.nsfw_level,
		cli.limit,
	)
	.await
	.inspect_err(|e| warn!(error = %e, "API fetch failed"))?;

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

		if image.is_filtered(cli.all_types) {
			stats.filtered += 1;
			let name = display_name(image);
			let ty = image.media_type.as_deref().unwrap_or("unknown");
			ui::status(
				SYM_FILTER,
				DIM,
				format!("Filtered    {name:<14}  (type={ty})"),
			);
			continue;
		}

		if storage::id_exists_anywhere(base, image.id) {
			stats.skipped += 1;
			let name = display_name(image);
			ui::status(
				SYM_SKIP,
				DIM,
				format!("Skipped     {name:<14}  (already present)"),
			);
			continue;
		}

		let name = display_name(image);
		ui::status(
			SYM_PROGRESS,
			EMBER,
			format!("Downloading {name:<14}  {}", format_meta(image)),
		);

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
								"Downloaded  {name:<14}  → {} ({})",
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
							format!("Sidecar     {name:<14}  ({e})"),
						);
					}
				}
			}
			DownloadOutcome::Skipped => {
				stats.skipped += 1;
				ui::status(
					SYM_SKIP,
					DIM,
					format!("Skipped     {name:<14}  (already present)"),
				);
			}
			DownloadOutcome::Failed(reason) => {
				stats.failed += 1;
				ui::status(
					SYM_FAIL,
					BRICK,
					format!("Failed      {name:<14}  ({reason})"),
				);
			}
		}
	}

	stats.duration = start.elapsed();

	// Bundle and upload run only on the happy path; an earlier `?`
	// skips both.
	if cli.bundle {
		match crate::bundle::create_tarball(base, date) {
			Ok(tarball) => ui::status(
				SYM_OK,
				FOREST,
				format!(
					"Bundled     → {}",
					tarball.display().to_string().style(EMBER)
				),
			),
			Err(e) => ui::status(SYM_FAIL, BRICK, format!("Bundle failed: {e}")),
		}
	}
	if let Some(repo) = &cli.upload_hf {
		let tarball = base.join(format!("{}.tar.gz", date.format("%Y-%m-%d")));
		match crate::upload::upload_to_hf(&tarball, repo, cli.hf_token.as_deref()).await {
			Ok(()) => ui::status(SYM_OK, FOREST, format!("Uploaded    → hf:{repo}")),
			Err(e) => ui::status(SYM_FAIL, BRICK, format!("Upload failed: {e}")),
		}
	}

	Ok(stats)
}

/// Compact `<w>×<h>` shown in the "Downloading …" line.
fn format_meta(image: &Image) -> String {
	match (image.width, image.height) {
		(Some(w), Some(h)) => format!("{w}×{h}"),
		_ => String::new(),
	}
}

/// `<id>.<ext>` as written to disk. Kept as one helper so the three
/// call sites can't drift.
fn display_name(image: &Image) -> String {
	format!("{}.{}", image.id, extension_from_url(&image.url))
}
