//! Daemon scheduler loop with signal-aware sleep.
//!
//! Per PLAN.md §5.6 / success criteria #5: a SIGTERM (or Ctrl+C) lets
//! the current image finish — the cycle is given the shutdown flag
//! and checks it between images — and then the daemon exits cleanly
//! with code 0.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Run the daemon loop until a shutdown signal arrives.
pub async fn run_daemon(client: &reqwest::Client, cli: &Cli) -> Result<(), DaemonError> {
	let cooldown = cli
		.period
		.cooldown()
		.ok_or(DaemonError::AllTimeNotAllowed)?;
	let cooldown_human = humanize_duration(cooldown);

	let shutdown = Arc::new(AtomicBool::new(false));
	spawn_signal_watcher(shutdown.clone());

	loop {
		// 0. Pre-cycle flag check. If a signal arrived during the
		//    previous sleep, skip starting a fresh cycle entirely
		//    and exit immediately. Without this, a Ctrl+C during
		//    sleep would start an empty cycle that the cycle's
		//    per-image check would then immediately abort.
		if shutdown.load(Ordering::SeqCst) {
			cleanup_after_shutdown(cli);
			ui::status(SYM_OK, FOREST, "shutdown signal received, exiting cleanly");
			return Ok(());
		}

		// 1. Run one cycle. The cycle checks `shutdown` between
		//    images and returns early if it's set.
		let stats = cycle::run_cycle(client, cli, Some(&shutdown)).await?;
		ui::print_cycle_summary(&stats);

		// 2. Post-cycle flag check (signal may have arrived during
		//    the cycle, between the top-of-loop check and here).
		if shutdown.load(Ordering::SeqCst) {
			cleanup_after_shutdown(cli);
			ui::status(SYM_OK, FOREST, "shutdown signal received, exiting cleanly");
			return Ok(());
		}

		// 3. Sleep, polling the shutdown flag once a second. The
		//    single signal handler (the spawned watcher) is the
		//    only thing that ever sets the flag — we don't race
		//    the sleep against a second `wait_for_signal`, which
		//    was the cause of the double Ctrl-C log + empty cycle
		//    bug. Worst-case shutdown latency: 1 s.
		ui::status(
			SYM_IDLE,
			DIM,
			format!("Sleeping {} until next cycle", cooldown_human),
		);
		wait_with_shutdown_poll(cooldown, &shutdown).await;
	}
}

/// Install the signal handlers and spawn a watcher task that flips
/// the shutdown flag when SIGINT or SIGTERM is received. This is
/// the **only** place in the daemon that waits for signals — the
/// sleep path polls the flag instead of racing a second signal
/// future, which previously caused duplicate "Ctrl-C received" log
/// lines and a wasted empty cycle on shutdown.
fn spawn_signal_watcher(shutdown: Arc<AtomicBool>) {
	tokio::spawn(async move {
		wait_for_signal().await;
		shutdown.store(true, Ordering::SeqCst);
	});
}

/// Sleep for `cooldown`, returning early as soon as `shutdown` is
/// set. Polls once per second; the granularity caps worst-case
/// shutdown latency at 1 s after the signal lands.
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

/// Best-effort cleanup of `.partial` files in the current date
/// partition (PLAN.md §8.4 option 2). Errors are swallowed because
/// leaving a stray partial on disk is harmless — the dedup check
/// ignores it and a future run can overwrite it.
fn cleanup_after_shutdown(cli: &Cli) {
	let date = chrono::Utc::now().date_naive();
	let partition = storage::partition_dir(&cli.output_dir, date);
	storage::cleanup_partials(&partition);
}

/// Format a `Duration` as a short human string (`"24h"`, `"7d"`,
/// `"30d"`, …). Used in the "Sleeping …" status line.
fn humanize_duration(d: Duration) -> String {
	let total_secs = d.as_secs();
	let days = total_secs / 86_400;
	let hours = (total_secs % 86_400) / 3600;
	let minutes = (total_secs % 3600) / 60;
	if days > 0 {
		format!("{}d {}h", days, hours)
	} else if hours > 0 {
		format!("{}h {}m", hours, minutes)
	} else {
		format!("{}m", minutes)
	}
}

// ── Signal platform shims ──────────────────────────────────────────────────

#[cfg(unix)]
async fn wait_for_signal() {
	use tokio::signal::unix::{SignalKind, signal};
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
	// Both branches log the same line — we only care that *some*
	// shutdown signal landed, not which one. The "Ctrl-C received"
	// wording is what the user typed; the OS might have delivered
	// SIGTERM (e.g. Docker stop) instead. Either way, the daemon
	// is shutting down.
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
