//! Calls the module's `ingest` reducer over one SpacetimeDB connection.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use spacetimedb_sdk::DbContext;
use tokio::sync::{oneshot, watch};

use ingest_core::vda::Message;

use crate::convert;
use crate::module_bindings::{DbConnection, IngestMessage, ingest};

/// No answer for this long: the connection counts as dead, even if no
/// disconnect was reported (network gone, server frozen).
pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq)]
enum State {
    Connecting,
    Up,
    /// Why the connection is gone.
    Closed(String),
}

pub struct Sink {
    conn: Arc<DbConnection>,
    state: watch::Receiver<State>,
}

impl Sink {
    /// Connects to SpacetimeDB, e.g. `ws://spacetimedb:3000`, and waits until
    /// the connection is accepted.
    pub async fn connect(uri: &str, database: &str) -> Result<Self> {
        let (state_tx, state) = watch::channel(State::Connecting);
        let state_tx = Arc::new(state_tx);
        let conn = DbConnection::builder()
            .with_uri(uri)
            .with_database_name(database)
            .on_connect({
                let state = state_tx.clone();
                move |_, _, _| {
                    state.send_if_modified(|s| {
                        let connecting = *s == State::Connecting;
                        if connecting {
                            *s = State::Up;
                        }
                        connecting
                    });
                }
            })
            .on_connect_error({
                let state = state_tx.clone();
                move |_, error| close(&state, format!("connect failed: {error}"))
            })
            .on_disconnect({
                let state = state_tx.clone();
                move |_, error| {
                    close(
                        &state,
                        error.map_or("disconnected".into(), |e| format!("disconnected: {e}")),
                    )
                }
            })
            .build()
            .context("connecting to SpacetimeDB")?;
        let conn = Arc::new(conn);
        // Processes incoming messages and runs callbacks until the connection ends.
        tokio::spawn({
            let conn = conn.clone();
            async move {
                let ended = conn.run_async().await;
                close(&state_tx, format!("connection ended: {ended:?}"));
            }
        });

        let sink = Self { conn, state };
        let mut state = sink.state.clone();
        let reached = state
            .wait_for(|s| *s != State::Connecting)
            .await
            .map(|s| s.clone());
        match reached {
            Ok(State::Closed(reason)) => bail!("SpacetimeDB {reason}"),
            Ok(_) => Ok(sink),
            Err(_) => bail!("SpacetimeDB connection dropped"),
        }
    }

    /// Calls `ingest` within `limit`. The outer error: the connection failed,
    /// so the call may not have run. The inner error: the module rejected the batch.
    pub async fn write(&self, batch: &[Message], limit: Duration) -> Result<Result<(), String>> {
        let messages: Vec<IngestMessage> = batch.iter().filter_map(convert::to_ingest).collect();
        let skipped = batch.len() - messages.len();
        if skipped > 0 {
            tracing::debug!(skipped, "messages SpacetimeDB does not store, not sent");
        }

        let (tx, rx) = oneshot::channel();
        self.conn
            .reducers
            .ingest_then(messages, move |_, result| {
                let _ = tx.send(result);
            })
            .map_err(|error| anyhow!("sending the ingest call: {error}"))?;

        let answer = async {
            tokio::select! {
                result = rx => match result {
                    Ok(Ok(answer)) => Ok(answer),
                    Ok(Err(error)) => Err(anyhow!("ingest call failed: {error}")),
                    Err(_) => Err(anyhow!("ingest call dropped without an answer")),
                },
                reason = self.closed() => Err(anyhow!("SpacetimeDB {reason}")),
            }
        };
        tokio::time::timeout(limit, answer)
            .await
            .unwrap_or_else(|_| Err(anyhow!("no answer from SpacetimeDB within {limit:?}")))
    }

    /// Waits until the connection is gone, and says why.
    pub async fn closed(&self) -> String {
        let mut state = self.state.clone();
        let closed = state
            .wait_for(|s| matches!(s, State::Closed(_)))
            .await
            .map(|s| s.clone());
        match closed {
            Ok(State::Closed(reason)) => reason,
            _ => "connection dropped".into(),
        }
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.conn.disconnect();
    }
}

/// Keeps the first reason.
fn close(state: &watch::Sender<State>, reason: String) {
    state.send_if_modified(|s| {
        let open = !matches!(s, State::Closed(_));
        if open {
            *s = State::Closed(reason);
        }
        open
    });
}
