//! Keeps long-running tasks alive: a task that fails, panics, or returns is
//! started again after an exponential backoff with jitter. Only shutdown
//! stops it.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::backoff::Backoff;
use crate::telemetry::Metrics;

/// A run at least this long counts as healthy, so the next failure starts the
/// backoff from the beginning.
const HEALTHY_RUN: Duration = Duration::from_secs(60);

/// Runs `task` until `shutdown` is cancelled, restarting it whenever it ends.
/// The task gets the shutdown token and should return soon after it fires.
pub async fn supervise<F, Fut>(
    name: &'static str,
    shutdown: CancellationToken,
    metrics: Metrics,
    task: F,
) where
    F: FnMut(CancellationToken) -> Fut,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    supervise_with(
        name,
        shutdown,
        metrics,
        Backoff::default(),
        HEALTHY_RUN,
        task,
    )
    .await
}

pub async fn supervise_with<F, Fut>(
    name: &'static str,
    shutdown: CancellationToken,
    metrics: Metrics,
    mut backoff: Backoff,
    healthy_run: Duration,
    mut task: F,
) where
    F: FnMut(CancellationToken) -> Fut,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        let started = Instant::now();
        // A separate tokio task, so a panic is caught as a JoinError.
        let outcome = tokio::spawn(task(shutdown.clone())).await;

        if shutdown.is_cancelled() {
            if let Ok(Err(error)) | Err(error) = outcome.map_err(anyhow::Error::from) {
                tracing::warn!(task = name, error = %format!("{error:#}"), "task ended with an error during shutdown");
            }
            return;
        }

        match outcome {
            Ok(Ok(())) => tracing::warn!(task = name, "task returned unexpectedly; restarting"),
            Ok(Err(error)) => {
                tracing::error!(task = name, error = %format!("{error:#}"), "task failed; restarting")
            }
            Err(error) => tracing::error!(task = name, error = %error, "task panicked; restarting"),
        }

        metrics.task_restarted(name);

        if started.elapsed() >= healthy_run {
            backoff.reset();
        }

        let delay = backoff.next_delay();

        tracing::info!(
            task = name,
            delay_ms = delay.as_millis() as u64,
            "waiting before restart"
        );

        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn restarts_failing_and_panicking_tasks_until_shutdown() {
        let runs = Arc::new(AtomicU32::new(0));
        let shutdown = CancellationToken::new();
        let counter = runs.clone();
        let stop = shutdown.clone();
        let supervisor = tokio::spawn(supervise_with(
            "test",
            shutdown.clone(),
            Metrics::disabled(),
            Backoff::new(Duration::from_millis(10), Duration::from_millis(100)),
            Duration::from_secs(60),
            move |_token| {
                let run = counter.fetch_add(1, Ordering::SeqCst) + 1;
                let stop = stop.clone();
                async move {
                    match run {
                        1 => anyhow::bail!("first run fails"),
                        2 => panic!("second run panics"),
                        3 => Ok(()),
                        _ => {
                            stop.cancel();
                            Ok(())
                        }
                    }
                }
            },
        ));
        tokio::time::timeout(Duration::from_secs(5), supervisor)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            4,
            "restarted after error, panic and return"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_interrupts_the_backoff_wait() {
        let shutdown = CancellationToken::new();
        let supervisor = tokio::spawn(supervise_with(
            "test",
            shutdown.clone(),
            Metrics::disabled(),
            Backoff::new(Duration::from_secs(3600), Duration::from_secs(3600)),
            Duration::from_secs(60),
            |_token| async { anyhow::bail!("always fails") },
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(1), supervisor)
            .await
            .unwrap()
            .unwrap();
    }
}
