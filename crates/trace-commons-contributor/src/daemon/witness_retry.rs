//! Per-trace retry delays and evidence-based pauses for background privacy checks.

use chrono::{DateTime, Duration, Utc};
use std::collections::VecDeque;
use uuid::Uuid;

const FAILURE_WINDOW: Duration = Duration::minutes(5);
const DISTINCT_FAILURES: usize = 3;
const FIRST_PAUSE_SECONDS: i64 = 30;
const MAX_PAUSE_SECONDS: i64 = 300;

pub(super) fn retry_wait(failures: u32, random: u64) -> Duration {
    let seconds = match failures {
        0 | 1 => 60,
        2 => 120,
        3 => 300,
        4 => 900,
        _ => 3600,
    };
    Duration::seconds(crate::retry_jitter::equal_jitter_secs(seconds, random) as i64)
}

/// No trace content, endpoint or identity is retained here. Entries already
/// own durable retry deadlines; this circuit only limits shared outage traffic.
#[derive(Default)]
pub(super) struct Circuit {
    failures: VecDeque<(Uuid, DateTime<Utc>)>,
    reopen_at: Option<DateTime<Utc>>,
    probe: Option<Uuid>,
    pause_seconds: i64,
    last_failure: Option<Uuid>,
}

impl Circuit {
    pub(super) fn can_start(&self, now: DateTime<Utc>) -> bool {
        self.probe.is_none() && self.reopen_at.is_none_or(|at| now >= at)
    }

    pub(super) fn recovering(&self) -> bool {
        self.reopen_at.is_some()
    }

    pub(super) fn avoid_probe(&self) -> Option<Uuid> {
        self.recovering().then_some(self.last_failure).flatten()
    }

    pub(super) fn started(&mut self, id: Uuid) {
        if self.recovering() {
            self.probe = Some(id);
        }
    }

    pub(super) fn failed(&mut self, id: Uuid, now: DateTime<Utc>) {
        if self.recovering() {
            // A request already in flight when the circuit opened is not
            // the recovery probe and cannot extend its pause.
            if self.probe != Some(id) {
                return;
            }
            self.pause_seconds = (self.pause_seconds * 2).min(MAX_PAUSE_SECONDS);
        } else {
            self.failures
                .retain(|(entry, at)| *entry != id && now - *at <= FAILURE_WINDOW);
            self.failures.push_back((id, now));
            if self.failures.len() < DISTINCT_FAILURES {
                return;
            }
            self.pause_seconds = FIRST_PAUSE_SECONDS;
        }
        self.last_failure = Some(id);
        self.probe = None;
        self.reopen_at = Some(now + Duration::seconds(self.pause_seconds));
    }

    /// Only the designated probe can close an open circuit. A late success
    /// from before the pause must not accidentally reopen a failing service.
    pub(super) fn succeeded(&mut self, id: Uuid) -> bool {
        if self.recovering() && self.probe != Some(id) {
            return false;
        }
        *self = Self::default();
        true
    }

    /// A cancelled, permanently refused or capacity-held probe says nothing
    /// about successful review. Release its slot without inventing a failure.
    pub(super) fn cancelled(&mut self, id: Uuid) {
        if self.probe == Some(id) {
            self.probe = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + seconds, 0).unwrap()
    }
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn witness_retry_delays_follow_the_approved_schedule_with_bounded_jitter() {
        for (failures, nominal) in [(1, 60), (2, 120), (3, 300), (4, 900), (5, 3600), (90, 3600)] {
            for random in [0, 1, 17, u64::MAX] {
                let delay = retry_wait(failures, random).num_seconds();
                assert!((nominal / 2..=nominal).contains(&delay));
            }
        }
    }

    #[test]
    fn one_trace_repeatedly_failing_never_pauses_other_traces() {
        let mut c = Circuit::default();
        for n in 0..20 {
            c.failed(id(1), at(n));
            assert!(c.can_start(at(n)));
            assert!(!c.recovering());
        }
    }

    #[test]
    fn distinct_recent_failures_open_then_allow_exactly_one_probe() {
        let mut c = Circuit::default();
        c.failed(id(1), at(0));
        c.failed(id(2), at(10));
        assert!(c.can_start(at(10)));
        c.failed(id(3), at(20));
        assert!(!c.can_start(at(49)));
        assert!(c.can_start(at(50)));
        assert!(c.recovering());
        assert_eq!(c.avoid_probe(), Some(id(3)));
        c.started(id(4));
        assert!(!c.can_start(at(500)));
        assert!(c.succeeded(id(4)));
        assert!(c.can_start(at(500)));
        assert!(!c.recovering());
    }

    #[test]
    fn old_failures_and_a_success_do_not_accumulate_into_an_outage() {
        let mut c = Circuit::default();
        c.failed(id(1), at(0));
        c.failed(id(2), at(1));
        c.failed(id(3), at(301));
        assert!(!c.recovering());
        assert!(c.succeeded(id(4)));
        c.failed(id(5), at(302));
        c.failed(id(6), at(303));
        assert!(!c.recovering());
    }

    #[test]
    fn failed_probes_extend_the_shared_pause_only_to_five_minutes() {
        let mut c = Circuit::default();
        for n in 1..=3 {
            c.failed(id(n), at(0));
        }
        let mut now = 30;
        for (n, wait) in [60, 120, 240, 300, 300].into_iter().enumerate() {
            assert!(c.can_start(at(now)));
            let probe = id(10 + n as u128);
            c.started(probe);
            c.failed(probe, at(now));
            assert!(!c.can_start(at(now + wait - 1)));
            now += wait;
        }
        assert!(c.can_start(at(now)));
    }

    #[test]
    fn late_success_cannot_clear_an_open_circuit_or_another_probe() {
        let mut c = Circuit::default();
        for n in 1..=3 {
            c.failed(id(n), at(0));
        }
        assert!(!c.succeeded(id(9)));
        assert!(!c.can_start(at(20)));
        c.started(id(4));
        assert!(!c.succeeded(id(9)));
        assert!(!c.can_start(at(60)));
        c.cancelled(id(4));
        assert!(c.can_start(at(60)));
    }
}
