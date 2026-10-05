//! VDA 5050 messages as TDengine SQL.
//!
//! One super table per message kind; one subtable per AGV (and per map for
//! `state` and `visualization`). TDengine creates a subtable on its first
//! insert (`USING ... TAGS`), so only the super tables are created up front.
//! `state` holds no position: VDA 5050 sends it more often in `visualization`.

use std::collections::HashMap;

use ingest_core::topic::Kind;
use ingest_core::vda::{Body, Message};
use sha2::{Digest, Sha256};

/// Creates the database and the super tables if missing.
pub fn create_sql(database: &str) -> [String; 4] {
    let tags = "site VARCHAR(64), manufacturer VARCHAR(128), serial_number VARCHAR(128)";
    [
        format!("CREATE DATABASE IF NOT EXISTS `{database}` PRECISION 'ms'"),
        format!(
            "CREATE STABLE IF NOT EXISTS `{database}`.agv_visualization (ts TIMESTAMP, x DOUBLE, y DOUBLE, \
             theta DOUBLE, vx DOUBLE, vy DOUBLE, omega DOUBLE, localization_score DOUBLE) \
             TAGS ({tags}, map_id VARCHAR(128))"
        ),
        format!(
            "CREATE STABLE IF NOT EXISTS `{database}`.agv_state (ts TIMESTAMP, header_id INT UNSIGNED, \
             order_id VARCHAR(256), order_update_id INT UNSIGNED, last_node_id VARCHAR(256), driving BOOL, \
             paused BOOL, operating_mode VARCHAR(16), battery_charge DOUBLE, charging BOOL, load_count INT, \
             error_count INT, errors VARCHAR(8192), e_stop VARCHAR(16), field_violation BOOL) \
             TAGS ({tags}, map_id VARCHAR(128))"
        ),
        // VDA 5050 `connection` has no map.
        format!(
            "CREATE STABLE IF NOT EXISTS `{database}`.agv_connection (ts TIMESTAMP, header_id INT UNSIGNED, \
             connection_state VARCHAR(16)) TAGS ({tags})"
        ),
    ]
}

pub fn stable(kind: Kind) -> &'static str {
    match kind {
        Kind::Visualization => "agv_visualization",
        Kind::State => "agv_state",
        Kind::Connection => "agv_connection",
    }
}

/// One message: the subtable and its tags (`head`), and its values.
pub struct Row {
    head: String,
    values: String,
}

impl Row {
    /// Upper bound of its bytes in a statement.
    pub fn size(&self) -> usize {
        self.head.len() + self.values.len() + 2
    }
}

/// Many rows go out as one `INSERT INTO <head> VALUES (..) (..) <head> ...`.
/// Rows of one subtable share one head, so the server parses its tags once.
pub fn insert_sql<'a>(rows: impl IntoIterator<Item = &'a Row>) -> String {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for row in rows {
        let at = *index.entry(&row.head).or_insert_with(|| {
            groups.push((&row.head, Vec::new()));
            groups.len() - 1
        });
        groups[at].1.push(&row.values);
    }

    let mut sql = String::from("INSERT INTO");
    for (head, values) in groups {
        sql.push_str(&format!(" {head} VALUES {}", values.join(" ")));
    }
    sql
}

