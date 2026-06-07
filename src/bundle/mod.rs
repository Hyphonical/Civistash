use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use flate2::write::GzEncoder;
use flate2::Compression;
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

/// Tar+gzip the contents of `<base>/<date>/` into `<base>/<date>.tar.gz`.
///
/// The archive's top-level directory is the date string (e.g. `2026-06-07/`)
/// so the bundle extracts to a clean directory. If the date directory does
/// not exist (first run), an effectively-empty tarball is still created.
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
		tar.append_dir_all(date.format("%Y-%m-%d").to_string(), &partition)
			.map_err(|source| BundleError::Append {
				entry: partition.clone(),
				source,
			})?;
	}

	tar.finish()
		.map_err(|source| BundleError::Finish { source })?;
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
