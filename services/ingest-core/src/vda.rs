//! VDA 5050 2.1.0 messages, trimmed to the fields the ingest services store.
//! Unknown fields are ignored, so newer minor versions still decode.
//!
//! Rules on top of the JSON schema:
//! - `timestamp` is required on every message (also `visualization`, where the
//!   schema leaves it optional) and must be RFC 3339; it orders messages.
//! - `manufacturer` and `serialNumber`, when present, must match the topic.

use chrono::DateTime;
use serde::Deserialize;

use crate::topic::{AgvKey, Kind, Route};

/// Microseconds since the Unix epoch.
pub type Micros = i64;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub agv: AgvKey,
    /// VDA 5050 header `timestamp`.
    pub sent_at: Micros,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    State(State),
    Visualization(Visualization),
    Connection(Connection),
}

impl Message {
    pub fn kind(&self) -> Kind {
        match self.body {
            Body::State(_) => Kind::State,
            Body::Visualization(_) => Kind::Visualization,
            Body::Connection(_) => Kind::Connection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub header_id: u32,
    pub order_id: String,
    pub order_update_id: u32,
    pub last_node_id: String,
    pub driving: bool,
    #[serde(default)]
    pub paused: Option<bool>,
    pub operating_mode: String,
    #[serde(default)]
    pub agv_position: Option<Position>,
    pub battery_state: BatteryState,
    #[serde(default)]
    pub loads: Option<Vec<serde_json::Value>>,
    pub errors: Vec<Error>,
    pub safety_state: SafetyState,
}

impl State {
    /// The map the AGV is on, if it reports a position.
    pub fn map_id(&self) -> Option<&str> {
        self.agv_position.as_ref().map(|p| p.map_id.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Visualization {
    /// Optional in the schema; a visualization without it carries nothing to
    /// store, so decoding rejects it.
    pub agv_position: Position,
    #[serde(default)]
    pub velocity: Option<Velocity>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub header_id: u32,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionState {
    Online,
    Offline,
    Connectionbroken,
}

impl ConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectionState::Online => "ONLINE",
            ConnectionState::Offline => "OFFLINE",
            ConnectionState::Connectionbroken => "CONNECTIONBROKEN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub theta: f64,
    pub map_id: String,
    #[serde(default)]
    pub localization_score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Velocity {
    #[serde(default)]
    pub vx: Option<f64>,
    #[serde(default)]
    pub vy: Option<f64>,
    #[serde(default)]
    pub omega: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryState {
    pub battery_charge: f64,
    pub charging: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub error_type: String,
    pub error_level: ErrorLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorLevel {
    Warning,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyState {
    pub e_stop: String,
    pub field_violation: bool,
}

/// Header fields checked against the topic.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    serial_number: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    BadJson,
    MissingTimestamp,
    BadTimestamp,
    HeaderMismatch,
    /// Valid JSON, but not the message shape of its topic.
    Schema,
}

impl Reject {
    /// Metric label.
    pub fn reason(self) -> &'static str {
        match self {
            Reject::BadJson => "bad_json",
            Reject::MissingTimestamp => "missing_timestamp",
            Reject::BadTimestamp => "bad_timestamp",
            Reject::HeaderMismatch => "header_mismatch",
            Reject::Schema => "schema",
        }
    }
}

pub fn decode(route: &Route, payload: &[u8]) -> Result<Message, Reject> {
    let value: serde_json::Value = serde_json::from_slice(payload).map_err(|_| Reject::BadJson)?;
    let header = Header::deserialize(&value).map_err(|_| Reject::Schema)?;
    let timestamp = header.timestamp.ok_or(Reject::MissingTimestamp)?;
    let sent_at = DateTime::parse_from_rfc3339(&timestamp).map_err(|_| Reject::BadTimestamp)?.timestamp_micros();
    if header.manufacturer.is_some_and(|m| m != route.agv.manufacturer)
        || header.serial_number.is_some_and(|s| s != route.agv.serial_number)
    {
        return Err(Reject::HeaderMismatch);
    }
    let body = match route.kind {
        Kind::State => Body::State(State::deserialize(&value).map_err(|_| Reject::Schema)?),
        Kind::Visualization => Body::Visualization(Visualization::deserialize(&value).map_err(|_| Reject::Schema)?),
        Kind::Connection => Body::Connection(Connection::deserialize(&value).map_err(|_| Reject::Schema)?),
    };
    Ok(Message { agv: route.agv.clone(), sent_at, body })
}

#[cfg(test)]
mod tests {
    use asimov_simulator::agv::{Agv, ConnectionState as SimConnection};
    use asimov_simulator::fleet::{FleetConfig, Rng, build_fleet};

    use super::*;
    use crate::config::Site;
    use crate::topic::Router;

    /// Messages of the real simulator, routed by their real topics.
    fn simulated(minutes: i64) -> Vec<(String, Vec<u8>)> {
        let config = FleetConfig { seed: "asimov".into(), site_count: 2, min_robots: 2, max_robots: 2 };
        let mut out = Vec::new();
        for site in build_fleet(&config).unwrap() {
            for robot in &site.robots {
                let start = 1_735_689_600_000;
                let mut agv = Agv::new("uagv", &site, robot, Rng::new(&robot.serial_number), start);
                out.push((agv.topic("connection"), json(&agv.connection_message(SimConnection::Online, start))));
                out.push((agv.topic("state"), json(&agv.take_state(start, 30_000, true).unwrap())));
                let mut now = start;
                while now < start + minutes * 60_000 {
                    now += 250;
                    agv.tick(now);
                    if let Some(state) = agv.take_state(now, 30_000, false) {
                        out.push((agv.topic("state"), json(&state)));
                    }
                    if now % 1000 == 0 {
                        out.push((agv.topic("visualization"), json(&agv.visualization_message(now))));
                    }
                }
            }
        }
        out
    }

    fn json(message: &impl serde::Serialize) -> Vec<u8> {
        serde_json::to_vec(message).unwrap()
    }

    fn router() -> Router {
        Router::new(vec![
            Site { id: "hamburg".into(), topic_prefix: "hamburg/uagv/v2".into() },
            Site { id: "rotterdam".into(), topic_prefix: "rotterdam/uagv/v2".into() },
        ])
    }

    #[test]
    fn decodes_every_simulator_message() {
        let router = router();
        let mut kinds = std::collections::HashMap::new();
        for (topic, payload) in simulated(10) {
            let route = router.route(&topic).unwrap();
            let message = decode(&route, &payload).unwrap_or_else(|e| panic!("{topic}: {e:?}"));
            assert_eq!(message.kind(), route.kind);
            assert!(message.sent_at >= 1_735_689_600_000_000);
            if let Body::State(state) = &message.body {
                assert!(state.map_id().unwrap().ends_with("-warehouse"));
            }
            *kinds.entry(route.kind).or_insert(0) += 1;
        }
        assert_eq!(kinds.len(), 3, "all kinds seen: {kinds:?}");
    }

    fn route(kind: Kind) -> Route {
        Route { agv: AgvKey { site: "s".into(), manufacturer: "M".into(), serial_number: "1".into() }, kind }
    }

    const CONNECTION: &str = r#"{"headerId":1,"timestamp":"2025-01-01T00:00:00.123Z","version":"2.1.0","manufacturer":"M","serialNumber":"1","connectionState":"ONLINE"}"#;

    fn with(json: &str, edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn decodes_timestamp_in_micros() {
        let message = decode(&route(Kind::Connection), CONNECTION.as_bytes()).unwrap();
        assert_eq!(message.sent_at, 1_735_689_600_123_000);
        assert_eq!(message.body, Body::Connection(Connection { header_id: 1, connection_state: ConnectionState::Online }));
    }

    #[test]
    fn rejects_bad_messages() {
        let connection = route(Kind::Connection);
        assert_eq!(decode(&connection, b"{not json"), Err(Reject::BadJson));
        let no_timestamp = with(CONNECTION, |v| {
            v.as_object_mut().unwrap().remove("timestamp");
        });
        assert_eq!(decode(&connection, &no_timestamp), Err(Reject::MissingTimestamp));
        let bad_timestamp = with(CONNECTION, |v| v["timestamp"] = "yesterday".into());
        assert_eq!(decode(&connection, &bad_timestamp), Err(Reject::BadTimestamp));
        let number_timestamp = with(CONNECTION, |v| v["timestamp"] = 5.into());
        assert_eq!(decode(&connection, &number_timestamp), Err(Reject::Schema));
        let other_agv = with(CONNECTION, |v| v["serialNumber"] = "2".into());
        assert_eq!(decode(&connection, &other_agv), Err(Reject::HeaderMismatch));
        let bad_state = with(CONNECTION, |v| v["connectionState"] = "SLEEPING".into());
        assert_eq!(decode(&connection, &bad_state), Err(Reject::Schema));
        assert_eq!(decode(&route(Kind::State), CONNECTION.as_bytes()), Err(Reject::Schema), "connection on a state topic");
    }

    #[test]
    fn visualization_needs_a_position() {
        let json = r#"{"timestamp":"2025-01-01T00:00:00Z","velocity":{"vx":1}}"#;
        assert_eq!(decode(&route(Kind::Visualization), json.as_bytes()), Err(Reject::Schema));
    }
}
