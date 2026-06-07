//! Date-partition bundling.
//!
//! After a cycle finishes, the user can opt in to a single
//! `<output-dir>/YYYY-MM-DD.tar.gz` archive containing the date
//! directory's images and sidecars. The archive is written with
//! `tar` + `flate2` (gzip) — a small dependency footprint and
//! ubiquitous format on Hugging Face.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use flate2::Compression;
use flate2::write::GzEncoder;
use thiserror::Error;
use tracing::info;

use crate::storage::partition_dir;

#[derive(Debug, Error)]
pub enum BundleError {
	#[error("failed to open tarball {path}: {source}")]
	Open { path: PathBuf, source: io::Error },

	#[error("failed to add {entry} to tarball: {source}")]
	Append { entry: PathBuf, source: io::Error },

	#[error("failed to finish tarball: {source}")]
	Finish { source: io::Error },
}

/// Tar+gzip the contents of `<base>/<date>/` into
/// `<base>/<date>.tar.gz`. The archive's top-level directory is
/// the date string (e.g. `2026-06-07/12345.jpeg`) so the bundle
/// extracts to a clean directory on the other end.
///
/// If the date directory does not exist (first-run edge case) the
/// bundle is still created, as an effectively-empty tarball. This
/// is fine — the user can see the file exists and inspect.
pub fn create_tarball(base: &Path, date: NaiveDate) -> Result<PathBuf, BundleError> {
	let partition = partition_dir(base, date);
	let tarball_path = base.join(format!("{}.tar.gz", date.format("%Y-%m-%d")));

	let file = File::create(&tarball_path).map_err(|source| BundleError::Open {
		path: tarball_path.clone(),
		source,
	})?;
	let enc = GzEncoder::new(file, Compression::default());
	let mut tar = tar::Builder::new(enc);

	if partition.is_dir() {
		// `append_dir_all` reads every file under `partition` and
		// adds it to the archive with the date string as the
		// archive's root directory.
		tar.append_dir_all(date.format("%Y-%m-%d").to_string(), &partition)
			.map_err(|source| BundleError::Append {
				entry: partition.clone(),
				source,
			})?;
	}

	tar.finish()
		.map_err(|source| BundleError::Finish { source })?;
	// Flush the gzip wrapper before reporting success.
	let enc = tar.into_inner().map_err(|e| BundleError::Finish {
		source: io::Error::other(e),
	})?;
	enc.finish()
		.map_err(|source| BundleError::Finish { source })?;

	let size = std::fs::metadata(&tarball_path)
		.map(|m| m.len())
		.unwrap_or(0);
	info!(path = %tarball_path.display(), bytes = size, "bundle written");

	Ok(tarball_path)
}
