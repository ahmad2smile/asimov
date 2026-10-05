//! Prints one snapshot of the simulated fleet as SpacetimeDB reducer calls,
//! for `scripts/seed-spacetime.sh`. Each AGV is simulated for a few minutes
//! first so the snapshot has a mix of driving, charging, and errors.
//!
//! Output: one line per call, `<reducer>\t<json arg>[\t<json arg>]`, in the
//! argument shapes of `backend/src/agv.ts`. Messages are stamped with the
//! current time (`sentAt`), so a rerun replaces older seeded rows.

use asimov_simulator::agv::{Agv, ConnectionState, Millis};
use asimov_simulator::fleet::{FleetConfig, Rng, build_fleet};
use serde_json::{Value, json};

const TICK_MS: Millis = 250; // same step as src/main.rs

fn main() -> Result<(), String> {
    let seed = std::env::var("SIM_SEED").unwrap_or_else(|_| "asimov".into());
    let config = FleetConfig { seed: seed.clone(), site_count: 5, min_robots: 2, max_robots: 5 };
    let start: Millis = 0;
    // VDA 5050 header timestamp as SpacetimeDB JSON.
    let sent_at = json!({ "__timestamp_micros_since_unix_epoch__": now_micros() });

    for site in build_fleet(&config)? {
        for robot in &site.robots {
            let id = format!("{}:{}:{}", seed, robot.model.manufacturer, robot.serial_number);
            let mut agv = Agv::new("uagv", &site, robot, Rng::new(&id), start);
            let run_ms = Rng::new(&format!("{id}:seed")).int(60, 20 * 60) * 1000;
            let mut now = start;
            while now < start + run_ms {
                now += TICK_MS;
                agv.tick(now);
            }
            let state = agv.take_state(now, 0, true).expect("forced state");
            let connection = agv.connection_message(ConnectionState::Online, now).connection_state;

            let row = json!({
                "order_id": state.order_id,
                "last_node_id": state.last_node_id,
                "driving": state.driving,
                "paused": state.paused,
                "charging": state.battery_state.charging,
                // The latest error: the last one reported.
                "error": match state.errors.last() {
                    Some(e) => json!({"some": {
                        "error_type": e.error_type,
                        "error_level": tag(e.error_level.to_lowercase()),
                    }}),
                    None => json!({"none": []}),
                },
            });
            let manufacturer = json!(agv.manufacturer);
            let serial_number = json!(agv.serial_number);
            let map_id = json!(state.agv_position.map_id);
            println!("upsert_map\t{map_id}\t{{\"none\": []}}");
            println!("upsert_agv\t{manufacturer}\t{serial_number}\t{map_id}");
            println!("upsert_agv_state\t{manufacturer}\t{serial_number}\t{row}\t{sent_at}");
            println!(
                "upsert_agv_connection\t{manufacturer}\t{serial_number}\t{}\t{sent_at}",
                tag(format!("{connection:?}"))
            );
        }
    }
    Ok(())
}

fn now_micros() -> i64 {
    let since_epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock after 1970");
    since_epoch.as_micros() as i64
}

/// SpacetimeDB JSON for a unit enum variant: `{"<variant>": []}`, with the
/// variant's first letter lowercased as the CLI expects.
fn tag(variant: String) -> Value {
    let mut chars = variant.chars();
    let name = match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    };
    json!({ name: [] })
}
