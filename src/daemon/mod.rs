use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::time::sleep;
use tracing::info;

use crate::cli::Cli;
use crate::cycle;
use crate::storage;
use crate::ui::{self, DIM, FOREST, SYM_IDLE, SYM_OK};

#[derive(Debug, Error)]
pub enum DaemonError {
	#[error("--daemon cannot be combined with --period AllTime (no sensible cooldown)")]
	AllTimeNotAllowed,

	#[error("cycle failed: {0}")]
	Cycle(#[from] anyhow::Error),

	#[error("failed to install signal handlers: {0}")]
	Signal(#[from] std::io::Error),
}

/// Run the daemon loop. A SIGTERM (or Ctrl+C) lets the current image finish
/// before the daemon exits cleanly.
pub async fn run_daemon(client: &reqwest::Client, cli: &Cli) -> Result<(), DaemonError> {
	let cooldown = cli
		.period
		.cooldown()
		.ok_or(DaemonError::AllTimeNotAllowed)?;
	let cooldown_human = humanize_duration(cooldown);

	let shutdown = Arc::new(AtomicBool::new(false));
	spawn_signal_watcher(shutdown.clone());

	loop {
		if shutdown.load(Ordering::SeqCst) {
			cleanup_after_shutdown(cli);
			ui::status(SYM_OK, FOREST, "shutdown signal received, exiting cleanly");
			return Ok(());
		}

		let stats = cycle::run_cycle(client, cli, Some(&shutdown)).await?;
		ui::print_cycle_summary(&stats);

		if shutdown.load(Ordering::SeqCst) {
			cleanup_after_shutdown(cli);
			ui::status(SYM_OK, FOREST, "shutdown signal received, exiting cleanly");
			return Ok(());
		}

		ui::status(
			SYM_IDLE,
			DIM,
			format!("Sleeping {} until next cycle", cooldown_human),
		);
		wait_with_shutdown_poll(cooldown, &shutdown).await;
	}
}

fn spawn_signal_watcher(shutdown: Arc<AtomicBool>) {
	tokio::spawn(async move {
		wait_for_signal().await;
		shutdown.store(true, Ordering::SeqCst);
	});
}

/// Sleep for `cooldown`, polling the shutdown flag once per second.
async fn wait_with_shutdown_poll(cooldown: Duration, shutdown: &Arc<AtomicBool>) {
	let poll_interval = Duration::from_secs(1);
	let mut remaining = cooldown;
	while !remaining.is_zero() {
		if shutdown.load(Ordering::Relaxed) {
			return;
		}
		let sleep_for = remaining.min(poll_interval);
		sleep(sleep_for).await;
		remaining = remaining.saturating_sub(sleep_for);
	}
}

fn cleanup_after_shutdown(cli: &Cli) {
	let date = chrono::Utc::now().date_naive();
	let partition = storage::partition_dir(&cli.output_dir, date);
	storage::cleanup_partials(&partition);
}

fn humanize_duration(d: Duration) -> String {
	let total_secs = d.as_secs();
	let days = total_secs / 86_400;
	let hours = (total_secs % 86_400) / 3600;
	let minutes = (total_secs % 3600) / 60;
	if days > 0 {
		format!("{days}d {hours}h")
	} else if hours > 0 {
		format!("{hours}h {minutes}m")
	} else {
		format!("{minutes}m")
	}
}

#[cfg(unix)]
async fn wait_for_signal() {
	use tokio::signal::unix::{signal, SignalKind};
	let mut sigterm = match signal(SignalKind::terminate()) {
		Ok(s) => s,
		Err(e) => {
			tracing::error!(error = %e, "failed to install SIGTERM handler");
			return;
		}
	};
	let mut sigint = match signal(SignalKind::interrupt()) {
		Ok(s) => s,
		Err(e) => {
			tracing::error!(error = %e, "failed to install SIGINT handler");
			return;
		}
	};
	tokio::select! {
		_ = sigterm.recv() => {}
		_ = sigint.recv()  => {}
	}
	info!("Ctrl-C received");
}

#[cfg(not(unix))]
async fn wait_for_signal() {
	if let Err(e) = tokio::signal::ctrl_c().await {
		tracing::error!(error = %e, "failed to install Ctrl-C handler");
	}
	info!("Ctrl-C received");
}