/// One message as a `Row`.
pub fn row(database: &str, message: &Message) -> Row {
    let agv = &message.agv;
    let mut tags = vec![agv.site.as_str(), &agv.manufacturer, &agv.serial_number];
    let values = match &message.body {
        Body::Visualization(v) => {
            let p = &v.agv_position;
            let velocity = v.velocity.as_ref();
            tags.push(&p.map_id);
            vec![
                p.x.to_string(),
                p.y.to_string(),
                p.theta.to_string(),
                or_null(velocity.and_then(|v| v.vx)),
                or_null(velocity.and_then(|v| v.vy)),
                or_null(velocity.and_then(|v| v.omega)),
                or_null(p.localization_score),
            ]
        }
        Body::State(s) => {
            // No position: the map is unknown, kept as an empty tag.
            tags.push(s.map_id().unwrap_or_default());
            vec![
                s.header_id.to_string(),
                text(&s.order_id),
                s.order_update_id.to_string(),
                text(&s.last_node_id),
                s.driving.to_string(),
                or_null(s.paused),
                text(&s.operating_mode),
                s.battery_state.battery_charge.to_string(),
                s.battery_state.charging.to_string(),
                s.loads.as_ref().map_or(0, Vec::len).to_string(),
                s.errors.len().to_string(),
                text(&serde_json::to_string(&s.errors).expect("errors serialize")),
                text(&s.safety_state.e_stop),
                s.safety_state.field_violation.to_string(),
            ]
        }
        Body::Connection(c) => vec![c.header_id.to_string(), text(c.connection_state.as_str())],
    };
    let kind = message.kind();
    // Milliseconds, the database precision.
    let ts = message.sent_at / 1000;
    let head = format!(
        "`{database}`.`{}` USING `{database}`.{} TAGS ({})",
        subtable(kind, &tags),
        stable(kind),
        tags.iter()
            .map(|tag| text(tag))
            .collect::<Vec<_>>()
            .join(", "),
    );
    Row {
        head,
        values: format!("({ts}, {})", values.join(", ")),
    }
}

/// `<kind>_<hash of tags>`: safe for any characters in the tags, and the same
/// AGV always lands in the same subtable.
fn subtable(kind: Kind, tags: &[&str]) -> String {
    let mut hash = Sha256::new();
    for tag in tags {
        // Length first, so moving characters between tags changes the hash.
        hash.update(tag.len().to_le_bytes());
        hash.update(tag.as_bytes());
    }
    let hex: String = hash.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let short = match kind {
        Kind::Visualization => "vis",
        Kind::State => "state",
        Kind::Connection => "conn",
    };
    format!("{short}_{hex}")
}

