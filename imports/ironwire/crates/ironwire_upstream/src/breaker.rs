//! Per-backend circuit breaking: remembering a failure past the end of the
//! request that hit it.
//!
//! The pipeline already fails over *within* a request (`docs/PROTOCOL.md` §5).
//! What it does not do on its own is remember: with no state between requests,
//! a backend that is down gets tried first again on the very next turn, and the
//! user pays for that discovery every single time — as latency, in the middle of
//! their work. Over a real outage that is one wasted round trip per turn.
//!
//! The state machine's vocabulary comes from `ironclaw_llm::circuit_breaker`
//! ([`CircuitState`], [`CircuitBreakerConfig`]) so IronWire and ironclaw report
//! backend health in the same words. The transitions are implemented here
//! because ironclaw's live on its `LlmProvider` datapath, which IronWire
//! deliberately does not use — the native lane forwards bytes, and a typed
//! provider trait cannot express "unchanged" (`docs/DESIGN.md` §7).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use ironwire_core::protocol::BackendId;

pub use ironclaw_llm::circuit_breaker::{CircuitBreakerConfig, CircuitState};

use crate::backend::UpstreamError;

/// One backend's breaker.
#[derive(Debug, Clone)]
struct Breaker {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<DateTime<Utc>>,
    half_open_successes: u32,
}

impl Default for Breaker {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            half_open_successes: 0,
        }
    }
}

/// Health of one backend, for `ironwire status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakerStatus {
    /// Which backend.
    pub backend: BackendId,
    /// Where its circuit is.
    pub state: CircuitState,
    /// Consecutive transient failures recorded.
    pub consecutive_failures: u32,
    /// When the circuit will next allow a probe, if it is open.
    pub retry_at: Option<DateTime<Utc>>,
}

/// Every backend's breaker.
#[derive(Debug)]
pub struct BreakerBoard {
    config: CircuitBreakerConfig,
    breakers: Mutex<HashMap<BackendId, Breaker>>,
}

