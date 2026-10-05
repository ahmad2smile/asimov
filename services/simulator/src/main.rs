//! Runs the simulated fleet: one MQTT client per AGV publishing VDA 5050 2.1.0
//! `connection`, `state`, and `visualization` messages. Configured through
//! environment variables documented in the README.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use asimov_simulator::agv::{Agv, ConnectionState, Millis};
use asimov_simulator::fleet::{FleetConfig, Robot, Rng, build_fleet};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Outgoing, Packet, QoS};
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};

/// Simulation step; short enough to catch every action stage.
const TICK: Duration = Duration::from_millis(250);

struct Config {
    mqtt_url: String,
    interface_name: String,
    fleet: FleetConfig,
    limits: Limits,
}

#[derive(Clone, Copy)]
struct Limits {
    state_max_interval_ms: i64,
    visualization_interval_ms: i64,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let config = Config {
        mqtt_url: env_or("MQTT_URL", "mqtt://localhost:1883"),
        interface_name: env_or("VDA_INTERFACE_NAME", "uagv"),
        fleet: FleetConfig {
            seed: env_or("SIM_SEED", "asimov"),
            site_count: int_env("SITE_COUNT", 5)? as usize,
            min_robots: int_env("ROBOTS_PER_SITE_MIN", 2)? as usize,
            max_robots: int_env("ROBOTS_PER_SITE_MAX", 5)? as usize,
        },
        limits: Limits {
            state_max_interval_ms: int_env("STATE_MAX_INTERVAL_MS", 30000)?,
            visualization_interval_ms: int_env("VISUALIZATION_INTERVAL_MS", 1000)?,
        },
    };
    let sites = build_fleet(&config.fleet)?;

    let robot_count: usize = sites.iter().map(|site| site.robots.len()).sum();
    println!("Simulating {robot_count} AGVs at {} sites on {}", sites.len(), config.mqtt_url);
    for site in &sites {
        println!("  {} (map {}):", site.name, site.map.map_id);
        for robot in &site.robots {
            println!("    {}/{}", robot.model.manufacturer, robot.serial_number);
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let mut vehicles = Vec::new();
    for site in &sites {
        for robot in &site.robots {
            let options = mqtt_options(&config, robot)?;
            let agv = Agv::new(
                &config.interface_name,
                site,
                robot,
                Rng::new(&format!("{}:{}:{}", config.fleet.seed, robot.model.manufacturer, robot.serial_number)),
                now_ms(),
            );
            vehicles.push(tokio::spawn(run_vehicle(agv, options, config.limits, stop_rx.clone())));
        }
    }

    wait_for_shutdown_signal().await;
    println!("Publishing OFFLINE and disconnecting...");
    let _ = stop_tx.send(true);
    for vehicle in vehicles {
        let _ = vehicle.await;
    }
    Ok(())
}

/// Spec 6.14: the broker announces CONNECTIONBROKEN (QoS 1, retained) if the
/// AGV drops off unexpectedly; keepalive ~15 s detects that promptly.
fn mqtt_options(config: &Config, robot: &Robot) -> Result<MqttOptions, String> {
    let client_id = format!("{}-{}", robot.model.manufacturer, robot.serial_number);
    let mut options = MqttOptions::parse_url(format!("{}?client_id={client_id}", config.mqtt_url))
        .map_err(|error| format!("MQTT_URL {}: {error}", config.mqtt_url))?;
    options.set_keep_alive(Duration::from_secs(15));
    Ok(options)
}

/// One AGV: simulates every tick and, while connected, publishes its messages.
/// A single task owns the AGV and drives its MQTT event loop, so no locking.
async fn run_vehicle(mut agv: Agv, mut options: MqttOptions, limits: Limits, mut stop: watch::Receiver<bool>) {
    let label = format!("{}/{}", agv.manufacturer, agv.serial_number);
    let will = agv.connection_message(ConnectionState::Connectionbroken, now_ms());
    options.set_last_will(LastWill::new(agv.topic("connection"), json(&will), QoS::AtLeastOnce, true));
    let (client, mut events) = AsyncClient::new(options, 64);

    let publish = |agv: &Agv, topic: &str, payload: String, qos: QoS, retain: bool| {
        // try_publish never blocks this task, which must keep polling `events`.
        if let Err(error) = client.try_publish(agv.topic(topic), qos, retain, payload) {
            eprintln!("{label}: dropped {topic}: {error}");
        }
    };

    let mut ticker = interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut connected = false;
    let mut last_visualization_at: Millis = 0;

    loop {
        tokio::select! {
            _ = stop.changed() => break,
            event = events.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    println!("{label} connected");
                    connected = true;
                    let online = agv.connection_message(ConnectionState::Online, now_ms());
                    publish(&agv, "connection", json(&online), QoS::AtLeastOnce, true);
                    if let Some(state) = agv.take_state(now_ms(), limits.state_max_interval_ms, true) {
                        publish(&agv, "state", json(&state), QoS::AtMostOnce, false);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    if connected {
                        println!("{label} offline, reconnecting");
                    }
                    eprintln!("{label}: {error}");
                    connected = false;
                    sleep(Duration::from_secs(1)).await; // next poll() reconnects
                }
            },
            _ = ticker.tick() => {
                // The AGV keeps working while disconnected (spec 6.2); it only
                // publishes while connected, so headerIds count messages sent.
                let now = now_ms();
                agv.tick(now);
                if !connected {
                    continue;
                }
                if let Some(state) = agv.take_state(now, limits.state_max_interval_ms, false) {
                    publish(&agv, "state", json(&state), QoS::AtMostOnce, false);
                }
                if limits.visualization_interval_ms > 0 && now - last_visualization_at >= limits.visualization_interval_ms {
                    last_visualization_at = now;
                    let visualization = agv.visualization_message(now);
                    publish(&agv, "visualization", json(&visualization), QoS::AtMostOnce, false);
                }
            }
        }
    }

    // Graceful shutdown (spec 6.14): publish OFFLINE, then disconnect so the
    // broker discards the last will.
    if !connected {
        return;
    }
    let offline = agv.connection_message(ConnectionState::Offline, now_ms());
    publish(&agv, "connection", json(&offline), QoS::AtLeastOnce, true);
    let _ = client.try_disconnect();
    let flushed = timeout(Duration::from_secs(3), async {
        loop {
            match events.poll().await {
                Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await;
    if flushed.is_err() {
        eprintln!("{label}: timed out sending OFFLINE");
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

fn json(message: &impl Serialize) -> String {
    serde_json::to_string(message).expect("VDA 5050 messages serialize")
}

fn now_ms() -> Millis {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after 1970").as_millis() as Millis
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).ok().filter(|value| !value.is_empty()).unwrap_or_else(|| fallback.to_owned())
}

fn int_env(name: &str, fallback: i64) -> Result<i64, String> {
    match std::env::var(name) {
        Ok(raw) if !raw.is_empty() => raw
            .parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| format!("{name} must be a non-negative integer")),
        _ => Ok(fallback),
    }
}
