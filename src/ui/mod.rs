use std::time::Duration;

use owo_colors::{OwoColorize, Style};

pub const EMBER: Style = Style::new().fg_rgb::<217, 119, 87>();
pub const FOREST: Style = Style::new().fg_rgb::<90, 138, 90>();
pub const BRICK: Style = Style::new().fg_rgb::<193, 69, 69>();
pub const DIM: Style = Style::new().fg_rgb::<122, 122, 140>();

pub const SYM_OK: &str = "✓";
pub const SYM_FAIL: &str = "✗";
pub const SYM_SKIP: &str = "⦿";
pub const SYM_FILTER: &str = "⊘";
pub const SYM_PROGRESS: &str = "⤓";
pub const SYM_IDLE: &str = "…";
#[allow(dead_code)]
pub const SYM_RETRY: &str = "↻";
pub const SYM_DIVIDER: &str = "■";

#[derive(Debug, Default, Clone, Copy)]
pub struct CycleStats {
	pub downloaded: u32,
	pub skipped: u32,
	pub filtered: u32,
	pub failed: u32,
	pub duration: Duration,
}

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

pub fn status(symbol: &str, style: Style, message: impl AsRef<str>) {
	eprintln!("{}  {}", symbol.style(style), message.as_ref());
}
