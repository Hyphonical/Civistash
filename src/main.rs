//! Entry point and command dispatch.
//!
//! Responsibilities (PLAN.md §5.1):
//! - Parse `Cli` via `clap::Parser`.
//! - Initialise the `tracing` subscriber with env-filter (`RUST_LOG`).
//! - Construct the shared `reqwest::Client`.
//! - Dispatch to one-shot or daemon path.
//! - Set process exit code.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use owo_colors::OwoColorize;
use tracing_subscriber::EnvFilter;

mod api;
mod cli;
mod cycle;
mod daemon;
mod download;
mod storage;
mod ui;

use cli::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let cli = Cli::parse();

	match run(cli).await {
		Ok(()) => ExitCode::SUCCESS,
		Err(err) => {
			// Use ui::BRICK for the error line. Status rules in
			// PLAN.md §10.4: errors are brick-coloured, one line, no
			// stack trace. The RUST_LOG=civistash=trace chain is
			// available separately via tracing.
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

/// Initialise `tracing` with an env filter. The CLI's `--log-level`
/// flag is the default filter; `RUST_LOG` overrides it.
fn init_tracing(default_directive: &str) {
	let filter =
		EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));

	tracing_subscriber::fmt()
		.with_env_filter(filter)
		.with_writer(std::io::stderr)
		.without_time()
		.init();
}

/// Build the shared `reqwest::Client`. Reused across every cycle in
/// daemon mode — connection pooling is one of the wins of a long-
/// lived process.
///
/// Timeouts:
/// - `connect_timeout` = 15s. A failed TCP/TLS handshake fails
///   fast instead of hanging the cycle.
/// - `timeout` = 90s total. Comfortable for a 200-image response
///   with `withMeta=true` over a slow network; the old 30s limit
///   was too aggressive in practice.
fn build_client() -> Result<reqwest::Client> {
	let user_agent = format!("civistash/{}", env!("CARGO_PKG_VERSION"));
	let client = reqwest::Client::builder()
		.user_agent(user_agent)
		.connect_timeout(Duration::from_secs(15))
		.timeout(Duration::from_secs(90))
		.build()?;
	Ok(client)
}
