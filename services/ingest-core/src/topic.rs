//! MQTT topics of the configured sites: what to subscribe to, and which site
//! and AGV a received topic belongs to.
//!
//! Topic: `<topicPrefix>/<manufacturer>/<serialNumber>/<kind>`.

use std::fmt;

use crate::config::Site;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    State,
    Visualization,
    Connection,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::State, Kind::Visualization, Kind::Connection];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::State => "state",
            Kind::Visualization => "visualization",
            Kind::Connection => "connection",
        }
    }

    fn parse(level: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.as_str() == level)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One AGV at one site. VDA 5050 identifies an AGV by manufacturer and serial
/// number; the site comes from the topic prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgvKey {
    pub site: String,
    pub manufacturer: String,
    pub serial_number: String,
}

impl fmt::Display for AgvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}/{}", self.site, self.manufacturer, self.serial_number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub agv: AgvKey,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// No configured site's prefix matches (e.g. a site removed from the
    /// config while the broker still has its subscription).
    UnknownSite,
    /// The site matches but the rest is not `<manufacturer>/<serial>/<kind>`.
    BadTopic,
}

impl RouteError {
    pub fn reason(self) -> &'static str {
        match self {
            RouteError::UnknownSite => "unknown_site",
            RouteError::BadTopic => "bad_topic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub filter: String,
    /// Retained `connection` messages are not sent to shared subscriptions,
    /// so `connection` is subscribed by every instance.
    pub shared: bool,
}

pub struct Router {
    sites: Vec<Site>,
}

impl Router {
    pub fn new(sites: Vec<Site>) -> Self {
        Self { sites }
    }

    pub fn sites(&self) -> &[Site] {
        &self.sites
    }

    /// `state` and `visualization` are shared within `group`, so instances of
    /// one service split them; `connection` goes to every instance.
    pub fn subscriptions(&self, group: &str, kinds: &[Kind]) -> Vec<Subscription> {
        let mut subscriptions = Vec::new();
        for site in &self.sites {
            for &kind in kinds {
                let topic = format!("{}/+/+/{kind}", site.topic_prefix);
                subscriptions.push(match kind {
                    Kind::Connection => Subscription { filter: topic, shared: false },
                    _ => Subscription { filter: format!("$share/{group}/{topic}"), shared: true },
                });
            }
        }
        subscriptions
    }

    pub fn route(&self, topic: &str) -> Result<Route, RouteError> {
        let site = self
            .sites
            .iter()
            .find(|site| topic.strip_prefix(&site.topic_prefix).is_some_and(|rest| rest.starts_with('/')))
            .ok_or(RouteError::UnknownSite)?;
        let rest = &topic[site.topic_prefix.len() + 1..];
        let mut levels = rest.split('/');
        let (Some(manufacturer), Some(serial_number), Some(kind), None) =
            (levels.next(), levels.next(), levels.next(), levels.next())
        else {
            return Err(RouteError::BadTopic);
        };
        if manufacturer.is_empty() || serial_number.is_empty() {
            return Err(RouteError::BadTopic);
        }
        let kind = Kind::parse(kind).ok_or(RouteError::BadTopic)?;
        Ok(Route {
            agv: AgvKey { site: site.id.clone(), manufacturer: manufacturer.into(), serial_number: serial_number.into() },
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router() -> Router {
        Router::new(vec![
            Site { id: "hamburg".into(), topic_prefix: "hamburg/uagv/v2".into() },
            Site { id: "lyon".into(), topic_prefix: "lyon/uagv/v2".into() },
        ])
    }

    #[test]
    fn routes_a_site_topic() {
        assert_eq!(
            router().route("lyon/uagv/v2/KUKA/KMP-1/state"),
            Ok(Route {
                agv: AgvKey { site: "lyon".into(), manufacturer: "KUKA".into(), serial_number: "KMP-1".into() },
                kind: Kind::State,
            })
        );
    }

    #[test]
    fn rejects_other_topics() {
        let r = router();
        assert_eq!(r.route("madrid/uagv/v2/KUKA/1/state"), Err(RouteError::UnknownSite));
        assert_eq!(r.route("hamburg/uagv/v2x/KUKA/1/state"), Err(RouteError::UnknownSite));
        assert_eq!(r.route("hamburg/uagv/v2"), Err(RouteError::UnknownSite));
        assert_eq!(r.route("hamburg/uagv/v2/KUKA/1"), Err(RouteError::BadTopic));
        assert_eq!(r.route("hamburg/uagv/v2/KUKA/1/state/x"), Err(RouteError::BadTopic));
        assert_eq!(r.route("hamburg/uagv/v2/KUKA/1/order"), Err(RouteError::BadTopic));
        assert_eq!(r.route("hamburg/uagv/v2//1/state"), Err(RouteError::BadTopic));
    }

    #[test]
    fn shares_all_but_connection() {
        let subscriptions = router().subscriptions("ingest", &Kind::ALL);
        assert_eq!(subscriptions.len(), 6);
        assert!(subscriptions.contains(&Subscription {
            filter: "$share/ingest/hamburg/uagv/v2/+/+/state".into(),
            shared: true
        }));
        assert!(subscriptions.contains(&Subscription {
            filter: "$share/ingest/lyon/uagv/v2/+/+/visualization".into(),
            shared: true
        }));
        assert!(subscriptions.contains(&Subscription { filter: "lyon/uagv/v2/+/+/connection".into(), shared: false }));
    }

    #[test]
    fn subscribes_only_the_requested_kinds() {
        let subscriptions = router().subscriptions("g", &[Kind::Connection]);
        assert_eq!(subscriptions.len(), 2);
    }
}
