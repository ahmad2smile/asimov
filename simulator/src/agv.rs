//! One simulated AGV and the VDA 5050 2.1.0 messages it publishes.
//!
//! The AGV acts as if a master control sent it one order per mission:
//!   drive to a dock -> pick a pallet -> drive to a rack slot -> drop it
//! and, when the battery runs low, an order to its charger with startCharging.
//! Order, node, edge, and action states follow the VDA 5050 life cycle, so a
//! master control or dashboard sees the same sequence as from a real vehicle.
//!
//! Spec: https://github.com/VDA5050/VDA5050/blob/release/2.1.0/VDA5050_EN.md

use chrono::{DateTime, SecondsFormat};
use serde::Serialize;

use crate::fleet::{CORRIDOR_Y, MapNode, Model, Rng, Robot, Site, SiteMap};

pub const PROTOCOL_VERSION: &str = "2.1.0";

const TURN_SPEED: f64 = 1.0; // rad/s
const MAX_STEP_SECONDS: f64 = 5.0; // cap simulated time if the process stalls
const DISPATCH_DELAY: (f64, f64) = (2.0, 8.0); // s between finishing an order and the next one
const PICK_DROP_INITIALIZING: f64 = 1.0; // s per actionStatus
const PICK_DROP_RUNNING: f64 = 6.0;
const CHARGER_HANDSHAKE: f64 = 2.0; // s that startCharging stays RUNNING
const BATTERY_LOW: f64 = 25.0; // % below which the next order goes to the charger
const BATTERY_FULL: f64 = 95.0; // % at which the AGV stops charging
const DRAIN_IDLE: f64 = 0.02; // %/s
const DRAIN_DRIVING: f64 = 0.12; // %/s, plus DRAIN_LOADED while carrying
const DRAIN_LOADED: f64 = 0.05;
const CHARGE_RATE: f64 = 0.8; // %/s
const OBSTACLE_CHANCE: f64 = 0.01; // per second of driving

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// No order running (possibly charging).
    Idle,
    /// Traversing the order's nodes.
    Driving,
    /// Running the action on the order's last node.
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionType {
    Pick,
    Drop,
    StartCharging,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionStatus {
    Waiting,
    Initializing,
    Running,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionState {
    Online,
    Offline,
    Connectionbroken,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct LastNode {
    node_id: String,
    sequence_id: u32,
}

#[derive(Debug, Clone)]
struct OrderNode {
    id: String,
    x: f64,
    y: f64,
    sequence_id: u32,
}

#[derive(Debug, Clone, Default)]
struct Order {
    order_id: String,
    nodes: Vec<OrderNode>,
    edges: Vec<EdgeState>,
    actions: Vec<ActionState>,
    /// Index of the next node to traverse.
    next_index: usize,
}

/// Fields whose change is a state-publishing event (spec 6.10): order,
/// node/edge/action states, driving, loads, errors, charging, safety.
#[derive(Debug, Clone, PartialEq)]
struct StateSignature {
    order_id: String,
    last_node: LastNode,
    next_index: usize,
    action_statuses: Vec<ActionStatus>,
    driving: bool,
    load_id: Option<String>,
    error_types: Vec<&'static str>,
    charging: bool,
}

#[derive(Debug, Clone, Copy)]
enum Topic {
    State,
    Visualization,
    Connection,
}

pub struct Agv {
    interface_name: String,
    pub manufacturer: String,
    pub serial_number: String,
    model: Model,
    charger: MapNode,
    map: SiteMap,
    rng: Rng,

    x: f64,
    y: f64,
    theta: f64,
    vx: f64,
    omega: f64,
    battery: f64,
    battery_health: f64,
    charging: bool,
    load: Option<Load>,
    blocked_for: f64,

    // `here` is the map node the AGV stands on or passed last (used for
    // routing); `last_node` is what the state reports. Before the first order
    // the spec requires lastNodeId "" and lastNodeSequenceId 0.
    here: MapNode,
    last_node: LastNode,
    distance_since_last_node: f64,
    order_count: u32,
    order: Order,

    phase: Phase,
    phase_remaining: f64,

    header_ids: [u32; 3],
    last_tick: Millis,
    last_state_at: Option<Millis>,
    last_state_signature: Option<StateSignature>,
}

impl Agv {
    pub fn new(interface_name: &str, site: &Site, robot: &Robot, mut rng: Rng, now: Millis) -> Self {
        let battery = rng.float(40.0, 100.0);
        let battery_health = rng.int(88, 100) as f64;
        let phase_remaining = rng.float(1.0, 10.0);
        Self {
            interface_name: interface_name.to_owned(),
            manufacturer: robot.model.manufacturer.to_owned(),
            serial_number: robot.serial_number.clone(),
            model: robot.model.clone(),
            charger: robot.charger.clone(),
            map: site.map.clone(),
            rng,
            x: robot.charger.x,
            y: robot.charger.y,
            theta: 0.0,
            vx: 0.0,
            omega: 0.0,
            battery,
            battery_health,
            charging: false,
            load: None,
            blocked_for: 0.0,
            here: robot.charger.clone(),
            last_node: LastNode::default(),
            distance_since_last_node: 0.0,
            order_count: 0,
            order: Order::default(),
            phase: Phase::Idle,
            phase_remaining,
            header_ids: [0; 3],
            last_tick: now,
            last_state_at: None,
            last_state_signature: None,
        }
    }

    pub fn topic(&self, name: &str) -> String {
        format!("{}/v2/{}/{}/{name}", self.interface_name, self.manufacturer, self.serial_number)
    }

    // --- Simulation ----------------------------------------------------------

    pub fn tick(&mut self, now: Millis) {
        let dt = ((now - self.last_tick) as f64 / 1000.0).clamp(0.0, MAX_STEP_SECONDS);
        self.last_tick = now;
        self.vx = 0.0;
        self.omega = 0.0;

        let moving = self.phase == Phase::Driving && self.blocked_for == 0.0;
        match self.phase {
            Phase::Idle => self.idle(dt),
            Phase::Driving => self.drive_or_wait(dt),
            Phase::Action => self.run_action(dt),
        }

        if self.charging {
            self.battery = (self.battery + CHARGE_RATE * dt).min(100.0);
        } else {
            let drain = if moving {
                DRAIN_DRIVING + if self.load.is_some() { DRAIN_LOADED } else { 0.0 }
            } else {
                DRAIN_IDLE
            };
            self.battery = (self.battery - drain * dt).max(0.0);
        }
    }

    fn idle(&mut self, dt: f64) {
        if self.charging {
            // startCharging finished when charging began; the vehicle itself ends
            // charging when the battery is full (allowed by the spec for stopCharging).
            if self.battery >= BATTERY_FULL {
                self.charging = false;
                self.phase_remaining = self.rng.float(DISPATCH_DELAY.0, DISPATCH_DELAY.1);
            }
            return;
        }
        self.phase_remaining -= dt;
        if self.phase_remaining <= 0.0 {
            self.accept_next_order();
        }
    }

    fn drive_or_wait(&mut self, dt: f64) {
        if self.blocked_for > 0.0 {
            self.blocked_for = (self.blocked_for - dt).max(0.0);
        } else if self.rng.next_f64() < OBSTACLE_CHANCE * dt {
            self.blocked_for = self.rng.int(3, 10) as f64;
        } else {
            self.drive(dt);
        }
    }

    /// Turn on the spot towards the next node, then drive straight to it.
    /// Velocity is reported as the average over the tick.
    fn drive(&mut self, dt: f64) {
        let mut budget = dt;
        let mut travelled = 0.0;
        let mut turned = 0.0;

        while budget > 0.0 && self.order.next_index < self.order.nodes.len() {
            let target = self.order.nodes[self.order.next_index].clone();
            let (dx, dy) = (target.x - self.x, target.y - self.y);
            let distance = dx.hypot(dy);

            if distance < 1e-6 {
                self.traverse_node(&target);
                continue;
            }

            let turn = normalize_angle(dy.atan2(dx) - self.theta);
            if turn.abs() > 1e-3 {
                let angle = turn.abs().min(TURN_SPEED * budget).copysign(turn);
                self.theta = normalize_angle(self.theta + angle);
                turned += angle;
                budget -= angle.abs() / TURN_SPEED;
                continue;
            }

            let travel = distance.min(self.model.speed * budget);
            self.x += dx / distance * travel;
            self.y += dy / distance * travel;
            self.distance_since_last_node += travel;
            travelled += travel;
            budget -= travel / self.model.speed;
        }

        if self.order.next_index >= self.order.nodes.len() {
            // Arrived: the AGV stands still while the node's action runs.
            self.trigger_node_actions();
        } else if dt > 0.0 {
            self.vx = travelled / dt;
            self.omega = turned / dt;
        }
    }

    /// Reaching a node removes it (and the edge leading to it) from the state.
    fn traverse_node(&mut self, node: &OrderNode) {
        self.x = node.x;
        self.y = node.y;
        self.here = MapNode { id: node.id.clone(), x: node.x, y: node.y };
        self.last_node = LastNode { node_id: node.id.clone(), sequence_id: node.sequence_id };
        self.distance_since_last_node = 0.0;
        self.order.next_index += 1;
    }

    // --- Orders and actions --------------------------------------------------

    /// Stand-in for master control: build the next mission as a VDA 5050 order.
    fn accept_next_order(&mut self) {
        let (destination, action_type, action_description) = match &self.load {
            None if self.battery < BATTERY_LOW => {
                let charger = self.charger.clone();
                let description = format!("Start charging at {}", charger.id);
                (charger, ActionType::StartCharging, description)
            }
            None => {
                let dock = self.rng.pick(&self.map.docks).clone();
                let description = format!("Pick EUR-pallet at {}", dock.id);
                (dock, ActionType::Pick, description)
            }
            Some(load) => {
                let aisle = *self.rng.pick(&self.map.aisles);
                let slot = *self.rng.pick(&self.map.slots);
                let rack = MapNode { id: format!("RACK-X{aisle}-Y{slot}"), x: aisle, y: slot };
                let description = format!("Drop {} at {}", load.load_id, rack.id);
                (rack, ActionType::Drop, description)
            }
        };

        self.order_count += 1;
        let order_id = format!("{}-order-{}", self.serial_number, self.order_count);

        // The order starts at the node the AGV stands on (sequenceId 0), which
        // counts as traversed as soon as the order is accepted.
        let start = self.here.clone();
        self.last_node = LastNode { node_id: start.id.clone(), sequence_id: 0 };
        self.distance_since_last_node = 0.0;

        let nodes: Vec<OrderNode> = route(&start, &destination)
            .into_iter()
            .zip(1..)
            .map(|(point, i)| OrderNode { id: point.id, x: point.x, y: point.y, sequence_id: 2 * i })
            .collect();
        let edges = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let from = if i == 0 { &start.id } else { &nodes[i - 1].id };
                EdgeState { edge_id: format!("{from}--{}", node.id), sequence_id: node.sequence_id - 1, released: true }
            })
            .collect();

        let empty = nodes.is_empty();
        self.order = Order {
            // New orders replace all previous action states.
            actions: vec![ActionState {
                action_id: format!("{order_id}-{}", action_type_name(action_type)),
                action_type,
                action_description,
                action_status: ActionStatus::Waiting,
                result_description: None,
            }],
            order_id,
            nodes,
            edges,
            next_index: 0,
        };
        self.phase = Phase::Driving;
        if empty {
            self.trigger_node_actions();
        }
    }

    /// Actions sit on the order's last node and start once it is traversed.
    fn trigger_node_actions(&mut self) {
        let action = &mut self.order.actions[0];
        self.phase = Phase::Action;
        if action.action_type == ActionType::StartCharging {
            // startCharging has no INITIALIZING stage; RUNNING = talking to charger.
            action.action_status = ActionStatus::Running;
            self.phase_remaining = CHARGER_HANDSHAKE;
        } else {
            action.action_status = ActionStatus::Initializing;
            self.phase_remaining = PICK_DROP_INITIALIZING;
        }
    }

    fn run_action(&mut self, dt: f64) {
        self.phase_remaining -= dt;
        if self.phase_remaining > 0.0 {
            return;
        }

        let action = &mut self.order.actions[0];
        if action.action_status == ActionStatus::Initializing {
            action.action_status = ActionStatus::Running;
            self.phase_remaining = PICK_DROP_RUNNING;
            return;
        }

        // FINISHED stays in the state until the next order arrives, so master
        // control sees the result before actionStates are replaced.
        action.action_status = ActionStatus::Finished;
        let result = match action.action_type {
            ActionType::Pick => {
                let load = Load {
                    load_id: format!("PAL-{}", self.rng.int(100000, 999999)),
                    load_type: "EUR-pallet",
                    load_position: self.model.load_position,
                    load_dimensions: LoadDimensions {
                        length: 1.2,
                        width: 0.8,
                        height: round(self.rng.float(0.6, 1.6), 2),
                    },
                    weight: self.rng.int(150, 900) as f64,
                };
                let result = format!("Picked {}", load.load_id);
                self.load = Some(load);
                result
            }
            ActionType::Drop => {
                let load = self.load.take().expect("drop orders are only created while loaded");
                format!("Dropped {}", load.load_id)
            }
            ActionType::StartCharging => {
                self.charging = true;
                "Charging started".to_owned()
            }
        };
        self.order.actions[0].result_description = Some(result);

        self.phase = Phase::Idle;
        self.phase_remaining = self.rng.float(DISPATCH_DELAY.0, DISPATCH_DELAY.1);
    }

    // --- VDA 5050 messages ---------------------------------------------------

    /// Returns a state message when a state-relevant event happened since the
    /// last one or `max_interval_ms` elapsed (spec: on events, at the latest
    /// every 30 s), otherwise `None`. `force` publishes regardless, e.g. after
    /// connecting.
    pub fn take_state(&mut self, now: Millis, max_interval_ms: i64, force: bool) -> Option<StateMessage> {
        let signature = self.state_signature();
        let due = self.last_state_at.is_none_or(|at| now - at >= max_interval_ms);
        if !force && !due && self.last_state_signature.as_ref() == Some(&signature) {
            return None;
        }
        self.last_state_signature = Some(signature);
        self.last_state_at = Some(now);
        Some(self.state_message(now))
    }

    fn state_signature(&self) -> StateSignature {
        StateSignature {
            order_id: self.order.order_id.clone(),
            last_node: self.last_node.clone(),
            next_index: self.order.next_index,
            action_statuses: self.order.actions.iter().map(|a| a.action_status).collect(),
            driving: self.is_driving(),
            load_id: self.load.as_ref().map(|load| load.load_id.clone()),
            error_types: self.errors().iter().map(|e| e.error_type).collect(),
            charging: self.charging,
        }
    }

    fn state_message(&mut self, now: Millis) -> StateMessage {
        let map_id = self.map.map_id.clone();
        let next = self.order.next_index;
        StateMessage {
            header: self.header(Topic::State, now),
            maps: vec![MapState { map_id: map_id.clone(), map_version: self.map.map_version, map_status: "ENABLED" }],
            order_id: self.order.order_id.clone(),
            order_update_id: 0,
            last_node_id: self.last_node.node_id.clone(),
            last_node_sequence_id: self.last_node.sequence_id,
            driving: self.is_driving(),
            paused: false,
            new_base_request: false,
            distance_since_last_node: round(self.distance_since_last_node, 3),
            operating_mode: "AUTOMATIC",
            node_states: self.order.nodes[next..]
                .iter()
                .map(|node| NodeState {
                    node_id: node.id.clone(),
                    sequence_id: node.sequence_id,
                    released: true,
                    node_position: NodePosition { x: node.x, y: node.y, map_id: map_id.clone() },
                })
                .collect(),
            edge_states: self.order.edges[next..].to_vec(),
            agv_position: self.position(),
            velocity: self.velocity(),
            loads: self.load.iter().cloned().collect(),
            action_states: self.order.actions.clone(),
            battery_state: BatteryState {
                battery_charge: round(self.battery, 1),
                battery_voltage: round(44.0 + self.battery / 10.0, 2),
                battery_health: self.battery_health,
                charging: self.charging,
                reach: (self.battery * 40.0).round() as u32,
            },
            errors: self.errors(),
            information: vec![],
            safety_state: SafetyState { e_stop: "NONE", field_violation: self.blocked_for > 0.0 },
        }
    }

    pub fn visualization_message(&mut self, now: Millis) -> VisualizationMessage {
        VisualizationMessage {
            header: self.header(Topic::Visualization, now),
            agv_position: self.position(),
            velocity: self.velocity(),
        }
    }

    pub fn connection_message(&mut self, connection_state: ConnectionState, now: Millis) -> ConnectionMessage {
        ConnectionMessage { header: self.header(Topic::Connection, now), connection_state }
    }

    /// headerId is counted per topic and increases with every message sent.
    fn header(&mut self, topic: Topic, now: Millis) -> Header {
        let id = &mut self.header_ids[topic as usize];
        let header_id = *id;
        *id += 1;
        Header {
            header_id,
            timestamp: DateTime::from_timestamp_millis(now)
                .expect("timestamp in range")
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            version: PROTOCOL_VERSION,
            manufacturer: self.manufacturer.clone(),
            serial_number: self.serial_number.clone(),
        }
    }

    /// `driving` covers driving and rotating, not load handling.
    fn is_driving(&self) -> bool {
        self.vx != 0.0 || self.omega != 0.0
    }

    fn position(&mut self) -> AgvPosition {
        AgvPosition {
            x: round(self.x, 3),
            y: round(self.y, 3),
            theta: round(self.theta, 4),
            map_id: self.map.map_id.clone(),
            position_initialized: true,
            localization_score: round(self.rng.float(0.9, 1.0), 3),
            deviation_range: 0.05,
        }
    }

    fn velocity(&self) -> Velocity {
        Velocity { vx: round(self.vx, 3), vy: 0.0, omega: round(self.omega, 3) }
    }

    fn errors(&self) -> Vec<Error> {
        let mut list = Vec::new();
        if self.blocked_for > 0.0 {
            list.push(Error {
                error_type: "obstacleDetected",
                error_level: "WARNING",
                error_description: "Protective field violated; waiting for the path to clear.".to_owned(),
                error_references: Some(vec![ErrorReference {
                    reference_key: "orderId",
                    reference_value: self.order.order_id.clone(),
                }]),
            });
        }
        if self.battery < BATTERY_LOW && !self.charging {
            list.push(Error {
                error_type: "batteryLow",
                error_level: "WARNING",
                error_description: format!("Battery at {}%.", round(self.battery, 1)),
                error_references: None,
            });
        }
        list
    }
}

/// Axis-aligned route via the main corridor. Returns the nodes after `from`;
/// the last one carries the destination's node ID.
fn route(from: &MapNode, to: &MapNode) -> Vec<MapNode> {
    let waypoints = [(from.x, CORRIDOR_Y), (to.x, CORRIDOR_Y), (to.x, to.y)];
    let mut nodes: Vec<MapNode> = Vec::new();
    let mut previous = (from.x, from.y);
    for (x, y) in waypoints {
        if (x, y) == previous {
            continue;
        }
        nodes.push(MapNode { id: format!("X{x}-Y{y}"), x, y });
        previous = (x, y);
    }
    if let Some(last) = nodes.last_mut() {
        last.id = to.id.clone();
    }
    nodes
}

fn action_type_name(action_type: ActionType) -> &'static str {
    match action_type {
        ActionType::Pick => "pick",
        ActionType::Drop => "drop",
        ActionType::StartCharging => "startCharging",
    }
}

