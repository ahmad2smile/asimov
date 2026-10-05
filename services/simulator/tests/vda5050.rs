//! Fast-forwards the whole simulated fleet and checks that what it would
//! publish is valid VDA 5050 2.1.0: every message against the official JSON
//! schemas (vendored in ./vda5050-2.1.0, MIT), plus the protocol rules that a
//! schema cannot express (order/node/edge/action life cycle, headerIds, ...).

use std::collections::{BTreeMap, HashSet};

use asimov_simulator::agv::{Agv, ConnectionState, Millis};
use asimov_simulator::fleet::{FleetConfig, MODELS, Rng, Site, build_fleet};
use chrono::DateTime;
use serde_json::Value;

const SIMULATED_HOURS: i64 = 4;
const TICK_MS: i64 = 250; // same step as src/main.rs
const STATE_MAX_INTERVAL_MS: i64 = 30000;

fn config() -> FleetConfig {
    FleetConfig { seed: "asimov".into(), site_count: 5, min_robots: 2, max_robots: 5 }
}

/// Spec 6.3: allowed characters in topic levels.
fn is_topic_level(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
}

fn validator(name: &str) -> jsonschema::Validator {
    let path = format!("{}/tests/vda5050-2.1.0/{name}.schema", env!("CARGO_MANIFEST_DIR"));
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    jsonschema::options().should_validate_formats(true).build(&schema).unwrap()
}

#[test]
fn fleet_has_the_configured_sites_and_real_manufacturers() {
    let sites = build_fleet(&config()).unwrap();
    assert_eq!(sites.len(), 5);
    assert_eq!(sites.iter().map(|s| &s.map.map_id).collect::<HashSet<_>>().len(), 5);

    let mut ids = HashSet::new();
    for site in &sites {
        assert!((2..=5).contains(&site.robots.len()), "{} has {} robots", site.site_id, site.robots.len());
        for robot in &site.robots {
            assert!(MODELS.contains(&robot.model));
            assert!(is_topic_level(robot.model.manufacturer));
            assert!(is_topic_level(&robot.serial_number));
            ids.insert(format!("{}/{}", robot.model.manufacturer, robot.serial_number));
        }
    }
    assert_eq!(ids.len(), sites.iter().map(|s| s.robots.len()).sum::<usize>(), "serial numbers are unique");
    assert_eq!(build_fleet(&config()).unwrap(), sites, "same seed, same fleet");
}

#[test]
fn topics_start_with_the_site() {
    let sites = build_fleet(&config()).unwrap();
    for site in &sites {
        assert!(is_topic_level(site.site_id));
        for robot in &site.robots {
            let agv = Agv::new("uagv", site, robot, Rng::new("topic"), 0);
            assert_eq!(
                agv.topic("state"),
                format!("{}/uagv/v2/{}/{}/state", site.site_id, robot.model.manufacturer, robot.serial_number)
            );
        }
    }
}

#[test]
fn fleet_matches_the_javascript_simulator() {
    // Same seed as before the Rust port, so MQTT topics and stored rows stay valid.
    let sites = build_fleet(&config()).unwrap();
    let robots_per_site: Vec<_> = sites.iter().map(|s| s.robots.len()).collect();
    assert_eq!(robots_per_site, [2, 4, 3, 2, 4]);
    let mut ids: Vec<_> = sites
        .iter()
        .flat_map(|s| &s.robots)
        .map(|r| format!("{}/{}", r.model.manufacturer, r.serial_number))
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "AGILOX/ONE-15194",
            "Jungheinrich/ERC215a-49925",
            "Jungheinrich/ERC215a-75297",
            "KUKA/KMP1500-30567",
            "KUKA/KMP1500-33720",
            "KUKA/KMP1500-68604",
            "Linde/L-MATIC-19050",
            "MiR/MiR1350-43617",
            "STILL/FM-X-84786",
            "STILL/FM-X-88713",
            "ek-robotics/VarioMove-21650",
            "ek-robotics/VarioMove-26481",
            "ek-robotics/VarioMove-52519",
            "ek-robotics/VarioMove-84420",
            "ek-robotics/VarioMove-87537",
        ]
    );
}

