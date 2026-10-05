//! Shared parts of the MQTT ingest services.

pub mod app;
pub mod backoff;
pub mod config;
pub mod health;
pub mod mqtt;
mod reject_log;
pub mod supervisor;
pub mod telemetry;
pub mod topic;
pub mod vda;

/// Cancels the token on SIGTERM or Ctrl-C.
pub async fn wait_for_shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing the SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
}
