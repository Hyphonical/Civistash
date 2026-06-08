use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;
use tracing::debug;

use crate::api::Image;
use crate::storage::extension_from_url;

const USER_AGENT: &str = concat!("Civistash/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub enum DownloadOutcome {
	Ok(PathBuf),
	Skipped,
	Failed(String),
}

#[derive(Debug, Error)]
enum DownloadError {
	#[error("HTTP {status} after {attempts} attempt(s)")]
	Http { status: u16, attempts: u32 },

	#[error("transport error: {0}")]
	Transport(#[from] reqwest::Error),

	#[error("io error: {0}")]
	Io(#[from] std::io::Error),
}

/// Download one image to `.partial`, then rename on success. Exits early
/// if the final path already exists. Cleans up the partial on failure.
pub async fn download_image(
	client: &reqwest::Client,
	image: &Image,
	dest_dir: &Path,
) -> DownloadOutcome {
	let ext = extension_from_url(&image.url);
	let final_path = dest_dir.join(format!("{}.{}", image.id, ext));
	let partial_path = dest_dir.join(format!("{}.{}.partial", image.id, ext));

	if final_path.exists() {
		return DownloadOutcome::Skipped;
	}

	let backoff = [
		Duration::from_secs(1),
		Duration::from_secs(2),
		Duration::from_secs(4),
	];

	for attempt in 0..=backoff.len() {
		match try_once(client, &image.url, &partial_path).await {
			Ok(()) => match fs::rename(&partial_path, &final_path).await {
				Ok(()) => return DownloadOutcome::Ok(final_path),
				Err(e) => return DownloadOutcome::Failed(format!("rename failed: {e}")),
			},
			Err(e) => {
				debug!(
					attempt = attempt + 1,
					url = %image.url,
					error = %e,
					"download attempt failed"
				);
				let is_retryable = matches!(&e, DownloadError::Http { status, .. } if *status == 429 || (500..600).contains(status))
					|| matches!(&e, DownloadError::Transport(_));
				if !is_retryable || attempt == backoff.len() {
					let _ = fs::remove_file(&partial_path).await;
					return DownloadOutcome::Failed(e.to_string());
				}
				sleep(backoff[attempt]).await;
			}
		}
	}

	// Unreachable: the loop either returns or retries to exhaustion.
	let _ = fs::remove_file(&partial_path).await;
	DownloadOutcome::Failed("retry budget exhausted".to_string())
}

/// Single attempt. Streams the response body into `.partial`.
async fn try_once(
	client: &reqwest::Client,
	url: &str,
	partial: &Path,
) -> Result<(), DownloadError> {
	let resp = client
		.get(url)
		.header(reqwest::header::USER_AGENT, USER_AGENT)
		.send()
		.await?;
	let status = resp.status();
	if !status.is_success() {
		return Err(DownloadError::Http {
			status: status.as_u16(),
			attempts: 1,
		});
	}

	let mut file = fs::File::create(partial).await?;
	let mut stream = resp.bytes_stream();
	while let Some(chunk) = stream.next().await {
		let bytes = chunk?;
		file.write_all(&bytes).await?;
	}
	file.flush().await?;
	Ok(())
}
