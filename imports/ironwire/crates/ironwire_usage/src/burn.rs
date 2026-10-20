//! Burn rate and projection.
//!
//! Ported from the usage monitor's `core/calculations.py`. Two rates live in
//! there and both are here, because they answer different questions:
//!
//! * [`block_burn_rate`] — how fast the *open window* is being spent. The
//!   number that decides whether this window survives the next hour.
//! * [`hourly_burn_rate`] — how fast the last sixty minutes went, across every
//!   window that overlapped them. Insensitive to when a window happened to
//!   open, so it does not jump the moment a new one does.
//!
//! Neither is a quota. IronWire does not know a subscription's limit and does
//! not guess one (`AGENTS.md` rule 2) — these measure the traffic IronWire
//! itself routed, which is a thing it watched happen.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::blocks::SessionBlock;

/// How fast a window is being spent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BurnRate {
    /// Tokens per minute, over the part of the window that had work in it.
    pub tokens_per_minute: f64,
    /// Cost per hour at metered rates — what this pace *would* cost on the
    /// meter, including work a subscription had already paid for.
    pub cost_per_hour: f64,
}

/// What the open window ends at if nothing changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    /// Tokens by the time the window closes.
    pub total_tokens: i64,
    /// Cost at metered rates by the time it closes.
    pub total_cost_usd: f64,
    /// Minutes left in the window.
    pub remaining_minutes: f64,
}

/// Rate of the open window, or `None` when there is nothing to divide.
///
/// Returns `None` rather than zero for a closed window, one whose provider
/// never reported usage, or one holding a single request: a fabricated `0.0`
/// tokens/min reads as "you have stopped", which is a different claim from
/// "we have not measured anything yet".
///
/// The single-request case is the monitor's one behaviour not carried over.
/// There, `duration_minutes` floors at one, so a lone 100k-token request
/// reports 100k tokens/minute and a window that looks minutes from death. One
/// request is not an interval — it is a point, and a rate needs two.
#[must_use]
pub fn block_burn_rate(block: &SessionBlock) -> Option<BurnRate> {
    if !block.is_active || block.exchanges < 2 {
        return None;
    }
    let minutes = block.active_minutes();
    let total = block.tokens.total();
    if minutes < 1.0 || total == 0 {
        return None;
    }
    Some(BurnRate {
        tokens_per_minute: total as f64 / minutes,
        cost_per_hour: (block.cost_usd / minutes) * 60.0,
    })
}

/// Project the open window forward at its current rate.
///
/// `None` once the window has closed — there is nothing left to project into.
#[must_use]
pub fn project(block: &SessionBlock, now: DateTime<Utc>) -> Option<Projection> {
    let rate = block_burn_rate(block)?;
    let remaining_minutes = block.remaining_minutes(now);
    if remaining_minutes <= 0.0 {
        return None;
    }
    let additional = rate.tokens_per_minute * remaining_minutes;
    Some(Projection {
        total_tokens: block.tokens.total() + additional as i64,
        total_cost_usd: block.cost_usd + rate.cost_per_hour * (remaining_minutes / 60.0),
        remaining_minutes,
    })
}

/// Tokens per minute over the last hour, across every window that overlapped
/// it.
///
/// A window's tokens are apportioned by how much of it fell inside the hour —
/// the monitor's approach, and the reason this does not spike to zero the
/// instant a five-hour window rolls over. `None` when nothing overlapped the
/// hour at all, which is not the same as a rate of zero.
#[must_use]
pub fn hourly_burn_rate(blocks: &[SessionBlock], now: DateTime<Utc>) -> Option<f64> {
    let hour_ago = now - Duration::hours(1);
    let mut tokens = 0.0;
    let mut overlapped = false;

    for block in blocks {
        if block.is_gap {
            continue;
        }
        let total = block.tokens.total();
        if total == 0 {
            continue;
        }
        // An open window is still running, so its work reaches up to now.
        let block_end = if block.is_active {
            now
        } else {
            block.last_activity.unwrap_or(block.end)
        };
        if block_end < hour_ago {
            continue;
        }

        let from = block.start.max(hour_ago);
        let to = block_end.min(now);
        if to <= from {
            continue;
        }
        let block_minutes = (block_end - block.start).num_seconds() as f64 / 60.0;
        if block_minutes <= 0.0 {
            continue;
        }
        let in_hour = (to - from).num_seconds() as f64 / 60.0;
        overlapped = true;
        tokens += total as f64 * (in_hour / block_minutes);
    }

    overlapped.then_some(tokens / 60.0)
}

