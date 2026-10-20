//! Observed quota. Never estimated.
//!
//! Inferring subscription headroom from token counts is hopeless — the windows
//! are rolling, opaque and model-weighted — and one confidently wrong
//! percentage costs us the user's trust in every other number we show
//! (`docs/CRITIQUE.md` §4). So [`Headroom`] has no variant for a guess:
//! either the provider told us, or we say `unknown`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// What we know about a backend's remaining capacity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Headroom {
    /// The provider reported a usage percentage. Carries the observation time
    /// so callers can show its age instead of implying it is live.
    Observed {
        /// Percentage of the window consumed, 0.0–100.0.
        used_pct: f32,
        /// When the window resets, if the provider said.
        resets_at: Option<DateTime<Utc>>,
        /// When we read this.
        observed_at: DateTime<Utc>,
    },
    /// We are inside a 429's `retry-after` window.
    Exhausted {
        /// Earliest time this backend is worth trying again.
        until: DateTime<Utc>,
    },
    /// A spend cap the *user* set has been reached.
    ///
    /// Deliberately not [`Self::Exhausted`]: the provider is willing and would
    /// serve this request. Reporting a user's own budget as a provider limit
    /// would put "resets in 4h" on a screen where the honest answer is "you
    /// asked me to stop".
    CapReached {
        /// Spent against this cap in the current window.
        spent_usd: f64,
        /// The cap itself.
        cap_usd: f64,
        /// When the window rolls over — the next local midnight.
        resets_at: DateTime<Utc>,
    },
    /// Authenticated, but the provider has told us nothing yet.
    Unknown,
}

impl Headroom {
    /// Whether the router should consider this backend right now.
    #[must_use]
    pub fn is_available(&self, now: DateTime<Utc>) -> bool {
        match self {
            Self::Exhausted { until } => now >= *until,
            Self::CapReached { resets_at, .. } => now >= *resets_at,
            // An unknown backend is available: refusing to try it is how we'd
            // end up with a router that never discovers anything.
            Self::Observed { .. } | Self::Unknown => true,
        }
    }

    /// Age of the observation, if there is one.
    #[must_use]
    pub fn age(&self, now: DateTime<Utc>) -> Option<Duration> {
        match self {
            Self::Observed { observed_at, .. } => Some(now - *observed_at),
            Self::Exhausted { .. } | Self::CapReached { .. } | Self::Unknown => None,
        }
    }

    /// Whether headroom is tight enough to justify descending the ladder.
    ///
    /// Deliberately conservative: descending costs a cache, so we only do it
    /// when the provider says we are genuinely near the wall.
    ///
    /// A reading stops counting at the reset time the provider gave with it.
    /// Nothing refreshes a reading once traffic has left the backend it
    /// describes, so a login that read 95% before its window rolled over
    /// otherwise stays "pressured" indefinitely, and sorts behind a metered key
    /// long after it reset. Past its reset the reading says nothing about
    /// pressure, which is what `Unknown` already means.
    #[must_use]
    pub fn is_pressured(&self, now: DateTime<Utc>) -> bool {
        match self {
            Self::Observed {
                resets_at: Some(resets_at),
                ..
            } if *resets_at <= now => false,
            Self::Observed { used_pct, .. } => *used_pct >= 90.0,
            Self::Exhausted { until } => now < *until,
            Self::CapReached { resets_at, .. } => now < *resets_at,
            Self::Unknown => false,
        }
    }
}

/// A backend's observed capacity state, as shown by `ironwire status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    /// Primary rate-limit window (Anthropic unified, or Codex primary).
    pub primary: Headroom,
    /// Secondary window where the provider exposes one (Codex weekly).
    pub secondary: Option<Headroom>,
    /// Metered spend since local midnight, in USD. An estimate — computed from
    /// observed token counts and a price table — and labelled as one.
    pub spend_today_usd: Option<f64>,
}

impl Default for QuotaSnapshot {
    fn default() -> Self {
        Self {
            primary: Headroom::Unknown,
            secondary: None,
            spend_today_usd: None,
        }
    }
}

impl QuotaSnapshot {
    /// Whether the router should consider this backend right now.
    #[must_use]
    pub fn is_available(&self, now: DateTime<Utc>) -> bool {
        self.primary.is_available(now)
            && self.secondary.as_ref().is_none_or(|h| h.is_available(now))
    }