impl Default for BreakerBoard {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

impl BreakerBoard {
    /// A board with the given thresholds.
    #[must_use]
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            breakers: Mutex::new(HashMap::new()),
        }
    }

    fn recovery_timeout(&self) -> Duration {
        Duration::from_std(self.config.recovery_timeout).unwrap_or_else(|_| Duration::seconds(30))
    }

    fn with<T>(&self, f: impl FnOnce(&mut HashMap<BackendId, Breaker>) -> T) -> T {
        let mut guard = match self.breakers.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// Whether this backend may be tried right now.
    ///
    /// An open circuit whose recovery timeout has elapsed moves to half-open
    /// *here*, on the way past — the probe is the next request, not a timer we
    /// have to run.
    pub fn allows(&self, backend: &BackendId, now: DateTime<Utc>) -> bool {
        let timeout = self.recovery_timeout();
        self.with(|breakers| {
            let breaker = breakers.entry(backend.clone()).or_default();
            match breaker.state {
                CircuitState::Closed | CircuitState::HalfOpen => true,
                CircuitState::Open => {
                    let ready = breaker
                        .opened_at
                        .is_none_or(|opened| now - opened >= timeout);
                    if ready {
                        breaker.state = CircuitState::HalfOpen;
                        breaker.half_open_successes = 0;
                        tracing::info!(%backend, "circuit half-open; letting one request probe it");
                    }
                    ready
                }
            }
        })
    }

    /// Record a request that reached its first byte.
    pub fn record_success(&self, backend: &BackendId) {
        let needed = self.config.half_open_successes_needed;
        self.with(|breakers| {
            let breaker = breakers.entry(backend.clone()).or_default();
            breaker.consecutive_failures = 0;
            if breaker.state == CircuitState::HalfOpen {
                breaker.half_open_successes += 1;
                if breaker.half_open_successes >= needed {
                    breaker.state = CircuitState::Closed;
                    breaker.opened_at = None;
                    tracing::info!(%backend, "circuit closed; backend is healthy again");
                }
            } else {
                breaker.state = CircuitState::Closed;
                breaker.opened_at = None;
            }
        });
    }

    /// Record a failed request.
    ///
    /// Only failures that actually say something about the backend's *health*
    /// count toward opening it — see
    /// [`UpstreamError::indicates_unhealthy_backend`].
    pub fn record_failure(&self, backend: &BackendId, error: &UpstreamError, now: DateTime<Utc>) {
        if !error.indicates_unhealthy_backend() {
            return;
        }
        let threshold = self.config.failure_threshold;
        self.with(|breakers| {
            let breaker = breakers.entry(backend.clone()).or_default();
            breaker.consecutive_failures += 1;
            // A probe that fails puts the circuit straight back, without
            // spending the whole threshold again.
            if breaker.state == CircuitState::HalfOpen || breaker.consecutive_failures >= threshold
            {
                if breaker.state != CircuitState::Open {
                    tracing::warn!(
                        %backend,
                        failures = breaker.consecutive_failures,
                        "circuit open; this backend will be skipped until it recovers"
                    );
                }
                breaker.state = CircuitState::Open;
                breaker.opened_at = Some(now);
                breaker.half_open_successes = 0;
            }
        });
    }

    /// Current state of every backend the board has seen.
    ///
    /// Takes no clock: `retry_at` is absolute, so there is nothing here that
    /// depends on when it is asked. Rendering it as a countdown is the caller's
    /// business.
    #[must_use]
    pub fn statuses(&self) -> Vec<BreakerStatus> {
        let timeout = self.recovery_timeout();
        self.with(|breakers| {
            let mut out: Vec<BreakerStatus> = breakers
                .iter()
                .map(|(backend, breaker)| BreakerStatus {
                    backend: backend.clone(),
                    state: breaker.state,
                    consecutive_failures: breaker.consecutive_failures,
                    retry_at: (breaker.state == CircuitState::Open)
                        .then(|| breaker.opened_at.map(|opened| opened + timeout))
                        .flatten(),
                })
                .collect();
            out.sort_by(|a, b| a.backend.as_str().cmp(b.backend.as_str()));
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> BreakerBoard {
        BreakerBoard::new(CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: std::time::Duration::from_secs(30),
            half_open_successes_needed: 2,
        })
    }

    fn id() -> BackendId {
        BackendId::from("claude-sub")
    }

    fn transient() -> UpstreamError {
        UpstreamError::Transport {
            backend: id(),
            detail: "connection reset".to_string(),
        }
    }

    #[test]
    fn a_healthy_backend_is_always_allowed() {
        let board = board();
        let now = Utc::now();
        assert!(board.allows(&id(), now));
        board.record_success(&id());
        assert!(board.allows(&id(), now));
    }

    #[test]
    fn one_blip_does_not_open_the_circuit() {
        // The whole point of the ladder is that a single failure is handled
        // inside the request. Opening on the first one would move a warm
        // conversation off its subscription over nothing.
        let board = board();
        let now = Utc::now();
        board.record_failure(&id(), &transient(), now);
        assert!(board.allows(&id(), now));
    }

    #[test]
    fn a_sustained_outage_opens_the_circuit_and_a_wait_reopens_it_for_a_probe() {
        let board = board();
        let now = Utc::now();
        for _ in 0..3 {
            board.record_failure(&id(), &transient(), now);
        }
        assert!(
            !board.allows(&id(), now),
            "a backend that failed three times running must stop being tried first"
        );

        // Still shut a moment later …
        assert!(!board.allows(&id(), now + Duration::seconds(29)));
        // … and open to exactly one probe once the timeout elapses.
        assert!(board.allows(&id(), now + Duration::seconds(31)));
    }

    #[test]
    fn a_failed_probe_reopens_without_spending_the_threshold_again() {
        let board = board();
        let now = Utc::now();
        for _ in 0..3 {
            board.record_failure(&id(), &transient(), now);
        }
        let later = now + Duration::seconds(31);
        assert!(board.allows(&id(), later), "probe allowed");
        board.record_failure(&id(), &transient(), later);
        assert!(
            !board.allows(&id(), later),
            "one failed probe must put the circuit straight back"
        );
    }

    #[test]
    fn recovery_needs_more_than_one_success() {
        // A backend that answers once and then falls over again is not healthy,
        // and closing on the first success would send the whole conversation
        // back to it.
        let board = board();
        let now = Utc::now();
        for _ in 0..3 {
            board.record_failure(&id(), &transient(), now);
        }
        let later = now + Duration::seconds(31);
        assert!(board.allows(&id(), later));

        board.record_success(&id());
        assert_eq!(board.statuses()[0].state, CircuitState::HalfOpen);
        board.record_success(&id());
        assert_eq!(board.statuses()[0].state, CircuitState::Closed);
    }

    #[test]
    fn a_missing_credential_never_opens_a_circuit() {
        // It is a configuration problem, not a health problem. Opening here
        // would replace "re-run `claude login`" with "temporarily unavailable",
        // which sends the user looking in the wrong place.
        let board = board();
        let now = Utc::now();
        for _ in 0..10 {
            board.record_failure(
                &id(),
                &UpstreamError::NeedsAuth {
                    backend: id(),
                    detail: "token expired".to_string(),
                },
                now,
            );
        }
        assert!(board.allows(&id(), now));
    }

    #[test]
    fn a_rate_limit_never_opens_a_circuit() {
        // The backend is working exactly as designed, and its own quota
        // accounting already steers routing away from it. Opening as well would
        // take it out twice for one event — and for far longer than the
        // provider asked for.
        let board = board();
        let now = Utc::now();
        for _ in 0..10 {
            board.record_failure(
                &id(),
                &UpstreamError::RateLimited {
                    backend: id(),
                    retry_after_secs: Some(60),
                },
                now,
            );
        }
        assert!(board.allows(&id(), now));
    }

    #[test]
    fn a_success_clears_the_failure_count() {
        let board = board();
        let now = Utc::now();
        board.record_failure(&id(), &transient(), now);
        board.record_failure(&id(), &transient(), now);
        board.record_success(&id());
        board.record_failure(&id(), &transient(), now);
        assert!(
            board.allows(&id(), now),
            "failures either side of a success must not accumulate"
        );
    }

    #[test]
    fn backends_are_tracked_independently() {
        let board = board();
        let now = Utc::now();
        let other = BackendId::from("nearai");
        for _ in 0..3 {
            board.record_failure(&id(), &transient(), now);
        }
        assert!(!board.allows(&id(), now));
        assert!(
            board.allows(&other, now),
            "one backend's outage must not take out another"
        );
    }

    #[test]
    fn an_open_circuit_reports_when_it_will_next_be_tried() {
        let board = board();
        let now = Utc::now();
        for _ in 0..3 {
            board.record_failure(&id(), &transient(), now);
        }
        let status = &board.statuses()[0];
        assert_eq!(status.state, CircuitState::Open);
        assert_eq!(status.consecutive_failures, 3);
        assert_eq!(status.retry_at, Some(now + Duration::seconds(30)));
    }
}
