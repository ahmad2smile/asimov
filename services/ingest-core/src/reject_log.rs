//! Rate-limited logging of rejected messages.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

/// Logs a rejected message at most once per reason every 10 s, with how many
/// were not logged, so a flood of bad messages cannot flood the logs.
#[derive(Default)]
pub(crate) struct RejectLog {
    last: HashMap<&'static str, (Instant, u64)>,
}

const REJECT_LOG_INTERVAL: Duration = Duration::from_secs(10);

impl RejectLog {
    pub(crate) fn log(&mut self, reason: &'static str, topic: &str) {
        let now = Instant::now();
        match self.last.get_mut(reason) {
            Some((at, suppressed)) if now.duration_since(*at) < REJECT_LOG_INTERVAL => *suppressed += 1,
            entry => {
                let suppressed = entry.map_or(0, |(_, suppressed)| *suppressed);
                tracing::warn!(reason, topic, suppressed, "rejected MQTT message");
                self.last.insert(reason, (now, 0));
            }
        }
    }
}
