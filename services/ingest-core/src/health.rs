//! `/healthz`: the process is up and its runtime answers.
//! `/readyz`: every component (MQTT, sink) is connected; 503 lists the ones
//! that are not.
//!
//! `healthcheck` is for the container healthcheck, so images need no curl.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use tokio_util::sync::CancellationToken;

/// Readiness of named components. Cheap to clone.
#[derive(Clone, Default)]
pub struct Health(Arc<Mutex<BTreeMap<&'static str, bool>>>);

impl Health {
    /// Registers a component as not ready.
    pub fn component(&self, name: &'static str) -> Component {
        self.0.lock().unwrap().insert(name, false);
        Component { health: self.clone(), name }
    }

    /// Names of components that are not ready.
    pub fn not_ready(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().iter().filter(|(_, ready)| !**ready).map(|(name, _)| *name).collect()
    }
}

#[derive(Clone)]
pub struct Component {
    health: Health,
    name: &'static str,
}

impl Component {
    pub fn set(&self, ready: bool) {
        self.health.0.lock().unwrap().insert(self.name, ready);
    }
}

pub async fn serve(port: u16, health: Health, shutdown: CancellationToken) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready))
        .with_state(health);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
        .await
        .with_context(|| format!("binding health port {port}"))?;
    axum::serve(listener, app).with_graceful_shutdown(shutdown.cancelled_owned()).await?;
    Ok(())
}

async fn ready(State(health): State<Health>) -> (StatusCode, String) {
    let not_ready = health.not_ready();
    if not_ready.is_empty() {
        (StatusCode::OK, "ready".into())
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, format!("not ready: {}", not_ready.join(", ")))
    }
}

/// `GET http://127.0.0.1:<port><path>`; Ok when it answers 200.
pub fn healthcheck(port: u16, path: &str) -> Result<()> {
    let timeout = Duration::from_secs(3);
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("200") {
        bail!("{path}: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ready_only_when_every_component_is() {
        let health = Health::default();
        let mqtt = health.component("mqtt");
        let sink = health.component("sink");
        let shutdown = CancellationToken::new();
        // Port 0 is not usable with `healthcheck`, so pick a free one first.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let server = tokio::spawn(serve(port, health.clone(), shutdown.clone()));

        let check = move |path: &'static str| tokio::task::spawn_blocking(move || healthcheck(port, path));
        for _ in 0..50 {
            if check("/healthz").await.unwrap().is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(check("/healthz").await.unwrap().is_ok());
        let error = check("/readyz").await.unwrap().unwrap_err().to_string();
        assert!(error.contains("503"), "{error}");

        mqtt.set(true);
        assert!(check("/readyz").await.unwrap().is_err());
        sink.set(true);
        assert!(check("/readyz").await.unwrap().is_ok());
        mqtt.set(false);
        assert_eq!(health.not_ready(), ["mqtt"]);

        shutdown.cancel();
        server.await.unwrap().unwrap();
    }
}
