use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use owo_colors::OwoColorize;
use tracing_subscriber::EnvFilter;

mod api;
mod bundle;
mod cli;
mod cycle;
mod daemon;
mod download;
mod storage;
mod ui;
mod upload;

use cli::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let cli = Cli::parse();

	match run(cli).await {
		Ok(()) => ExitCode::SUCCESS,
		Err(err) => {
			eprintln!(
				"{}  {}",
				ui::SYM_FAIL.style(ui::BRICK),
				format!("{err:#}").style(ui::BRICK)
			);
			ExitCode::FAILURE
		}
	}
}

async fn run(cli: Cli) -> Result<()> {
	init_tracing(&cli.log_level);

	if cli.daemon && cli.period.cooldown().is_none() {
		anyhow::bail!("--daemon cannot be combined with --period AllTime (no sensible cooldown)");
	}

	let client = build_client()?;

	if cli.daemon {
		daemon::run_daemon(&client, &cli).await?;
	} else {
		let stats = cycle::run_cycle(&client, &cli, None).await?;
		ui::print_cycle_summary(&stats);
	}
	Ok(())
}

/// `RUST_LOG` can override the default; third-party crates default to `warn`
/// to keep output manageable. Set e.g. `RUST_LOG=civistash=debug,hf_hub=info`.
fn init_tracing(default_directive: &str) {
	let filter = EnvFilter::try_from_default_env()
		.unwrap_or_else(|_| EnvFilter::new(format!("civistash={default_directive},warn")));

	tracing_subscriber::fmt()
		.with_env_filter(filter)
		.with_writer(std::io::stderr)
		.without_time()
		.init();
}

/// Shared client reused across daemon cycles (connection-pooling is the win).
fn build_client() -> Result<reqwest::Client> {
	let user_agent = format!("civistash/{}", env!("CARGO_PKG_VERSION"));
	Ok(reqwest::Client::builder()
		.user_agent(user_agent)
		.connect_timeout(Duration::from_secs(15))
		.timeout(Duration::from_secs(90))
		.build()?)
}