#[test]
fn simulated_hours_of_every_agv_follow_vda5050() {
    let sites = build_fleet(&config()).unwrap();
    let validators = ["state", "visualization", "connection"].map(|name| (name, validator(name)));
    let mut seen: BTreeMap<&str, u32> =
        ["orders", "pick", "drop", "startCharging", "obstacle"].into_iter().map(|k| (k, 0)).collect();

    for site in &sites {
        for robot in &site.robots {
            let start: Millis = 1_735_689_600_000; // 2025-01-01T00:00:00Z
            let end = start + SIMULATED_HOURS * 3_600_000;
            let rng = Rng::new(&format!("test:{}", robot.serial_number));
            let mut agv = Agv::new("uagv", site, robot, rng, start);

            let mut state = vec![to_value(&agv.take_state(start, STATE_MAX_INTERVAL_MS, true).unwrap())];
            let mut visualization = Vec::new();
            let mut connection = vec![to_value(&agv.connection_message(ConnectionState::Online, start))];
            let mut now = start + TICK_MS;
            while now <= end {
                agv.tick(now);
                if let Some(message) = agv.take_state(now, STATE_MAX_INTERVAL_MS, false) {
                    state.push(to_value(&message));
                }
                if (now - start) % 1000 == 0 {
                    visualization.push(to_value(&agv.visualization_message(now)));
                }
                now += TICK_MS;
            }
            connection.push(to_value(&agv.connection_message(ConnectionState::Offline, end)));

            let label = format!("{}/{}", robot.model.manufacturer, robot.serial_number);
            for ((topic, validator), list) in validators.iter().zip([&state, &visualization, &connection]) {
                for (i, message) in list.iter().enumerate() {
                    let errors: Vec<_> = validator.iter_errors(message).map(|e| e.to_string()).collect();
                    assert!(errors.is_empty(), "{label} {topic} #{i}: {errors:?}");
                    assert_eq!(message["headerId"], i as u64, "{label} {topic}: headerIds count up by 1");
                    assert_eq!(message["version"], "2.1.0");
                    assert_eq!(message["manufacturer"], robot.model.manufacturer);
                    assert_eq!(message["serialNumber"], robot.serial_number.as_str());
                }
            }
            check_state_sequence(&label, &state, site, &mut seen);
        }
    }

    // The run must have exercised every behaviour, or the checks prove little.
    for (what, count) in seen {
        assert!(count > 0, "no {what} observed");
    }
}

fn to_value(message: &impl serde::Serialize) -> Value {
    serde_json::to_value(message).unwrap()
}

fn next_status(action_type: &str, status: &str) -> Option<&'static str> {
    match (action_type, status) {
        ("pick" | "drop", "WAITING") => Some("INITIALIZING"),
        ("pick" | "drop", "INITIALIZING") => Some("RUNNING"),
        // startCharging has no INITIALIZING stage (spec 6.8.2).
        ("startCharging", "WAITING") => Some("RUNNING"),
        (_, "RUNNING") => Some("FINISHED"),
        _ => None,
    }
}

fn is_done(action: &Value) -> bool {
    matches!(action["actionStatus"].as_str(), Some("FINISHED" | "FAILED"))
}

fn list(value: &Value) -> &Vec<Value> {
    value.as_array().expect("array")
}

fn millis(timestamp: &Value) -> i64 {
    DateTime::parse_from_rfc3339(timestamp.as_str().unwrap()).unwrap().timestamp_millis()
}

