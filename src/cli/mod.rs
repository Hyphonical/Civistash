use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};

/// One-shot vs. daemon mode is the `--daemon` flag — no subcommands.
#[derive(Parser, Debug, Clone)]
#[command(
	name = "civistash",
	author,
	version,
	about = "Archiver for popular CivitAI images with full metadata sidecars",
	long_about = None
)]
pub struct Cli {
	#[arg(long, default_value_t = false)]
	pub daemon: bool,

	#[arg(long, env = "CIVITAI_TOKEN")]
	pub ca_token: Option<String>,

	/// Hugging Face token (also via `HUGGINGFACE_TOKEN` env). Required for
	/// `--upload-hf`.
	#[arg(long, env = "HUGGINGFACE_TOKEN")]
	pub hf_token: Option<String>,

	#[arg(long, default_value = "stash")]
	pub output_dir: PathBuf,

	/// Number of images to fetch per cycle. Values above 200 trigger
	/// cursor-based pagination.
	#[arg(long, default_value_t = 100)]
	pub limit: u32,

	#[arg(long, value_enum, default_value_t = Period::Day)]
	pub period: Period,

	#[arg(long, value_enum, default_value_t = SortOrder::MostReactions)]
	pub sort: SortOrder,

	/// NSFW level filter. Comma-separated values are OR'd into a single
	/// bitmask (e.g. `--nsfw-level mature,x` → `browsingLevel=12`).
	#[arg(long, value_enum, value_delimiter = ',')]
	pub nsfw_level: Vec<NsfwLevel>,

	/// Download all media types. By default only `type=image` is kept.
	#[arg(long, default_value_t = false)]
	pub all_types: bool,

	/// Bundle the date partition into a `.tar.gz` after the cycle.
	#[arg(long, default_value_t = false)]
	pub bundle: bool,

	/// Upload the bundle to a Hugging Face dataset repo (e.g.
	/// `my-org/my-dataset`). Requires `--bundle`. Credentials supplied
	/// via `--hf-token` or `HUGGINGFACE_TOKEN`.
	#[arg(long, value_name = "REPO")]
	pub upload_hf: Option<String>,

	/// Delete local files after a successful HF upload. No effect without
	/// `--bundle --upload-hf`. Preserves files on upload failure.
	#[arg(long, default_value_t = false)]
	pub delete_after: bool,

	/// Log verbosity for `civistash::*`. Third-party crates default to
	/// `warn`; override via `RUST_LOG`.
	#[arg(long, default_value = "info")]
	pub log_level: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Period {
	Day,
	Week,
	Month,
	AllTime,
}

impl Period {
	/// Cooldown between daemon cycles. `None` for `AllTime` (no sensible
	/// interval).
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
		f.write_str(match self {
			Period::Day => "Day",
			Period::Week => "Week",
			Period::Month => "Month",
			Period::AllTime => "AllTime",
		})
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SortOrder {
	MostReactions,
	MostComments,
	Newest,
	Oldest,
}

impl std::fmt::Display for SortOrder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			SortOrder::MostReactions => "Most Reactions",
			SortOrder::MostComments => "Most Comments",
			SortOrder::Newest => "Newest",
			SortOrder::Oldest => "Oldest",
		})
	}
}

/// NSFW browsing level bitmask values. Bit positions are suspected but
/// need confirmation against the live API before release.
///
/// TODO(verify): confirm the bit positions against the live API
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum NsfwLevel {
	None,
	Soft,
	Mature,
	X,
}

impl NsfwLevel {
	pub fn bit(&self) -> u32 {
		match self {
			NsfwLevel::None => 1,
			NsfwLevel::Soft => 2,
			NsfwLevel::Mature => 4,
			NsfwLevel::X => 8,
		}
	}
}

/// OR a slice of `NsfwLevel` values into a `browsingLevel` bitmask.
pub fn browsing_level_bitmask(levels: &[NsfwLevel]) -> u32 {
	levels.iter().fold(0u32, |acc, l| acc | l.bit())
}

impl std::fmt::Display for NsfwLevel {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			NsfwLevel::None => "None",
			NsfwLevel::Soft => "Soft",
			NsfwLevel::Mature => "Mature",
			NsfwLevel::X => "X",
		})
	}
}
