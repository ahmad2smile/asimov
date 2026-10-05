//! Simulated warehouse AGV fleet publishing VDA 5050 2.1.0 messages.
//! `fleet` decides who works where; `agv` simulates one vehicle and builds
//! its messages. The MQTT runtime lives in `main.rs`.

pub mod agv;
pub mod fleet;
