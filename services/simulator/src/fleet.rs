//! Fleet composition: which sites exist, what their warehouse maps look like,
//! and which robots (manufacturer, model, serial number) work at each site.
//!
//! Everything is derived from a seed, so the same seed always produces the
//! same fleet and therefore the same MQTT topics across restarts.

use std::collections::HashSet;

/// A robot type. Real AGV/AMR manufacturers that support VDA 5050; names are
/// labels only, speeds and load positions are simulation values, not vendor
/// specs. Names only use characters VDA 5050 allows in topic levels
/// (A-Z a-z 0-9 _ - . :).
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub manufacturer: &'static str,
    pub model: &'static str,
    pub load_position: &'static str,
    /// m/s
    pub speed: f64,
}

pub const MODELS: [Model; 7] = [
    model("KUKA", "KMP1500", "deck", 1.0),
    model("Jungheinrich", "ERC215a", "forks", 1.4),
    model("Linde", "L-MATIC", "forks", 1.2),
    model("STILL", "FM-X", "forks", 1.3),
    model("AGILOX", "ONE", "forks", 1.0),
    model("MiR", "MiR1350", "deck", 1.2),
    model("ek-robotics", "VarioMove", "forks", 1.5),
];

const fn model(manufacturer: &'static str, model: &'static str, load_position: &'static str, speed: f64) -> Model {
    Model { manufacturer, model, load_position, speed }
}

/// (siteId, display name)
pub const SITES: [(&str, &str); 8] = [
    ("hamburg", "Hamburg DC"),
    ("rotterdam", "Rotterdam DC"),
    ("poznan", "Poznań DC"),
    ("lyon", "Lyon DC"),
    ("milan", "Milan DC"),
    ("madrid", "Madrid DC"),
    ("prague", "Prague DC"),
    ("vienna", "Vienna DC"),
];

// Warehouse layout (metres), shared by every site map:
// - docks along the south wall (y = 0.5), one every 6 m from x = 6
// - a main corridor at y = CORRIDOR_Y that every route uses
// - rack aisles every 5 m from x = 5, with slots every 3 m from y = 5
// - one charger per robot along the west wall (x = 1), every 3 m from y = 4
pub const CORRIDOR_Y: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct FleetConfig {
    pub seed: String,
    pub site_count: usize,
    pub min_robots: usize,
    pub max_robots: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub site_id: &'static str,
    pub name: &'static str,
    pub map: SiteMap,
    pub robots: Vec<Robot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SiteMap {
    pub map_id: String,
    pub map_version: &'static str,
    pub docks: Vec<MapNode>,
    /// x of each rack aisle
    pub aisles: Vec<f64>,
    /// y of each rack slot
    pub slots: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapNode {
    pub id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Robot {
    pub model: Model,
    pub serial_number: String,
    pub charger: MapNode,
}

pub fn build_fleet(config: &FleetConfig) -> Result<Vec<Site>, String> {
    if config.site_count < 1 || config.site_count > SITES.len() {
        return Err(format!("site count must be between 1 and {}", SITES.len()));
    }
    if config.min_robots < 1 || config.max_robots < config.min_robots {
        return Err("robots per site must satisfy 1 <= min <= max".into());
    }

    let mut rng = Rng::new(&format!("{}:fleet", config.seed));
    let mut taken = HashSet::new();

    let sites = SITES[..config.site_count]
        .iter()
        .map(|&(site_id, name)| {
            let docks = (0..rng.int(3, 5))
                .map(|i| MapNode { id: format!("DOCK-{}", i + 1), x: 6.0 * (i + 1) as f64, y: 0.5 })
                .collect();
            let aisles = (0..rng.int(4, 6)).map(|i| 5.0 * (i + 1) as f64).collect();
            let slots = (0..rng.int(4, 6)).map(|i| 5.0 + 3.0 * i as f64).collect();
            let map = SiteMap { map_id: format!("{site_id}-warehouse"), map_version: "1.0", docks, aisles, slots };

            let robot_count = rng.int(config.min_robots as i64, config.max_robots as i64);
            let robots = (0..robot_count)
                .map(|i| {
                    let model = rng.pick(&MODELS).clone();
                    let serial_number = loop {
                        let serial = format!("{}-{}", model.model, rng.int(10000, 99999));
                        if taken.insert(format!("{}/{serial}", model.manufacturer)) {
                            break serial;
                        }
                    };
                    let charger = MapNode { id: format!("CHARGER-{}", i + 1), x: 1.0, y: 4.0 + 3.0 * i as f64 };
                    Robot { model, serial_number, charger }
                })
                .collect();

            Site { site_id, name, map, robots }
        })
        .collect();
    Ok(sites)
}

/// Small seeded PRNG (FNV-1a hash of the seed feeding mulberry32).
/// Simulations need repeatable randomness, including across the earlier
/// JavaScript simulator: the same seed yields the same fleet and topics.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: &str) -> Self {
        let mut state: u32 = 2166136261;
        for c in seed.chars() {
            state = (state ^ c as u32).wrapping_mul(16777619);
        }
        Self { state }
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b79f5);
        let s = self.state;
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }

    pub fn float(&mut self, min: f64, max: f64) -> f64 {
        min + self.next_f64() * (max - min)
    }

    /// Uniform integer in [min, max].
    pub fn int(&mut self, min: i64, max: i64) -> i64 {
        min + (self.next_f64() * (max - min + 1) as f64).floor() as i64
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next_f64() * items.len() as f64).floor() as usize]
    }
}