/// A quoted string literal.
fn text(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn or_null(value: Option<impl ToString>) -> String {
    value.map_or_else(|| "NULL".into(), |v| v.to_string())
}

#[cfg(test)]
mod tests {
    use ingest_core::topic::AgvKey;
    use ingest_core::vda::{
        BatteryState, Connection, ConnectionState, Error, ErrorLevel, Position, SafetyState, State,
        Velocity, Visualization,
    };

    use super::*;

    fn agv(serial: &str) -> AgvKey {
        AgvKey {
            site: "hamburg".into(),
            manufacturer: "KUKA".into(),
            serial_number: serial.into(),
        }
    }

    fn position(map_id: &str) -> Position {
        Position {
            x: 1.0,
            y: 2.5,
            theta: 0.5,
            map_id: map_id.into(),
            localization_score: Some(0.9),
        }
    }

    fn visualization(serial: &str, map_id: &str) -> Message {
        Message {
            agv: agv(serial),
            sent_at: 1_735_689_600_123_456,
            body: Body::Visualization(Visualization {
                agv_position: position(map_id),
                velocity: Some(Velocity {
                    vx: Some(0.5),
                    vy: None,
                    omega: Some(0.1),
                }),
            }),
        }
    }

    fn state(errors: Vec<Error>, position: Option<Position>) -> Message {
        Message {
            agv: agv("1"),
            sent_at: 1_000_000,
            body: Body::State(State {
                header_id: 7,
                order_id: "o-1".into(),
                order_update_id: 2,
                last_node_id: "n-1".into(),
                driving: true,
                paused: None,
                operating_mode: "AUTOMATIC".into(),
                agv_position: position,
                battery_state: BatteryState {
                    battery_charge: 80.0,
                    charging: false,
                },
                loads: Some(vec![serde_json::json!({})]),
                errors,
                safety_state: SafetyState {
                    e_stop: "NONE".into(),
                    field_violation: false,
                },
            }),
        }
    }

    /// The head after the subtable name, which is a hash.
    fn after_subtable(row: &Row) -> &str {
        &row.head[row.head.find(" USING ").unwrap()..]
    }

    #[test]
    fn visualization_part() {
        let row = row("asimov", &visualization("1", "m"));
        assert!(row.head.starts_with("`asimov`.`vis_"));
        assert_eq!(
            after_subtable(&row),
            " USING `asimov`.agv_visualization TAGS ('hamburg', 'KUKA', '1', 'm')"
        );
        assert_eq!(
            row.values,
            "(1735689600123, 1, 2.5, 0.5, 0.5, NULL, 0.1, 0.9)"
        );
    }

    #[test]
    fn state_part_keeps_errors_as_json_and_no_position() {
        let errors = vec![Error {
            error_type: "obstacle".into(),
            error_level: ErrorLevel::Fatal,
            error_description: None,
        }];
        let row = row("asimov", &state(errors, None));
        assert_eq!(
            after_subtable(&row),
            " USING `asimov`.agv_state TAGS ('hamburg', 'KUKA', '1', '')"
        );
        assert_eq!(
            row.values,
            "(1000, 7, 'o-1', 2, 'n-1', true, NULL, 'AUTOMATIC', 80, false, 1, 1, \
             '[{\"errorType\":\"obstacle\",\"errorLevel\":\"FATAL\"}]', 'NONE', false)"
        );
    }

    #[test]
    fn connection_part_has_no_map() {
        let message = Message {
            agv: agv("1"),
            sent_at: 0,
            body: Body::Connection(Connection {
                header_id: 1,
                connection_state: ConnectionState::Connectionbroken,
            }),
        };
        let row = row("asimov", &message);
        assert_eq!(
            after_subtable(&row),
            " USING `asimov`.agv_connection TAGS ('hamburg', 'KUKA', '1')"
        );
        assert_eq!(row.values, "(0, 1, 'CONNECTIONBROKEN')");
    }

    #[test]
    fn quotes_and_backslashes_stay_inside_the_string() {
        assert_eq!(text(r"it's a \ path"), r"'it\'s a \\ path'");
        let mut message = visualization("1", "m");
        message.agv.serial_number = r"x\', 'y".into();
        assert!(
            after_subtable(&row("asimov", &message))
                .contains(r"TAGS ('hamburg', 'KUKA', 'x\\\', \'y', 'm')")
        );
    }

    #[test]
    fn subtable_depends_on_every_tag() {
        let name = |message: &Message| {
            row("asimov", message)
                .head
                .split('`')
                .nth(3)
                .unwrap()
                .to_owned()
        };
        let a = name(&visualization("1", "m"));
        assert_eq!(a, name(&visualization("1", "m")));
        assert_ne!(a, name(&visualization("2", "m")));
        assert_ne!(
            a,
            name(&visualization("1", "n")),
            "another map, another subtable"
        );
        assert!(a.starts_with("vis_") && a.len() == 4 + 32);
        let mut shifted = visualization("1", "m");
        shifted.agv.manufacturer = "KUKA1".into();
        shifted.agv.serial_number = String::new();
        assert_ne!(a, name(&shifted));
    }

    #[test]
    fn insert_shares_the_head_of_a_subtable() {
        let rows = [
            row("asimov", &visualization("1", "m")),
            row("asimov", &visualization("2", "m")),
            row("asimov", &visualization("1", "m")),
        ];
        let sql = insert_sql(&rows);
        assert_eq!(sql.matches(" USING ").count(), 2, "one head per subtable");
        assert_eq!(
            sql,
            format!(
                "INSERT INTO {} VALUES {} {} {} VALUES {}",
                rows[0].head, rows[0].values, rows[2].values, rows[1].head, rows[1].values
            )
        );
    }
}
