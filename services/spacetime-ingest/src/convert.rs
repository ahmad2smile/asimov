//! VDA 5050 messages as the module's `ingest` reducer takes them.

use ingest_core::vda::{self, Body, Message};
use spacetimedb_sdk::Timestamp;

use crate::module_bindings::{
    AgvError, AgvStateFields, ConnectionState, ErrorLevel, IngestBody, IngestMessage, IngestState,
};

/// None for `visualization`, which SpacetimeDB does not store.
pub fn to_ingest(message: &Message) -> Option<IngestMessage> {
    let body = match &message.body {
        Body::State(state) => IngestBody::State(IngestState {
            map_id: state.map_id().map(str::to_owned),
            state: state_fields(state),
        }),
        Body::Connection(connection) => IngestBody::Connection(match connection.connection_state {
            vda::ConnectionState::Online => ConnectionState::Online,
            vda::ConnectionState::Offline => ConnectionState::Offline,
            vda::ConnectionState::Connectionbroken => ConnectionState::Connectionbroken,
        }),
        Body::Visualization(_) => return None,
    };
    Some(IngestMessage {
        manufacturer: message.agv.manufacturer.clone(),
        serial_number: message.agv.serial_number.clone(),
        sent_at: Timestamp::from_micros_since_unix_epoch(message.sent_at),
        body,
    })
}

/// The dashboard's view of a VDA 5050 `state`: the latest error is the last
/// one reported.
fn state_fields(state: &vda::State) -> AgvStateFields {
    AgvStateFields {
        order_id: state.order_id.clone(),
        last_node_id: state.last_node_id.clone(),
        error: state.errors.last().map(|error| AgvError {
            error_type: error.error_type.clone(),
            error_level: match error.error_level {
                vda::ErrorLevel::Warning => ErrorLevel::Warning,
                vda::ErrorLevel::Fatal => ErrorLevel::Fatal,
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use ingest_core::topic::AgvKey;
    use ingest_core::vda::{BatteryState, Error, Position, SafetyState, State};

    use super::*;

    fn state(map_id: Option<&str>, errors: Vec<Error>) -> Message {
        Message {
            agv: AgvKey {
                site: "hamburg".into(),
                manufacturer: "KUKA".into(),
                serial_number: "1".into(),
            },
            sent_at: 1_000_000,
            body: Body::State(State {
                header_id: 0,
                order_id: "o-1".into(),
                order_update_id: 0,
                last_node_id: "n-1".into(),
                driving: false,
                paused: None,
                operating_mode: "AUTOMATIC".into(),
                agv_position: map_id.map(|map_id| Position {
                    x: 0.0,
                    y: 0.0,
                    theta: 0.0,
                    map_id: map_id.into(),
                    localization_score: None,
                }),
                battery_state: BatteryState {
                    battery_charge: 50.0,
                    charging: false,
                },
                loads: None,
                errors,
                safety_state: SafetyState {
                    e_stop: "NONE".into(),
                    field_violation: false,
                },
            }),
        }
    }

    fn ingest_state(message: &Message) -> IngestState {
        match to_ingest(message).unwrap().body {
            IngestBody::State(state) => state,
            body => panic!("not a state: {body:?}"),
        }
    }

    #[test]
    fn state_carries_its_map_and_the_last_error() {
        let message = state(Some("m"), Vec::new());
        let ingest = to_ingest(&message).unwrap();
        assert_eq!(
            (ingest.manufacturer.as_str(), ingest.serial_number.as_str()),
            ("KUKA", "1")
        );
        assert_eq!(
            ingest.sent_at,
            Timestamp::from_micros_since_unix_epoch(1_000_000)
        );
        let fields = ingest_state(&message);
        assert_eq!(fields.map_id.as_deref(), Some("m"));
        assert_eq!(fields.state.error, None);

        let errors = vec![
            Error {
                error_type: "batteryLow".into(),
                error_level: vda::ErrorLevel::Warning,
                error_description: None,
            },
            Error {
                error_type: "obstacle".into(),
                error_level: vda::ErrorLevel::Fatal,
                error_description: None,
            },
        ];
        let fields = ingest_state(&state(None, errors));
        assert_eq!(fields.map_id, None, "no position, no map");
        assert_eq!(fields.state.order_id, "o-1");
        assert_eq!(
            fields.state.error,
            Some(AgvError {
                error_type: "obstacle".into(),
                error_level: ErrorLevel::Fatal
            })
        );
    }
}
