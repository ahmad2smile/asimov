//! End-to-end tests: the ingest binaries against the compose stack and the
//! running simulator. Ignored by default; run them with
//! `npm run services:e2e` (one at a time; they restart shared containers).

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ingest_e2e::{Kind, Service, Spacetime, Tdengine, compose, config, eventually, fleet, publish, stack_up};

const STARTUP: Duration = Duration::from_secs(60);
const DATA: Duration = Duration::from_secs(60);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn sleep(seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds));
}

/// Sorted `<manufacturer>/<serialNumber>` of the simulated AGVs at `sites`.
fn agvs(sites: &[&str]) -> BTreeSet<String> {
    fleet(sites).into_iter().flat_map(|s| s.agvs).collect()
}

/// Each site name once per row group, e.g. `SELECT site, COUNT(*) ... GROUP BY site`.
fn sites_in(td: &Tdengine, stable: &str) -> anyhow::Result<BTreeSet<String>> {
    Ok(td
        .query(&format!("SELECT site, COUNT(*) FROM `{}`.`{stable}` GROUP BY site", td.database))?
        .into_iter()
        .filter_map(|row| row[0].as_str().map(str::to_owned))
        .collect())
}

#[test]
#[ignore = "needs the compose stack"]
fn stores_only_the_configured_sites() {
    stack_up();
    let sites = ["hamburg", "lyon"];
    let st = Spacetime::fresh("sites");
    let td = Tdengine::fresh("sites");
    let spacetime = Service::start(Kind::Spacetime, "e2e-sites-st", &config(Kind::Spacetime, "sites", &sites, &st.database, 18081));
    let tdengine = Service::start(Kind::Tdengine, "e2e-sites-td", &config(Kind::Tdengine, "sites", &sites, &td.database, 18082));
    spacetime.wait_ready(STARTUP);
    tdengine.wait_ready(STARTUP);

    let expected = agvs(&sites);
    let maps: BTreeSet<String> = fleet(&sites).into_iter().map(|s| s.map_id).collect();
    eventually("every AGV of the sites registered and online", DATA, || {
        let registered: BTreeSet<String> = st.sql("SELECT agv_id FROM agv")?.into_iter().map(|r| r[0].clone()).collect();
        let online = st
            .sql("SELECT agv_id, connection_state FROM agv_state")?
            .into_iter()
            .filter(|r| r.get(1).is_some_and(|c| c.contains("online")))
            .count();
        Ok((registered == expected && online == expected.len()).then_some(()))
    });
    let stored_maps: BTreeSet<String> = st.sql("SELECT map_id FROM map").unwrap().into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(stored_maps, maps, "only the configured sites' maps");

    let want: BTreeSet<String> = sites.iter().map(|s| s.to_string()).collect();
    for stable in ["agv_visualization", "agv_state", "agv_connection"] {
        eventually(&format!("{stable} rows of both sites only"), DATA, || {
            let found = sites_in(&td, stable)?;
            anyhow::ensure!(found.is_subset(&want), "unexpected sites {found:?}");
            Ok((found == want).then_some(()))
        });
    }
    let subtables = td.count(&format!("SELECT COUNT(*) FROM (SELECT DISTINCT tbname FROM `{}`.agv_visualization)", td.database)).unwrap();
    assert_eq!(subtables as usize, expected.len(), "one visualization subtable per AGV");
}

#[test]
#[ignore = "needs the compose stack"]
fn replicas_share_the_work_without_duplicates() {
    stack_up();
    let sites = ["hamburg", "rotterdam", "lyon"];
    let st = Spacetime::fresh("replicas");
    let td = Tdengine::fresh("replicas");
    let mut tdengine = [("e2e-replica-td-a", 18083), ("e2e-replica-td-b", 18084)]
        .map(|(name, port)| Service::start(Kind::Tdengine, name, &config(Kind::Tdengine, "replicas", &sites, &td.database, port)));
    let spacetime = [("e2e-replica-st-a", 18085), ("e2e-replica-st-b", 18086)]
        .map(|(name, port)| Service::start(Kind::Spacetime, name, &config(Kind::Spacetime, "replicas", &sites, &st.database, port)));
    for service in tdengine.iter().chain(&spacetime) {
        service.wait_ready(STARTUP);
    }
    sleep(30);

    // SpacetimeDB: every AGV current, and both instances did part of it.
    let expected = agvs(&sites);
    eventually("every AGV online with a state", DATA, || {
        let rows = st.sql("SELECT agv_id, connection_state, state_sent_at FROM agv_state")?;
        let complete = rows.iter().filter(|r| r[1].contains("online") && r[2].contains("some")).count();
        Ok((complete == expected.len()).then_some(()))
    });
    for service in &spacetime {
        assert!(service.log_sum("batch written", "written", |_| true) > 0, "{} wrote nothing", service.name);
    }

    // TDengine: both wrote visualizations, and each message only once.
    for service in &mut tdengine {
        assert!(service.stop(Duration::from_secs(15)), "{} did not stop cleanly", service.name);
    }
    let vis = |l: &serde_json::Value| l["fields"]["stable"] == "agv_visualization";
    let written: Vec<u64> = tdengine.iter().map(|s| s.log_sum("batch written", "written", vis)).collect();
    assert!(written.iter().all(|&n| n > 0), "both instances wrote: {written:?}");
    let stored = td.count(&format!("SELECT COUNT(*) FROM `{}`.agv_visualization", td.database)).unwrap();
    assert_eq!(written.iter().sum::<u64>() as i64, stored, "every visualization was written by exactly one instance");
}

