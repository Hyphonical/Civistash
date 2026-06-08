use std::fs;
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
/// Entries are added at the **tarball root** with their basename only
/// (e.g. `12345.png`, `12345.json`) — no date directory prefix. This makes
/// the archive a valid WebDataset: paired files share the same stem and
/// the Hugging Face dataset viewer can group them into rows.
///
/// `archive_date` is recorded inside each sidecar JSON's `_civistash`
/// block by `storage::write_sidecar`, so the date is preserved without
/// the directory prefix.
///
/// If the date directory does not exist (first run), an effectively-empty
/// tarball is still created.
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
		let entries = fs::read_dir(&partition).map_err(|source| BundleError::Append {
			entry: partition.clone(),
			source,
		})?;
		for entry in entries {
			let entry = entry.map_err(|source| BundleError::Append {
				entry: partition.clone(),
				source,
			})?;
			let path = entry.path();
			if !path.is_file() {
				continue;
			}
			let name = path.file_name().ok_or_else(|| BundleError::Append {
				entry: path.clone(),
				source: io::Error::new(io::ErrorKind::InvalidInput, "missing filename"),
			})?;
			tar.append_path_with_name(&path, name)
				.map_err(|source| BundleError::Append {
					entry: path.clone(),
					source,
				})?;
		}
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
