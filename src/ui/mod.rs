//! Terminal theme and progress symbols.
//!
//! The palette and symbol set are defined in PLAN.md §10.4. Every
//! status line in the application is rendered through this module so
//! that colour, symbol, and ordering stay consistent.

use std::time::Duration;

use owo_colors::{OwoColorize, Style};

/// Ember — primary emphasis: timestamps, paths, the app name.
pub const EMBER: Style = Style::new().fg_rgb::<217, 119, 87>();

/// Forest — success: `✓` marks, "complete" lines.
pub const FOREST: Style = Style::new().fg_rgb::<90, 138, 90>();

/// Brick — error: `✗` marks, error messages.
pub const BRICK: Style = Style::new().fg_rgb::<193, 69, 69>();

/// Dim — secondary: filtered/skipped items, "idle" markers.
pub const DIM: Style = Style::new().fg_rgb::<122, 122, 140>();

// ── Symbols ────────────────────────────────────────────────────────────────
//
// Single source of truth for the glyphs used in every status line. Keep
// them exactly as defined in PLAN.md §10.4 so log scrapers and colour
// schemes stay stable.

pub const SYM_OK: &str = "✓";
pub const SYM_FAIL: &str = "✗";
pub const SYM_SKIP: &str = "⦿";
pub const SYM_FILTER: &str = "⊘";
pub const SYM_PROGRESS: &str = "⤓";
pub const SYM_IDLE: &str = "…";
#[allow(dead_code)] // surfaced when retry attempts need a user-visible marker
pub const SYM_RETRY: &str = "↻";
pub const SYM_DIVIDER: &str = "■";

// ── Stats & summary ────────────────────────────────────────────────────────

/// Per-cycle counters. See PLAN.md §6.1 step 5 for the canonical shape.
#[derive(Debug, Default, Clone, Copy)]
pub struct CycleStats {
	pub downloaded: u32,
	pub skipped: u32,
	pub filtered: u32,
	pub failed: u32,
	pub duration: Duration,
}

/// Format a byte count as a human-readable string (`"1.4 MB"`,
/// `"230 B"`, etc.). Used by the cycle when reporting downloaded file
/// sizes in the post-download status line.
pub fn humanize_bytes(n: u64) -> String {
	const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
	let mut size = n as f64;
	let mut idx = 0;
	while size >= 1024.0 && idx < UNITS.len() - 1 {
		size /= 1024.0;
		idx += 1;
	}
	if idx == 0 {
		format!("{} {}", n, UNITS[0])
	} else {
		format!("{:.1} {}", size, UNITS[idx])
	}
}

// ── Summary printing ───────────────────────────────────────────────────────

/// Print the final per-cycle totals to stdout (not stderr) per the
/// status-reporting rules in PLAN.md §10.4.
pub fn print_cycle_summary(stats: &CycleStats) {
	println!();
	println!(
		"{} {}",
		SYM_DIVIDER.style(EMBER),
		"cycle complete".style(EMBER)
	);
	println!(
		"  {}  downloaded: {}",
		SYM_OK.style(FOREST),
		stats.downloaded.bold()
	);
	println!(
		"  {}  skipped:    {}",
		SYM_SKIP.style(DIM),
		stats.skipped.bold()
	);
	println!(
		"  {}  filtered:   {}",
		SYM_FILTER.style(DIM),
		stats.filtered.bold()
	);
	println!(
		"  {}  failed:     {}",
		SYM_FAIL.style(BRICK),
		stats.failed.bold()
	);
	println!(
		"  {}  duration:   {}",
		SYM_IDLE.style(DIM),
		format!("{:.1}s", stats.duration.as_secs_f64()).bold()
	);
}

// ── Status helpers ─────────────────────────────────────────────────────────

/// Print a one-line status message to stderr (the stream used for all
/// non-summary output per PLAN.md §10.4).
pub fn status(symbol: &str, style: Style, message: impl AsRef<str>) {
	eprintln!("{}  {}", symbol.style(style), message.as_ref());
}
