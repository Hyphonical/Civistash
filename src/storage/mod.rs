//! Filesystem layout, deduplication, and sidecar writer.
//!
//! Dedup is the entire deduplication mechanism described in PLAN.md
//! §5.5: walk the date-partitioned output directory, look for any
//! file whose stem matches the CivitAI image ID. No manifest, no
//! database.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{Days, NaiveDate, Utc};
use serde_json::json;

use crate::api::Image;

/// All filesystem-level errors raised by this module.
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

/// Return `<base>/YYYY-MM-DD/` for the given local date. The date
/// directory is **not** created by this function — callers do that
/// lazily so an empty cycle does not leave an empty folder behind.
pub fn partition_dir(base: &Path, date: NaiveDate) -> PathBuf {
	base.join(date.format("%Y-%m-%d").to_string())
}

/// Convenience wrapper that resolves today's local date partition.
#[allow(dead_code)] // public API per PLAN.md §5.5
pub fn today_partition_dir(base: &Path) -> PathBuf {
	partition_dir(base, Utc::now().date_naive())
}

/// Ensure a directory exists, including its parents. Idempotent.
pub fn ensure_dir(path: &Path) -> Result<(), StorageError> {
	if let Err(source) = fs::create_dir_all(path) {
		// `AlreadyExists` is not an error — a concurrent create_dir_all
		// by another process or a stale symlink can surface it.
		if source.kind() != ErrorKind::AlreadyExists {
			return Err(StorageError::DirCreate {
				path: path.to_path_buf(),
				source,
			});
		}
	}
	Ok(())
}

/// Walk all immediate subdirectories of `base` and return `true` if
/// any file has a stem that matches the CivitAI image ID. This
/// excludes in-flight `.partial` files (see PLAN.md §8.4) and tolerates
/// a non-existent base directory (returns `false`).
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
				// Match either the bare id (sidecar) or `{id}.{ext}` (image).
				if stem == id_str || stem.starts_with(&format!("{id}.")) {
					return true;
				}
			}
		}
	}
	false
}

/// Infer the file extension from a CDN URL. Falls back to `bin` if the
/// URL has no recognisable extension. See PLAN.md §8.3 — the URL
/// path is the chosen source.
pub fn extension_from_url(url: &str) -> String {
	let path = url.split('?').next().unwrap_or(url);
	let last_segment = path.rsplit('/').next().unwrap_or(path);
	let ext = last_segment.rsplit('.').next().unwrap_or("bin");
	if ext.is_empty() || ext.contains('/') || ext.len() > 5 {
		return "bin".to_string();
	}
	ext.to_ascii_lowercase()
}

/// Serialise the image and the three enrichment fields (PLAN.md §6.3)
/// into a single `serde_json::Value` and write it to
/// `<partition>/<id>.json`. The image's original API fields are
/// preserved verbatim via `Image::extra`.
pub fn write_sidecar(
	base: &Path,
	date: NaiveDate,
	image: &Image,
	local_path: &Path,
	source_url: &str,
) -> Result<PathBuf, StorageError> {
	let partition = partition_dir(base, date);
	ensure_dir(&partition)?;

	// Start from the image's own JSON representation, then layer the
	// three enrichment fields on top so they take precedence on key
	// collision. (Collisions are not expected from the live API, but
	// defending against them is cheap.)
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

/// Delete any `*.partial` files in the given date partition. Called
/// during graceful daemon shutdown per PLAN.md §8.4 option 2.
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

/// Delete the tarball file at `path`. Idempotent — does nothing if the
/// file is absent. Errors are silently ignored (logged as a warning).
pub fn delete_tarball(path: &Path) {
	match fs::remove_file(path) {
		Ok(()) => tracing::info!(path = %path.display(), "deleted tarball"),
		Err(e) if e.kind() == ErrorKind::NotFound => {}
		Err(e) => tracing::warn!(path = %path.display(), error = %e, "failed to delete tarball"),
	}
}

/// Delete date partitions **older than** `keep_days` days from `base`.
///
/// CivitAI's `period=Day` is a rolling 24h window, not a calendar-day
/// reset, so today's top 100 can overlap with yesterday's. Keeping
/// today + yesterday (`keep_days=2`) means the dedup scan still finds
/// overlapping entries if the daemon restarts mid-cycle.
///
/// Example with `keep_days=2` on June 9:
///
/// - Keeps `2026-06-08/` and `2026-06-09/`
/// - Deletes `2026-06-07/` and older
pub fn delete_old_partitions(base: &Path, today: NaiveDate, keep_days: u32) {
	let cutoff = match today.checked_sub_days(Days::new(keep_days as u64)) {
		Some(d) => d,
		None => return, // today earlier than keep_days → nothing to delete
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
		// Only match "YYYY-MM-DD" directories (our date partitions).
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
