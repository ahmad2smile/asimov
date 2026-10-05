//! Config parts shared by every ingest service, and loading of the JSON file.
//!
//! Each service has its own top-level struct that embeds these parts, so
//! unknown fields are rejected at every level (`deny_unknown_fields`).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Reads and parses the config file at `CONFIG_PATH`, or `default_path`.
pub fn load<T: DeserializeOwned>(default_path: &str) -> Result<T> {
    let path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| default_path.to_owned());
    load_from(Path::new(&path))
}

pub fn load_from<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading config {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MqttConfig {
    /// `mqtt://host:port`
    pub url: String,
    /// The client id is `<clientIdPrefix>-<hostname>`, unique per instance.
    pub client_id_prefix: String,
    /// Shared subscription group. Instances with the same group split the
    /// `state` and `visualization` traffic.
    pub group: String,
    #[serde(default = "default_keep_alive_secs")]
    pub keep_alive_secs: u16,
    /// How long the broker keeps this instance's session while it is away.
    #[serde(default = "default_session_expiry_secs")]
    pub session_expiry_secs: u32,
}

fn default_keep_alive_secs() -> u16 {
    15
}

fn default_session_expiry_secs() -> u32 {
    3600
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Site {
    pub id: String,
    /// Topic levels before `<manufacturer>/<serialNumber>/<topic>`,
    /// e.g. `hamburg/uagv/v2`.
    pub topic_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TelemetryConfig {
    /// OTLP gRPC endpoint for metrics; metrics are off when unset.
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    #[serde(default = "default_export_interval_secs")]
    pub export_interval_secs: u64,
    /// Port of `/healthz` and `/readyz`.
    #[serde(default = "default_health_port")]
    pub health_port: u16,
}

fn default_export_interval_secs() -> u64 {
    10
}

fn default_health_port() -> u16 {
    8080
}

impl MqttConfig {
    pub fn validate(&self) -> Result<()> {
        if !(self.url.starts_with("mqtt://") || self.url.starts_with("tcp://")) {
            bail!("mqtt.url must start with mqtt:// or tcp://, got {:?}", self.url);
        }
        if self.url.contains('?') {
            bail!("mqtt.url must not have query parameters, got {:?}", self.url);
        }
        if self.client_id_prefix.is_empty() {
            bail!("mqtt.clientIdPrefix must not be empty");
        }
        if !is_topic_level(&self.group) {
            bail!("mqtt.group must be one topic level without wildcards, got {:?}", self.group);
        }
        if self.keep_alive_secs < 5 {
            bail!("mqtt.keepAliveSecs must be at least 5");
        }
        Ok(())
    }
}

/// Checks that sites are usable and do not overlap, so every topic belongs to
/// exactly one site.
pub fn validate_sites(sites: &[Site]) -> Result<()> {
    if sites.is_empty() {
        bail!("sites must list at least one site");
    }
    let mut ids = HashSet::new();
    for site in sites {
        if !is_topic_level(&site.id) {
            bail!("site id must be one topic level without wildcards, got {:?}", site.id);
        }
        if !ids.insert(&site.id) {
            bail!("site id {:?} is listed twice", site.id);
        }
        let prefix = &site.topic_prefix;
        if prefix.is_empty() || prefix.starts_with('$') || !prefix.split('/').all(is_topic_level) {
            bail!("site {:?}: topicPrefix must be topic levels without wildcards, got {:?}", site.id, prefix);
        }
    }
    for a in sites {
        for b in sites {
            if a != b && (a.topic_prefix == b.topic_prefix || b.topic_prefix.starts_with(&format!("{}/", a.topic_prefix))) {
                bail!("sites {:?} and {:?} have overlapping topic prefixes", a.id, b.id);
            }
        }
    }
    Ok(())
}

/// One non-empty topic level without wildcards or separators.
fn is_topic_level(value: &str) -> bool {
    !value.is_empty() && !value.contains(['/', '+', '#', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(id: &str, prefix: &str) -> Site {
        Site { id: id.into(), topic_prefix: prefix.into() }
    }

    fn mqtt() -> MqttConfig {
        serde_json::from_str(r#"{"url":"mqtt://broker:1883","clientIdPrefix":"x","group":"g"}"#).unwrap()
    }

    #[test]
    fn valid_config_parses_with_defaults() {
        let m = mqtt();
        assert_eq!((m.keep_alive_secs, m.session_expiry_secs), (15, 3600));
        m.validate().unwrap();
        validate_sites(&[site("hamburg", "hamburg/uagv/v2"), site("lyon", "lyon/uagv/v2")]).unwrap();
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = serde_json::from_str::<MqttConfig>(r#"{"url":"mqtt://b","clientIdPrefix":"x","group":"g","grup":"h"}"#);
        assert!(error.is_err());
        assert!(serde_json::from_str::<Site>(r#"{"id":"a","topicPrefix":"a","x":1}"#).is_err());
    }

    #[test]
    fn missing_group_is_rejected() {
        assert!(serde_json::from_str::<MqttConfig>(r#"{"url":"mqtt://b","clientIdPrefix":"x"}"#).is_err());
    }

    #[test]
    fn bad_mqtt_values_are_rejected() {
        for (field, value) in [("url", "http://b"), ("url", "mqtt://b?x=1"), ("group", "a/b"), ("group", "+"), ("clientIdPrefix", "")] {
            let mut json: serde_json::Value = serde_json::to_value(serde_json::json!({"url":"mqtt://b","clientIdPrefix":"x","group":"g"})).unwrap();
            json[field] = value.into();
            let config: MqttConfig = serde_json::from_value(json).unwrap();
            assert!(config.validate().is_err(), "{field} = {value:?} should be rejected");
        }
    }

    #[test]
    fn bad_sites_are_rejected() {
        assert!(validate_sites(&[]).is_err(), "empty");
        assert!(validate_sites(&[site("a", "a/v2"), site("a", "b/v2")]).is_err(), "duplicate id");
        assert!(validate_sites(&[site("a", "x/v2"), site("b", "x/v2")]).is_err(), "same prefix");
        assert!(validate_sites(&[site("a", "x"), site("b", "x/v2")]).is_err(), "nested prefix");
        for prefix in ["", "a/+/v2", "a/#", "/a", "a/", "a//b", "$SYS/a"] {
            assert!(validate_sites(&[site("a", prefix)]).is_err(), "prefix {prefix:?}");
        }
        assert!(validate_sites(&[site("a/b", "a")]).is_err(), "id with a separator");
        assert!(validate_sites(&[site("x", "ab/v2"), site("y", "a/v2")]).is_ok(), "shared text, not shared levels");
    }
}