    /// Whether either window is under pressure.
    #[must_use]
    pub fn is_pressured(&self, now: DateTime<Utc>) -> bool {
        self.primary.is_pressured(now)
            || self.secondary.as_ref().is_some_and(|h| h.is_pressured(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(offset_secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + offset_secs, 0).expect("valid timestamp")
    }

    #[test]
    fn exhausted_becomes_available_after_retry_after() {
        let h = Headroom::Exhausted { until: t(60) };
        assert!(!h.is_available(t(0)));
        assert!(h.is_available(t(60)));
        assert!(h.is_available(t(61)));
    }

    #[test]
    fn unknown_is_tried_rather_than_avoided() {
        // A router that skips backends it hasn't measured never measures any.
        assert!(Headroom::Unknown.is_available(t(0)));
        assert!(!Headroom::Unknown.is_pressured(t(0)));
    }

    #[test]
    fn pressure_needs_the_provider_to_say_so() {
        let mild = Headroom::Observed {
            used_pct: 71.0,
            resets_at: None,
            observed_at: t(0),
        };
        let tight = Headroom::Observed {
            used_pct: 96.0,
            resets_at: None,
            observed_at: t(0),
        };
        assert!(!mild.is_pressured(t(0)));
        assert!(tight.is_pressured(t(0)));
    }

    /// Once traffic leaves a login at 95%, nothing refreshes its reading. If
    /// that reading kept counting past the reset the provider announced with
    /// it, the login would sort behind a metered key for good.
    #[test]
    fn an_observed_reading_past_its_own_reset_time_is_no_longer_pressure() {
        let tight = Headroom::Observed {
            used_pct: 95.0,
            resets_at: Some(t(3600)),
            observed_at: t(0),
        };
        assert!(tight.is_pressured(t(3599)), "still inside the window");
        assert!(!tight.is_pressured(t(3600)), "the window reset");
        assert!(!tight.is_pressured(t(7200)));
        // Availability was never in question: an observation alone does not
        // make a backend unavailable, before or after the reset.
        assert!(tight.is_available(t(0)));
        assert!(tight.is_available(t(7200)));
        // And the reading is still there to show, with its age.
        assert_eq!(tight.age(t(7200)), Some(Duration::seconds(7200)));
    }

    #[test]
    fn secondary_window_can_block_an_otherwise_healthy_backend() {
        let snapshot = QuotaSnapshot {
            primary: Headroom::Observed {
                used_pct: 10.0,
                resets_at: None,
                observed_at: t(0),
            },
            secondary: Some(Headroom::Exhausted { until: t(3600) }),
            spend_today_usd: None,
        };
        assert!(!snapshot.is_available(t(0)));
        assert!(snapshot.is_pressured(t(0)));
    }

    #[test]
    fn observation_age_is_reportable() {
        let h = Headroom::Observed {
            used_pct: 50.0,
            resets_at: None,
            observed_at: t(0),
        };
        assert_eq!(h.age(t(40)), Some(Duration::seconds(40)));
        assert_eq!(Headroom::Unknown.age(t(40)), None);
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).expect("valid timestamp")
    }

    fn capped(resets_at: DateTime<Utc>) -> Headroom {
        Headroom::CapReached {
            spent_usd: 10.0,
            cap_usd: 10.0,
            resets_at,
        }
    }

    /// The point of expressing a cap as `Headroom`: every consumer that
    /// already asks "can I route here" gets the right answer with no new code
    /// path.
    #[test]
    fn a_capped_backend_is_not_available_until_the_window_rolls() {
        let headroom = capped(t(3600));
        assert!(!headroom.is_available(t(0)));
        assert!(headroom.is_available(t(3600)));
    }

    #[test]
    fn a_capped_backend_reads_as_pressured() {
        // So a conversation sitting on it descends rather than waiting.
        assert!(capped(t(3600)).is_pressured(t(0)));
        assert!(!capped(t(3600)).is_pressured(t(7200)));
    }

    /// A cap has no observation behind it, so it has no age — the same rule
    /// that keeps a fabricated number off the status screen.
    #[test]
    fn a_cap_reports_no_observation_age() {
        assert!(capped(t(3600)).age(t(0)).is_none());
    }
}
