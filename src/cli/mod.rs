//! Command-line argument definitions.
//!
//! See PLAN.md §5.2 for the canonical surface area. A single `Cli`
//! struct with no subcommands — the only operational difference between
//! one-shot and daemon mode is the `--daemon` flag.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};

/// Top-level CLI struct, populated by `clap::Parser::parse()`.
#[derive(Parser, Debug, Clone)]
#[command(
	name = "civistash",
	author,
	version,
	about = "Archiver for popular CivitAI images with full metadata sidecars",
	long_about = None
)]
pub struct Cli {
	/// Run continuously, sleeping between cycles (cooldown = --period).
	#[arg(long, default_value_t = false)]
	pub daemon: bool,

	/// CivitAI API token (also via `CIVITAI_TOKEN` env).
	#[arg(long, env = "CIVITAI_TOKEN")]
	pub ca_token: Option<String>,

	/// Hugging Face token (also via `HUGGINGFACE_TOKEN` env).
	/// Required for `--upload-hf` to work; the same flag/env is
	/// the only way to authenticate against the Hub.
	#[arg(long, env = "HUGGINGFACE_TOKEN")]
	pub hf_token: Option<String>,

	/// Output directory. Date-partitioned subdirectories are created
	/// inside.
	#[arg(long, default_value = "stash")]
	pub output_dir: PathBuf,

	/// Number of images to fetch per cycle. CivitAI's API caps each
	/// request at 200, so values above 200 trigger automatic
	/// pagination (multiple requests chained by `cursor`).
	#[arg(long, default_value_t = 100)]
	pub limit: u32,

	/// Time window for the popularity ranking.
	#[arg(long, value_enum, default_value_t = Period::Day)]
	pub period: Period,

	/// Sort order.
	#[arg(long, value_enum, default_value_t = SortOrder::MostReactions)]
	pub sort: SortOrder,

	/// NSFW level filter. Pass one or more comma-separated values to
	/// OR them into a single bitmask (`--nsfw-level mature,x` →
	/// `browsingLevel=12` = Mature + X). Omit to use the account
	/// default.
	#[arg(long, value_enum, value_delimiter = ',')]
	pub nsfw_level: Vec<NsfwLevel>,

	/// Download all media types (images, videos, audio, anything
	/// the API returns). By default only `type=image` is kept.
	#[arg(long, default_value_t = false)]
	pub all_types: bool,

	/// Bundle the date partition into a `.tar.gz` after the cycle
	/// finishes. Output: `<output-dir>/YYYY-MM-DD.tar.gz`. Pairs
	/// naturally with `--upload-hf`.
	#[arg(long, default_value_t = false)]
	pub bundle: bool,

	/// Upload the per-cycle bundle to a Hugging Face dataset repo
	/// (e.g. `my-org/my-dataset`). Requires `--bundle` (or a
	/// pre-existing bundle file). The `HUGGINGFACE_TOKEN` env var
	/// or `--hf-token` flag supplies credentials.
	#[arg(long, value_name = "REPO")]
	pub upload_hf: Option<String>,

	/// Log verbosity for `civistash::*` (third-party crates
	/// always default to `warn`; override via `RUST_LOG`).
	#[arg(long, default_value = "info")]
	pub log_level: String,
}

// ── Period ─────────────────────────────────────────────────────────────────

/// CivitAI `period` query parameter. The string form is sent verbatim
/// to the API.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Period {
	Day,
	Week,
	Month,
	AllTime,
}

impl Period {
	/// Cooldown between cycles in daemon mode. `None` means daemon
	/// mode is nonsensical for this period (AllTime).
	pub fn cooldown(&self) -> Option<Duration> {
		match self {
			Period::Day => Some(Duration::from_secs(86_400)),
			Period::Week => Some(Duration::from_secs(604_800)),
			Period::Month => Some(Duration::from_secs(2_592_000)),
			Period::AllTime => None,
		}
	}
}

impl std::fmt::Display for Period {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let s = match self {
			Period::Day => "Day",
			Period::Week => "Week",
			Period::Month => "Month",
			Period::AllTime => "AllTime",
		};
		f.write_str(s)
	}
}

// ── SortOrder ──────────────────────────────────────────────────────────────

/// CivitAI `sort` query parameter. The string form is sent verbatim
/// to the API.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SortOrder {
	MostReactions,
	MostComments,
	Newest,
	Oldest,
}

impl std::fmt::Display for SortOrder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let s = match self {
			SortOrder::MostReactions => "Most Reactions",
			SortOrder::MostComments => "Most Comments",
			SortOrder::Newest => "Newest",
			SortOrder::Oldest => "Oldest",
		};
		f.write_str(s)
	}
}

// ── NsfwLevel ──────────────────────────────────────────────────────────────

/// NSFW browsing level. The numeric bitmask is what the CivitAI API
/// actually consumes; the mapping is documented in PLAN.md §8.1.
///
/// TODO(verify): confirm the bit positions against the live API
/// before shipping a release.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum NsfwLevel {
	None,
	Soft,
	Mature,
	X,
}

impl NsfwLevel {
	/// Single-bit position per the suspected bitmask layout.
	pub fn bit(&self) -> u32 {
		match self {
			NsfwLevel::None => 1,
			NsfwLevel::Soft => 2,
			NsfwLevel::Mature => 4,
			NsfwLevel::X => 8,
		}
	}
}

/// OR a slice of `NsfwLevel` values into a single `browsingLevel`
/// bitmask. An empty slice is "no filter" (caller should omit the
/// `browsingLevel` parameter from the request).
pub fn browsing_level_bitmask(levels: &[NsfwLevel]) -> u32 {
	levels.iter().fold(0u32, |acc, l| acc | l.bit())
}

impl std::fmt::Display for NsfwLevel {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let s = match self {
			NsfwLevel::None => "None",
			NsfwLevel::Soft => "Soft",
			NsfwLevel::Mature => "Mature",
			NsfwLevel::X => "X",
		};
		f.write_str(s)
	}
}