#[test]
#[ignore = "needs the compose stack"]
fn keeps_data_through_a_tdengine_outage() {
    stack_up();
    let sites = ["milan"];
    let td = Tdengine::fresh("outage");
    let mut service = Service::start(Kind::Tdengine, "e2e-outage-td", &config(Kind::Tdengine, "outage", &sites, &td.database, 18087));
    service.wait_ready(STARTUP);
    let count = |from: i64, to: i64| {
        td.count(&format!("SELECT COUNT(*) FROM `{}`.agv_visualization WHERE ts >= {from} AND ts < {to}", td.database))
    };
    eventually("visualizations stored", DATA, || Ok((count(0, i64::MAX)? > 0).then_some(())));

    let down = now_ms();
    compose(&["stop", "tdengine"]).unwrap();
    sleep(20);
    assert!(service.running(), "the service survives the outage");
    assert!(!service.is_ready(), "not ready while TDengine is down");
    compose(&["up", "-d", "--wait", "tdengine"]).unwrap();
    let up = now_ms();
    service.wait_ready(Duration::from_secs(120));

    // The simulator sends one visualization per AGV per second.
    let agv_count = agvs(&sites).len() as i64;
    let (from, to) = (down + 2_000, up - 2_000);
    let expected = agv_count * (to - from) / 1000;
    eventually("buffered rows of the outage written", DATA, || {
        let stored = count(from, to)?;
        Ok((stored * 10 >= expected * 9).then_some(stored))
    });
}

#[test]
#[ignore = "needs the compose stack"]
fn recovers_after_spacetimedb_and_mosquitto_restarts() {
    stack_up();
    let sites = ["poznan"];
    let st = Spacetime::fresh("restarts");
    let td = Tdengine::fresh("restarts");
    let spacetime = Service::start(Kind::Spacetime, "e2e-restarts-st", &config(Kind::Spacetime, "restarts", &sites, &st.database, 18088));
    let tdengine = Service::start(Kind::Tdengine, "e2e-restarts-td", &config(Kind::Tdengine, "restarts", &sites, &td.database, 18089));
    spacetime.wait_ready(STARTUP);
    tdengine.wait_ready(STARTUP);

    let latest_state = || -> anyhow::Result<BTreeSet<String>> {
        Ok(st.sql("SELECT state_sent_at FROM agv_state")?.into_iter().map(|r| r[0].clone()).collect())
    };
    let latest_vis = || td.count(&format!("SELECT COUNT(*) FROM `{}`.agv_visualization", td.database));
    let expected = agvs(&sites).len();
    eventually("states stored", DATA, || Ok((st.sql("SELECT agv_id FROM agv_state")?.len() == expected).then_some(())));

    for container in ["spacetimedb", "mosquitto"] {
        let before_state = latest_state().unwrap();
        let before_vis = latest_vis().unwrap();
        compose(&["restart", container]).unwrap();
        compose(&["up", "-d", "--wait", container]).unwrap();
        spacetime.wait_ready(Duration::from_secs(120));
        tdengine.wait_ready(Duration::from_secs(120));
        eventually(&format!("new states after restarting {container}"), DATA, || {
            Ok((latest_state()? != before_state).then_some(()))
        });
        eventually(&format!("new visualizations after restarting {container}"), DATA, || {
            Ok((latest_vis()? > before_vis).then_some(()))
        });
    }
}

