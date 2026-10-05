//! Helpers for the end-to-end tests: the compose stack, the ingest binaries as
//! local processes, and queries against SpacetimeDB and TDengine.
//!
//! Every test uses its own databases and MQTT group, so it never touches the
//! data of the dev services running in compose.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use asimov_simulator::fleet::{FleetConfig, build_fleet};
use serde_json::{Value, json};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn work_dir() -> PathBuf {
    let dir = repo_root().join("services/target/e2e");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(command: &mut Command) -> Result<String> {
    let output = command.output().with_context(|| format!("running {command:?}"))?;
    if !output.status.success() {
        bail!("{command:?} failed: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn compose(args: &[&str]) -> Result<String> {
    run(Command::new("docker").current_dir(repo_root()).arg("compose").args(args))
}

/// Starts what the tests need (idempotent).
pub fn stack_up() {
    compose(&["up", "-d", "--wait", "mosquitto", "spacetimedb", "tdengine", "agv-simulator"]).expect("starting the compose stack");
}

/// Polls `check` until it returns Some, or panics with the last error.
pub fn eventually<T>(what: &str, timeout: Duration, mut check: impl FnMut() -> Result<Option<T>>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        let last = match check() {
            Ok(Some(value)) => return value,
            Ok(None) => "not yet".to_owned(),
            Err(error) => format!("{error:#}"),
        };
        if Instant::now() > deadline {
            panic!("timed out after {timeout:?} waiting for {what}: {last}");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// --- Simulated fleet ---------------------------------------------------------

pub struct SiteFleet {
    pub site: String,
    pub map_id: String,
    /// `<manufacturer>/<serialNumber>`
    pub agvs: Vec<String>,
}

/// The fleet the compose simulator runs with its default settings.
pub fn fleet(sites: &[&str]) -> Vec<SiteFleet> {
    let config = FleetConfig { seed: "asimov".into(), site_count: 5, min_robots: 2, max_robots: 5 };
    build_fleet(&config)
        .unwrap()
        .into_iter()
        .filter(|site| sites.contains(&site.site_id))
        .map(|site| SiteFleet {
            site: site.site_id.into(),
            map_id: site.map.map_id.clone(),
            agvs: site.robots.iter().map(|r| format!("{}/{}", r.model.manufacturer, r.serial_number)).collect(),
        })
        .collect()
}

// --- SpacetimeDB ---------------------------------------------------------------

pub struct Spacetime {
    pub database: String,
}

impl Spacetime {
    /// Publishes the module to a new, empty database.
    pub fn fresh(name: &str) -> Self {
        let database = format!("asimov-e2e-{name}");
        run(Command::new("spacetime").current_dir(repo_root()).args([
            "publish",
            &database,
            "--server",
            "local",
            "--no-config",
            "--module-path",
            "backend",
            "--delete-data=always",
            "--yes",
        ]))
        .expect("publishing the module");
        Self { database }
    }

    /// Rows of a query as cell strings, as the CLI prints them.
    pub fn sql(&self, query: &str) -> Result<Vec<Vec<String>>> {
        let out = run(Command::new("spacetime").args(["sql", &self.database, "--server", "local", "--no-config", query]))?;
        Ok(out
            .lines()
            .skip(2)
            .filter(|line| line.contains('|') || !line.trim().is_empty())
            .map(|line| line.split('|').map(|cell| cell.trim().trim_matches('"').to_owned()).collect())
            .collect())
    }
}

impl Drop for Spacetime {
    fn drop(&mut self) {
        let _ = run(Command::new("spacetime").args(["delete", &self.database, "--server", "local", "--no-config", "--yes"]));
    }
}

// --- TDengine --------------------------------------------------------------------

pub struct Tdengine {
    pub database: String,
}

impl Tdengine {
    /// A database name that does not exist yet; the service creates it.
    pub fn fresh(name: &str) -> Self {
        let database = format!("asimov_e2e_{}", name.replace('-', "_"));
        let td = Self { database };
        td.query(&format!("DROP DATABASE IF EXISTS `{}`", td.database)).expect("dropping the test database");
        td
    }

    pub fn query(&self, sql: &str) -> Result<Vec<Vec<Value>>> {
        let response: Value = ureq::post("http://127.0.0.1:6041/rest/sql")
            .header("Authorization", "Basic cm9vdDp0YW9zZGF0YQ==") // root:taosdata
            .send(sql)?
            .body_mut()
            .read_json()?;
        if response["code"] != 0 {
            bail!("{sql}: {response}");
        }
        Ok(response["data"].as_array().cloned().unwrap_or_default().into_iter().map(|row| row.as_array().cloned().unwrap_or_default()).collect())
    }

    /// First cell of the first row as a number (0 when there is no row).
    pub fn count(&self, sql: &str) -> Result<i64> {
        Ok(self.query(sql)?.first().and_then(|row| row.first()).and_then(Value::as_i64).unwrap_or(0))
    }
}

impl Drop for Tdengine {
    fn drop(&mut self) {
        let _ = self.query(&format!("DROP DATABASE IF EXISTS `{}`", self.database));
    }
}

// --- Ingest services ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Spacetime,
    Tdengine,
}

impl Kind {
    fn binary(self) -> &'static str {
        match self {
            Kind::Spacetime => "spacetime-ingest",
            Kind::Tdengine => "tdengine-ingest",
        }
    }
}

/// Config for a test: its own MQTT group and client ids, local addresses,
/// no metrics export.
pub fn config(kind: Kind, test: &str, sites: &[&str], database: &str, health_port: u16) -> Value {
    let mut config = json!({
        "mqtt": {
            "url": "mqtt://127.0.0.1:1883",
            "clientIdPrefix": format!("e2e-{test}-{}", kind.binary()),
            "group": format!("e2e-{test}-{}", kind.binary()),
            "sessionExpirySecs": 60,
        },
        "sites": sites.iter().map(|s| json!({"id": s, "topicPrefix": format!("{s}/uagv/v2")})).collect::<Vec<_>>(),
        "telemetry": {"healthPort": health_port},
        "shutdownFlushSecs": 5,
    });
    match kind {
        Kind::Spacetime => config["spacetime"] = json!({"uri": "ws://127.0.0.1:3000", "database": database}),
        Kind::Tdengine => {
            config["tdengine"] = json!({"dsn": "ws://127.0.0.1:6041", "database": database, "batchMaxWaitMs": 200});
            config["buffer"] = json!({"maxMessages": 100000});
        }
    }
    config
}

pub struct Service {
    child: Option<Child>,
    pub name: String,
    pub port: u16,
    log: PathBuf,
}

impl Service {
    /// Starts the binary from `services/target/debug` (built by
    /// `npm run services:e2e`). `name` is also the instance name.
    pub fn start(kind: Kind, name: &str, config: &Value) -> Self {
        let dir = work_dir();
        let config_path = dir.join(format!("{name}.json"));
        std::fs::write(&config_path, serde_json::to_vec_pretty(config).unwrap()).unwrap();
        let log = dir.join(format!("{name}.log"));
        let binary = repo_root().join("services/target/debug").join(kind.binary());
        assert!(binary.exists(), "{} is missing; run `npm run services:e2e`", binary.display());
        let child = Command::new(binary)
            .env("CONFIG_PATH", &config_path)
            .env("HOSTNAME", name)
            .env("RUST_LOG", "info,spacetime_ingest=debug,tdengine_ingest=debug")
            .stdout(File::create(&log).unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let port = config["telemetry"]["healthPort"].as_u64().unwrap() as u16;
        Self { child: Some(child), name: name.into(), port, log }
    }

    pub fn is_ready(&self) -> bool {
        ureq::get(format!("http://127.0.0.1:{}/readyz", self.port)).call().is_ok()
    }

    pub fn wait_ready(&self, timeout: Duration) {
        eventually(&format!("{} ready", self.name), timeout, || Ok(self.is_ready().then_some(())));
    }

    /// Log lines (JSON) of this process.
    pub fn logs(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Sum of a numeric log field over lines with this message.
    pub fn log_sum(&self, message: &str, field: &str, filter: impl Fn(&Value) -> bool) -> u64 {
        self.logs()
            .iter()
            .filter(|l| l["fields"]["message"] == message && filter(l))
            .filter_map(|l| l["fields"][field].as_u64())
            .sum()
    }

    pub fn running(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| c.try_wait().unwrap().is_none())
    }

    /// SIGTERM, then waits; returns whether it exited cleanly.
    pub fn stop(&mut self, timeout: Duration) -> bool {
        let Some(mut child) = self.child.take() else { return true };
        let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                return status.success();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        false
    }

    /// SIGKILL: no shutdown work at all.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.kill();
    }
}

// --- MQTT --------------------------------------------------------------------

/// Publishes one message with QoS 1 and waits until the broker has it.
pub fn publish(topic: &str, payload: &[u8]) {
    use rumqttc::{Client, Event, MqttOptions, Outgoing, Packet, QoS};
    let mut options = MqttOptions::new(format!("e2e-publisher-{}", std::process::id()), "127.0.0.1", 1883);
    options.set_keep_alive(Duration::from_secs(5));
    let (client, mut connection) = Client::new(options, 10);
    client.publish(topic, QoS::AtLeastOnce, false, payload.to_vec()).unwrap();
    for event in connection.iter() {
        match event {
            Ok(Event::Incoming(Packet::PubAck(_))) => break,
            Ok(Event::Outgoing(Outgoing::Disconnect)) => break,
            Ok(_) => {}
            Err(error) => panic!("publishing {topic}: {error}"),
        }
    }
    let _ = client.disconnect();
}
