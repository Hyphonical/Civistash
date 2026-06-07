use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{Days, NaiveDate, Utc};
use serde_json::json;

use crate::api::Image;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
	#[error("failed to create directory {path}: {source}")]
	DirCreate {
		path: PathBuf,
		source: std::io::Error,
	},

	#[error("failed to write sidecar {path}: {source}")]
	SidecarWrite {
		path: PathBuf,
		source: std::io::Error,
	},
}

/// Return `<base>/YYYY-MM-DD/`. The directory is not created.
pub fn partition_dir(base: &Path, date: NaiveDate) -> PathBuf {
	base.join(date.format("%Y-%m-%d").to_string())
}

#[allow(dead_code)]
pub fn today_partition_dir(base: &Path) -> PathBuf {
	partition_dir(base, Utc::now().date_naive())
}

/// Ensure a directory exists, including parents. Idempotent.
pub fn ensure_dir(path: &Path) -> Result<(), StorageError> {
	if let Err(source) = fs::create_dir_all(path) {
		if source.kind() != ErrorKind::AlreadyExists {
			return Err(StorageError::DirCreate {
				path: path.to_path_buf(),
				source,
			});
		}
	}
	Ok(())
}

/// Walk all immediate subdirectories of `base` and return true if any
/// file has a stem matching the CivitAI image ID. Excludes `.partial`
/// files. Returns false if `base` doesn't exist.
pub fn id_exists_anywhere(base: &Path, id: i64) -> bool {
	let id_str = id.to_string();
	let Ok(entries) = fs::read_dir(base) else {
		return false;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_dir() {
			continue;
		}
		let Ok(sub) = fs::read_dir(&path) else {
			continue;
		};
		for sub_entry in sub.flatten() {
			let p = sub_entry.path();
			if p.extension().and_then(|s| s.to_str()) == Some("partial") {
				continue;
			}
			if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
				if stem == id_str || stem.starts_with(&format!("{id}.")) {
					return true;
				}
			}
		}
	}
	false
}

/// Infer the file extension from a CDN URL. Falls back to `bin` if the
/// URL has no recognisable extension.
pub fn extension_from_url(url: &str) -> String {
	let path = url.split('?').next().unwrap_or(url);
	let last_segment = path.rsplit('/').next().unwrap_or(path);
	let ext = last_segment.rsplit('.').next().unwrap_or("bin");
	if ext.is_empty() || ext.contains('/') || ext.len() > 5 {
		return "bin".to_string();
	}
	ext.to_ascii_lowercase()
}

/// Serialise the image + enrichment fields into `<partition>/<id>.json`.
pub fn write_sidecar(
	base: &Path,
	date: NaiveDate,
	image: &Image,
	local_path: &Path,
	source_url: &str,
) -> Result<PathBuf, StorageError> {
	let partition = partition_dir(base, date);
	ensure_dir(&partition)?;

	let mut value = serde_json::to_value(image)
		.expect("Image serialisation should never fail (no Map<non-string> keys)");
	if let Some(obj) = value.as_object_mut() {
		obj.insert(
			"downloaded_at".to_string(),
			json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
		);
		obj.insert(
			"local_path".to_string(),
			json!(local_path.to_string_lossy()),
		);
		obj.insert("source_url".to_string(), json!(source_url));
	}

	let sidecar_path = partition.join(format!("{}.json", image.id));
	let bytes = match serde_json::to_vec_pretty(&value) {
		Ok(b) => b,
		Err(e) => {
			return Err(StorageError::SidecarWrite {
				path: sidecar_path,
				source: std::io::Error::new(ErrorKind::InvalidData, e),
			});
		}
	};
	fs::write(&sidecar_path, bytes).map_err(|source| StorageError::SidecarWrite {
		path: sidecar_path.clone(),
		source,
	})?;
	Ok(sidecar_path)
}

/// Delete any `*.partial` files in the given date partition.
pub fn cleanup_partials(partition: &Path) {
	let Ok(entries) = fs::read_dir(partition) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension().and_then(|s| s.to_str()) == Some("partial") {
			let _ = fs::remove_file(&path);
		}
	}
}

/// Delete the tarball at `path`. Idempotent.
pub fn delete_tarball(path: &Path) {
	match fs::remove_file(path) {
		Ok(()) => tracing::info!(path = %path.display(), "deleted tarball"),
		Err(e) if e.kind() == ErrorKind::NotFound => {}
		Err(e) => tracing::warn!(path = %path.display(), error = %e, "failed to delete tarball"),
	}
}

/// Delete date partitions older than `keep_days` days from `today`.
///
/// `keep_days=2` preserves today + yesterday for the rolling 24h window
/// CivitAI uses with `period=Day`.
pub fn delete_old_partitions(base: &Path, today: NaiveDate, keep_days: u32) {
	let cutoff = match today.checked_sub_days(Days::new(keep_days as u64)) {
		Some(d) => d,
		None => return,
	};

	let Ok(entries) = fs::read_dir(base) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_dir() {
			continue;
		}
		let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
			continue;
		};
		if name.len() != 10 || !name.starts_with("20") {
			continue;
		}
		let Ok(dir_date) = NaiveDate::parse_from_str(name, "%Y-%m-%d") else {
			continue;
		};
		if dir_date <= cutoff {
			tracing::info!(path = %path.display(), "deleting old partition");
			if let Err(e) = fs::remove_dir_all(&path) {
				tracing::warn!(path = %path.display(), error = %e, "failed to delete partition");
			}
		}
	}
}

/// Recursively sum file sizes under `base`. Returns 0 if the directory
/// doesn't exist.
pub fn disk_usage(base: &Path) -> u64 {
	let mut total = 0u64;
	walk_disk_usage(base, &mut total);
	total
}

fn walk_disk_usage(dir: &Path, total: &mut u64) {
	let Ok(entries) = fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		match path.metadata() {
			Ok(m) if m.is_dir() => walk_disk_usage(&path, total),
			Ok(m) => *total += m.len(),
			Err(_) => {}
		}
	}
}