/// Minutes until `remaining` tokens are gone at this rate.
///
/// `None` when the rate is not positive — dividing by it would produce an
/// infinity that renders as a confident "never".
#[must_use]
pub fn minutes_until(remaining_tokens: i64, rate: BurnRate) -> Option<f64> {
    (rate.tokens_per_minute > 0.0 && remaining_tokens > 0)
        .then(|| remaining_tokens as f64 / rate.tokens_per_minute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{DEFAULT_SESSION_HOURS, build};
    use crate::test_support::{at, exchange, with_tokens};

    fn session() -> Duration {
        Duration::hours(DEFAULT_SESSION_HOURS)
    }

    /// One open window: 100k tokens over ten minutes, four hours fifty left.
    fn open_window(now: DateTime<Utc>) -> Vec<SessionBlock> {
        build(
            &[
                with_tokens(
                    exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                    50_000,
                    1.0,
                ),
                with_tokens(
                    exchange("claude-sub", at("2026-08-09T13:10:00Z")),
                    50_000,
                    1.0,
                ),
            ],
            session(),
            now,
        )
    }

    #[test]
    fn the_rate_is_measured_over_the_worked_part_of_the_window() {
        let blocks = open_window(at("2026-08-09T14:00:00Z"));
        let rate = block_burn_rate(&blocks[0]).expect("an active window with usage");
        // 100k tokens in ten minutes, not in the sixty since it opened.
        assert!((rate.tokens_per_minute - 10_000.0).abs() < 1e-6);
        assert!((rate.cost_per_hour - 12.0).abs() < 1e-6);
    }

    #[test]
    fn a_closed_window_has_no_current_rate_rather_than_a_rate_of_zero() {
        // "You have stopped" and "we are not measuring this any more" are
        // different claims, and only one of them is true here.
        let blocks = open_window(at("2026-08-09T20:00:00Z"));
        assert!(!blocks[0].is_active);
        assert!(block_burn_rate(&blocks[0]).is_none());
    }

    #[test]
    fn a_window_whose_provider_reported_nothing_has_no_rate() {
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
            ],
            session(),
            at("2026-08-09T14:00:00Z"),
        );
        assert_eq!(blocks[0].tokens.total(), 0);
        assert!(block_burn_rate(&blocks[0]).is_none());
    }

    #[test]
    fn a_single_request_is_a_point_not_a_rate() {
        // With the monitor's one-minute floor this reports 100k tokens/min and
        // a window minutes from death, on the strength of one request.
        let blocks = build(
            &[with_tokens(
                exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                100_000,
                2.0,
            )],
            session(),
            at("2026-08-09T14:00:00Z"),
        );
        assert_eq!(blocks[0].exchanges, 1);
        assert!(block_burn_rate(&blocks[0]).is_none());
    }

    #[test]
    fn the_projection_carries_the_current_total_forward_to_the_close() {
        let now = at("2026-08-09T14:00:00Z");
        let blocks = open_window(now);
        let projected = project(&blocks[0], now).expect("an open window projects");
        // Four hours from 14:00 to 18:00, at 10k/min.
        assert!((projected.remaining_minutes - 240.0).abs() < 1e-6);
        assert_eq!(projected.total_tokens, 100_000 + 2_400_000);
        assert!((projected.total_cost_usd - (2.0 + 48.0)).abs() < 1e-6);
    }

    #[test]
    fn a_window_at_its_close_projects_nothing() {
        let now = at("2026-08-09T18:00:00Z");
        let blocks = open_window(now);
        assert!(project(&blocks[0], now).is_none());
    }

    #[test]
    fn the_hourly_rate_apportions_a_window_by_its_overlap_with_the_hour() {
        // The window worked 13:00–13:10 for 100k. At 13:40 the whole of it is
        // inside the last hour, so the hourly rate is the full 100k/60.
        let now = at("2026-08-09T13:40:00Z");
        let blocks = open_window(now);
        let rate = hourly_burn_rate(&blocks, now).expect("overlaps the hour");
        // The open window runs to `now`, so its 40 minutes are all in-hour.
        assert!((rate - 100_000.0 / 60.0).abs() < 1e-6, "got {rate}");
    }

    #[test]
    fn a_window_that_ended_before_the_hour_does_not_count() {
        let now = at("2026-08-09T23:00:00Z");
        let blocks = open_window(now);
        assert!(hourly_burn_rate(&blocks, now).is_none());
    }

    #[test]
    fn an_hour_with_no_traffic_is_unmeasured_rather_than_zero() {
        assert!(hourly_burn_rate(&[], at("2026-08-09T13:00:00Z")).is_none());
    }

    #[test]
    fn a_rate_of_zero_never_produces_a_time_to_depletion() {
        // Dividing by it yields infinity, which renders as a confident
        // "never" — the most misleading thing this screen could say.
        let stopped = BurnRate {
            tokens_per_minute: 0.0,
            cost_per_hour: 0.0,
        };
        assert!(minutes_until(50_000, stopped).is_none());
        let moving = BurnRate {
            tokens_per_minute: 1_000.0,
            cost_per_hour: 1.0,
        };
        assert_eq!(minutes_until(50_000, moving), Some(50.0));
        assert!(minutes_until(0, moving).is_none());
    }
}