fn normalize_angle(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}

fn round(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor).round() / factor
}

// --- Message shapes (field names as in the VDA 5050 JSON schemas) ------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub header_id: u32,
    pub timestamp: String,
    pub version: &'static str,
    pub manufacturer: String,
    pub serial_number: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMessage {
    #[serde(flatten)]
    pub header: Header,
    pub maps: Vec<MapState>,
    pub order_id: String,
    pub order_update_id: u32,
    pub last_node_id: String,
    pub last_node_sequence_id: u32,
    pub driving: bool,
    pub paused: bool,
    pub new_base_request: bool,
    pub distance_since_last_node: f64,
    pub operating_mode: &'static str,
    pub node_states: Vec<NodeState>,
    pub edge_states: Vec<EdgeState>,
    pub agv_position: AgvPosition,
    pub velocity: Velocity,
    pub loads: Vec<Load>,
    pub action_states: Vec<ActionState>,
    pub battery_state: BatteryState,
    pub errors: Vec<Error>,
    pub information: Vec<serde_json::Value>,
    pub safety_state: SafetyState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualizationMessage {
    #[serde(flatten)]
    pub header: Header,
    pub agv_position: AgvPosition,
    pub velocity: Velocity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionMessage {
    #[serde(flatten)]
    pub header: Header,
    pub connection_state: ConnectionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapState {
    pub map_id: String,
    pub map_version: &'static str,
    pub map_status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeState {
    pub node_id: String,
    pub sequence_id: u32,
    pub released: bool,
    pub node_position: NodePosition,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
    pub map_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeState {
    pub edge_id: String,
    pub sequence_id: u32,
    pub released: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgvPosition {
    pub x: f64,
    pub y: f64,
    pub theta: f64,
    pub map_id: String,
    pub position_initialized: bool,
    pub localization_score: f64,
    pub deviation_range: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Velocity {
    pub vx: f64,
    pub vy: f64,
    pub omega: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    pub load_id: String,
    pub load_type: &'static str,
    pub load_position: &'static str,
    pub load_dimensions: LoadDimensions,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadDimensions {
    pub length: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionState {
    pub action_id: String,
    pub action_type: ActionType,
    pub action_description: String,
    pub action_status: ActionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryState {
    pub battery_charge: f64,
    pub battery_voltage: f64,
    pub battery_health: f64,
    pub charging: bool,
    pub reach: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub error_type: &'static str,
    pub error_level: &'static str,
    pub error_description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_references: Option<Vec<ErrorReference>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReference {
    pub reference_key: &'static str,
    pub reference_value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyState {
    pub e_stop: &'static str,
    pub field_violation: bool,
}
