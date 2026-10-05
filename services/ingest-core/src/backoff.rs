//! Exponential backoff with full jitter, for restarts and reconnects.

use std::time::Duration;

/// Exponential backoff with full jitter: attempt n waits a random time in
/// `[0, min(max, base * 2^n)]`, never less than `base`.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            attempt: 0,
        }
    }

    /// Upper bound of the next delay, before jitter.
    pub fn ceiling(&self) -> Duration {
        self.base
            .saturating_mul(1u32 << self.attempt.min(20))
            .min(self.max)
    }

    pub fn next_delay(&mut self) -> Duration {
        let jittered = self.ceiling().mul_f64(fastrand::f64());

        self.attempt = self.attempt.saturating_add(1);

        jittered.max(self.base)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_millis(100), Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_to_the_cap_and_resets() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(1));
        let ceilings: Vec<_> = (0..6)
            .map(|_| {
                let ceiling = backoff.ceiling();
                let delay = backoff.next_delay();
                assert!(
                    delay >= Duration::from_millis(100)
                        && delay <= ceiling.max(Duration::from_millis(100))
                );
                ceiling.as_millis()
            })
            .collect();
        assert_eq!(ceilings, [100, 200, 400, 800, 1000, 1000]);
        backoff.reset();
        assert_eq!(backoff.ceiling(), Duration::from_millis(100));
        for _ in 0..100 {
            backoff.next_delay();
        }
        assert_eq!(
            backoff.ceiling(),
            Duration::from_secs(1),
            "no overflow after many attempts"
        );
    }

    #[test]
    fn delay_never_exceeds_the_ceiling_of_its_attempt() {
        // Enough samples that a delay drawn from the next attempt's (doubled)
        // ceiling would show up with near certainty.
        for _ in 0..1000 {
            let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(10));
            for _ in 0..5 {
                let ceiling = backoff.ceiling();
                let delay = backoff.next_delay();
                assert!(
                    delay <= ceiling,
                    "delay {delay:?} above ceiling {ceiling:?}"
                );
            }
        }
    }
}