#[test]
#[ignore = "needs the compose stack"]
fn resumes_after_a_crash_and_stops_cleanly() {
    stack_up();
    let sites = ["hamburg"];
    let td = Tdengine::fresh("crash");
    let config = config(Kind::Tdengine, "crash", &sites, &td.database, 18090);
    let mut service = Service::start(Kind::Tdengine, "e2e-crash-td", &config);
    service.wait_ready(STARTUP);
    let count = || td.count(&format!("SELECT COUNT(*) FROM `{}`.agv_visualization", td.database));
    eventually("rows stored", DATA, || Ok((count()? > 0).then_some(())));

    service.kill();
    let after_crash = count().unwrap();
    let mut service = Service::start(Kind::Tdengine, "e2e-crash-td", &config);
    service.wait_ready(STARTUP);
    eventually("rows stored after the restart", DATA, || Ok((count()? > after_crash).then_some(())));
    assert!(service.stop(Duration::from_secs(10)), "SIGTERM: clean exit within the flush time");
}

/// A test-only site, so the dev services in compose never see these AGVs.
const TEST_SITE: &str = "e2e-site";

fn state_json(serial: &str, timestamp: Option<&str>) -> Vec<u8> {
    let mut state = serde_json::json!({
        "headerId": 1, "version": "2.1.0", "manufacturer": "E2E", "serialNumber": serial,
        "orderId": "order-1", "orderUpdateId": 0, "lastNodeId": "", "lastNodeSequenceId": 0,
        "nodeStates": [], "edgeStates": [], "actionStates": [], "driving": false, "operatingMode": "AUTOMATIC",
        "agvPosition": {"x": 1.0, "y": 2.0, "theta": 0.0, "mapId": "e2e-warehouse", "positionInitialized": true},
        "batteryState": {"batteryCharge": 50.0, "charging": false},
        "errors": [], "safetyState": {"eStop": "NONE", "fieldViolation": false}
    });
    if let Some(timestamp) = timestamp {
        state["timestamp"] = timestamp.into();
    }
    serde_json::to_vec(&state).unwrap()
}

#[test]
#[ignore = "needs the compose stack"]
fn rejects_bad_messages_and_keeps_running() {
    stack_up();
    let sites = [TEST_SITE];
    let st = Spacetime::fresh("bad");
    let td = Tdengine::fresh("bad");
    let mut spacetime = Service::start(Kind::Spacetime, "e2e-bad-st", &config(Kind::Spacetime, "bad", &sites, &st.database, 18091));
    let mut tdengine = Service::start(Kind::Tdengine, "e2e-bad-td", &config(Kind::Tdengine, "bad", &sites, &td.database, 18092));
    spacetime.wait_ready(STARTUP);
    tdengine.wait_ready(STARTUP);

    let topic = |serial: &str| format!("{TEST_SITE}/uagv/v2/E2E/{serial}/state");
    publish(&topic("bad-json"), b"{not json");
    publish(&topic("no-timestamp"), &state_json("no-timestamp", None));
    publish(&topic("bad-timestamp"), &state_json("bad-timestamp", Some("yesterday")));
    publish(&topic("other-serial"), &state_json("someone-else", Some("2026-01-01T00:00:00Z")));
    let now = chrono::Utc::now().to_rfc3339();
    publish(&topic("good"), &state_json("good", Some(&now)));

    eventually("the good message in SpacetimeDB", DATA, || {
        Ok((st.sql("SELECT agv_id FROM agv")?.iter().any(|r| r[0] == "E2E/good")).then_some(()))
    });
    eventually("the good message in TDengine", DATA, || {
        Ok((td.count(&format!("SELECT COUNT(*) FROM `{}`.agv_state", td.database))? == 1).then_some(()))
    });
    let agvs = st.sql("SELECT agv_id FROM agv").unwrap();
    assert_eq!(agvs.len(), 1, "only the good AGV: {agvs:?}");

    for service in [&mut spacetime, &mut tdengine] {
        let reasons: BTreeSet<String> = service
            .logs()
            .iter()
            .filter(|l| l["fields"]["message"] == "rejected MQTT message")
            .filter_map(|l| l["fields"]["reason"].as_str().map(str::to_owned))
            .collect();
        for reason in ["bad_json", "missing_timestamp", "bad_timestamp", "header_mismatch"] {
            assert!(reasons.contains(reason), "{}: no {reason} rejection in {reasons:?}", service.name);
        }
        assert!(service.running() && service.is_ready(), "{} keeps running", service.name);
    }
}
