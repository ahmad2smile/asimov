//! Writes SQL statements to TDengine over WebSocket (taosAdapter).

use std::time::Duration;

use anyhow::{Context, Result};
use taos::{AsyncQueryable, AsyncTBuilder, Dsn, Taos, TaosBuilder};

/// A stopped server can leave a call waiting forever; after this the
/// connection counts as lost.
pub const TIMEOUT: Duration = Duration::from_secs(10);

pub struct Sink {
    taos: Taos,
}

impl Sink {
    /// Connects to TDengine, e.g. `ws://root:taosdata@tdengine:6041`.
    pub async fn connect(dsn: Dsn) -> Result<Self> {
        let taos = TaosBuilder::from_dsn(dsn)?
            .build()
            .await
            .context("connecting to TDengine")?;
        Ok(Self { taos })
    }

    /// Runs one SQL statement on this connection.
    pub async fn write(&self, sql: &str) -> Result<()> {
        self.taos.exec(sql).await?;
        Ok(())
    }

    /// Runs an `INSERT` statement within `TIMEOUT`.
    pub async fn insert(&self, sql: &str) -> Result<()> {
        tokio::time::timeout(TIMEOUT, self.write(sql))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("write timed out after {TIMEOUT:?}")))
    }

    /// Whether the server still answers: tells a bad message from a lost connection.
    pub async fn alive(&self) -> bool {
        matches!(
            tokio::time::timeout(TIMEOUT, self.write("SELECT SERVER_STATUS()")).await,
            Ok(Ok(()))
        )
    }
}