fn check_state_sequence(label: &str, states: &[Value], site: &Site, seen: &mut BTreeMap<&str, u32>) {
    let first = &states[0];
    assert_eq!(first["orderId"], "", "{label}: no orderId before the first order");
    assert_eq!(first["lastNodeId"], "", "{label}: no lastNodeId before the first order");
    assert_eq!(first["lastNodeSequenceId"], 0);

    for (i, state) in states.iter().enumerate() {
        let at = format!("{label} state #{i}");
        let previous = i.checked_sub(1).map(|p| &states[p]);

        if let Some(previous) = previous {
            let gap = millis(&state["timestamp"]) - millis(&previous["timestamp"]);
            assert!(gap <= STATE_MAX_INTERVAL_MS, "{at}: {gap} ms since the previous state (max 30 s)");
        }

        // Only the current map is used and every position is on it.
        let maps: Vec<_> = list(&state["maps"]).iter().map(|m| m["mapId"].clone()).collect();
        assert_eq!(maps, [Value::from(site.map.map_id.as_str())]);
        assert_eq!(state["agvPosition"]["mapId"], site.map.map_id.as_str());

        // Remaining nodes continue from the last traversed node in steps of 2;
        // each node comes with the edge leading to it (sequenceId - 1).
        let nodes = list(&state["nodeStates"]);
        let edges = list(&state["edgeStates"]);
        let last_sequence = state["lastNodeSequenceId"].as_u64().unwrap();
        assert_eq!(edges.len(), nodes.len(), "{at}: one edge per remaining node");
        for (n, node) in nodes.iter().enumerate() {
            let sequence = node["sequenceId"].as_u64().unwrap();
            assert_eq!(sequence, last_sequence + 2 * (n as u64 + 1), "{at}: node sequence");
            assert_eq!(edges[n]["sequenceId"], sequence - 1, "{at}: edge sequence");
        }

        // Pick/drop/charging are HARD blocking: no driving while they run.
        let actions = list(&state["actionStates"]);
        let driving = state["driving"] == true;
        if actions.iter().any(|a| !is_done(a) && a["actionStatus"] != "WAITING") {
            assert!(!driving, "{at}: drives during a blocking action");
        }
        let charging = state["batteryState"]["charging"] == true;
        if charging {
            assert!(!driving, "{at}: drives while charging");
        }
        if state["safetyState"]["fieldViolation"] == true {
            *seen.get_mut("obstacle").unwrap() += 1;
            assert!(!driving, "{at}: drives with a protective field violation");
        }

        let Some(previous) = previous else { continue };
        let previous_actions = list(&previous["actionStates"]);
        let previous_charging = previous["batteryState"]["charging"] == true;
        let loads = list(&state["loads"]).len();
        let previous_loads = list(&previous["loads"]).len();

        if state["orderId"] != previous["orderId"] {
            *seen.get_mut("orders").unwrap() += 1;
            // Rule 3 of order acceptance: the previous order was visibly complete.
            assert!(list(&previous["nodeStates"]).is_empty(), "{at}: new order while nodes remain");
            assert!(previous_actions.iter().all(is_done), "{at}: new order before actions finished");
            assert!(!previous_charging, "{at}: new order while charging");
            // The new order starts on the node where the AGV stands.
            if previous["orderId"] != "" {
                assert_eq!(state["lastNodeId"], previous["lastNodeId"], "{at}: order starts elsewhere");
            }
            assert_eq!(state["lastNodeSequenceId"], 0);
            // Previous action states are replaced; new ones start WAITING.
            let order_id = state["orderId"].as_str().unwrap();
            assert!(actions.iter().all(|a| a["actionId"].as_str().unwrap().starts_with(order_id)));
            continue;
        }

        // Within one order: every action stage is published, in order.
        let mut finished = false;
        for (action, before) in actions.iter().zip(previous_actions) {
            let (status, before_status) = (action["actionStatus"].as_str().unwrap(), before["actionStatus"].as_str().unwrap());
            if status == before_status {
                continue;
            }
            let action_type = action["actionType"].as_str().unwrap();
            assert_eq!(Some(status), next_status(action_type, before_status), "{at}: {action_type} {before_status} -> {status}");
            if status != "FINISHED" {
                continue;
            }

            finished = true;
            *seen.get_mut(action_type).unwrap() += 1;
            assert!(action["resultDescription"].is_string(), "{at}: FINISHED without resultDescription");
            // Linked states change together with FINISHED (spec 6.8.2).
            match action_type {
                "pick" => assert_eq!((previous_loads, loads), (0, 1), "{at}: pick finished without a load"),
                "drop" => assert_eq!((previous_loads, loads), (1, 0), "{at}: drop finished with a load"),
                _ => assert!(charging, "{at}: charging not reported"),
            }
        }

        // Loads and charging only change through a finishing action.
        if !finished {
            assert_eq!(loads, previous_loads, "{at}: load changed without pick/drop");
            if !previous_charging {
                assert!(!charging, "{at}: charging without startCharging");
            }
        }

        // Nodes are only ever removed from the front when traversed.
        assert!(nodes.len() <= list(&previous["nodeStates"]).len(), "{at}: nodes added without a new order");
    }
}
