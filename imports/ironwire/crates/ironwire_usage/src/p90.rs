//! The user's own ninetieth percentile.
//!
//! Ported from the usage monitor's `core/p90_calculator.py`. The idea it
//! contributes is the one thing in that project IronWire could not have got
//! from a provider: **a limit derived from the user's own history rather than
//! from a published table.**
//!
//! That distinction is what makes this compatible with `AGENTS.md` rule 2.
//! IronWire still does not guess a provider's quota — [`Headroom`] has no
//! variant for that and does not gain one here. What this computes is a
//! statement about windows that already happened on this machine: *"in nine
//! out of ten of your past sessions you used no more than this many tokens."*
//! Every caller labels it as such. Nothing here feeds routing.
//!
//! [`Headroom`]: https://docs.rs/ironwire_core/latest/ironwire_core/quota/enum.Headroom.html

use serde::{Deserialize, Serialize};

use crate::blocks::SessionBlock;

/// Token totals a session is likely to be capped at, borrowed from the
/// monitor's `COMMON_TOKEN_LIMITS`.
///
/// These are *not* used as limits. They are used to recognise a window that
/// looks like it ran into one, so the percentile is taken over sessions that
/// were actually cut short rather than over the many short ones where the user
/// simply stopped — which would drag the estimate far below anything real.
pub const COMMON_TOKEN_LIMITS: [i64; 4] = [19_000, 88_000, 220_000, 880_000];

/// How close to a common limit a window has to land to count as having hit it.
pub const LIMIT_DETECTION_THRESHOLD: f64 = 0.95;

/// How far back to look. Eight days, as the monitor uses: long enough for a
/// working week's worth of windows, short enough that a change in how someone
/// works shows up within one.
pub const DEFAULT_HISTORY_HOURS: i64 = 192;

/// Knobs for [`p90_limit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct P90Config {
    /// Totals that look like a cap.
    pub common_limits: Vec<i64>,
    /// Fraction of a common limit that counts as having hit it.
    pub limit_threshold: f64,
    /// Never report less than this. Below it the figure is noise, not a
    /// pattern.
    pub minimum: i64,
}

impl Default for P90Config {
    fn default() -> Self {
        Self {
            common_limits: COMMON_TOKEN_LIMITS.to_vec(),
            limit_threshold: LIMIT_DETECTION_THRESHOLD,
            // The monitor's `DEFAULT_TOKEN_LIMIT`, which is its Pro plan
            // figure. Used only as a floor: it stops a first day of light use
            // from producing a "limit" of nine thousand tokens.
            minimum: 19_000,
        }
    }
}

/// What a percentile was computed over — the caller has to be able to say how
/// much history is behind it, because two sessions and forty are not the same
/// claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct P90 {
    /// The ninetieth percentile of tokens per completed window.
    pub tokens: i64,
    /// Completed windows it was taken over.
    pub sessions: usize,
    /// Whether those windows were ones that look like they hit a cap. When
    /// false, this is a percentile of ordinary sessions and describes how the
    /// user works, not where the provider stops them.
    pub from_capped_sessions: bool,
}

/// The ninetieth percentile of tokens across completed windows.
///
/// Prefers windows that look like they ran into a cap; falls back to every
/// completed window when none did. `None` when there is no completed history
/// at all — an empty screen is honest, and a floor printed as though it were
/// measured is not.
#[must_use]
pub fn p90_limit(blocks: &[SessionBlock], config: &P90Config) -> Option<P90> {
    let completed: Vec<&SessionBlock> = blocks.iter().filter(|b| b.is_complete()).collect();
    if completed.is_empty() {
        return None;
    }

    let capped: Vec<i64> = completed
        .iter()
        .map(|b| b.tokens.total())
        .filter(|tokens| hit_a_limit(*tokens, &config.common_limits, config.limit_threshold))
        .collect();
    let from_capped_sessions = !capped.is_empty();
    let mut samples = if from_capped_sessions {
        capped
    } else {
        completed.iter().map(|b| b.tokens.total()).collect()
    };
    samples.sort_unstable();

    Some(P90 {
        tokens: percentile(&samples, 9, 10).max(config.minimum),
        sessions: samples.len(),
        from_capped_sessions,
    })
}

fn hit_a_limit(tokens: i64, common_limits: &[i64], threshold: f64) -> bool {
    common_limits
        .iter()
        .any(|limit| tokens as f64 >= *limit as f64 * threshold)
}

/// The `i`-th of `n` quantile cut points, by the exclusive method.
///
/// Deliberately the same estimator as Python's `statistics.quantiles`, which
/// is what the monitor uses — an inclusive percentile over the same data gives
/// a visibly different figure, and matching it means the two tools can be
/// compared on the same machine.
///
/// One difference: `statistics.quantiles` raises on fewer than two samples.
/// Here a single completed window returns itself. Refusing to say anything
/// until the second one would leave the screen blank on a user's first day,
/// and the sample count travels with the figure so a caller can say how thin
/// it is.
///
/// Note that the exclusive method extrapolates: the p90 of two windows at 87k
/// and 88k is 88.7k, above either. That is what a p90 *is* on a small sample,
/// it is what the monitor reports, and it is why the sample count is part of
/// [`P90`] rather than a detail the caller can drop.
fn percentile(sorted: &[i64], i: usize, n: usize) -> i64 {
    match sorted {
        [] => 0,
        [only] => *only,
        _ => {
            let count = sorted.len();
            let m = count + 1;
            let j = (i * m / n).clamp(1, count - 1);
            let delta = (i * m) as f64 - (j * n) as f64;
            let low = sorted[j - 1] as f64;
            let high = sorted[j] as f64;
            ((low * (n as f64 - delta) + high * delta) / n as f64) as i64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{DEFAULT_SESSION_HOURS, build};
    use crate::test_support::{at, exchange, with_tokens};
    use chrono::Duration;

    /// `n` completed windows, one per day, each with `tokens` in it.
    fn history(totals: &[i64]) -> Vec<SessionBlock> {
        let exchanges: Vec<_> = totals
            .iter()
            .enumerate()
            .map(|(day, tokens)| {
                let day = u32::try_from(day).expect("small") + 1;
                with_tokens(
                    exchange("claude-sub", at(&format!("2026-08-{day:02}T13:00:00Z"))),
                    *tokens,
                    0.0,
                )
            })
            .collect();
        build(
            &exchanges,
            Duration::hours(DEFAULT_SESSION_HOURS),
            at("2026-09-01T00:00:00Z"),
        )
    }

    #[test]
    fn it_matches_pythons_exclusive_quantiles() {
        // `statistics.quantiles([100,200,...,1000], n=10)[8]` is 990.0. Being
        // bit-comparable with the monitor is the point of porting the
        // estimator rather than reaching for a textbook percentile.
        let samples: Vec<i64> = (1..=10).map(|i| i * 100).collect();
        assert_eq!(percentile(&samples, 9, 10), 990);
        // And it extrapolates past the largest sample on a short one:
        // `statistics.quantiles([10,20,30,40], n=10)[8]` is 45.0.
        assert_eq!(percentile(&[10, 20, 30, 40], 9, 10), 45);
        assert_eq!(percentile(&[87_000, 88_000], 9, 10), 88_700);
    }

    #[test]
    fn a_single_window_reports_itself_rather_than_refusing() {
        // Python's `quantiles` raises below two samples; a blank screen on
        // someone's first day is worse than a figure carrying `sessions: 1`.
        let p90 = p90_limit(&history(&[120_000]), &P90Config::default()).expect("one window");
        assert_eq!(p90.sessions, 1);
        assert_eq!(p90.tokens, 120_000);
    }

    #[test]
    fn windows_that_ran_into_a_cap_are_preferred_over_short_ones() {
        // Averaging in the many sessions where the user simply stopped drags
        // the estimate far below anything they ever actually hit.
        let p90 = p90_limit(
            &history(&[1_000, 2_000, 3_000, 87_000, 88_000]),
            &P90Config::default(),
        )
        .expect("history");
        assert!(p90.from_capped_sessions);
        assert_eq!(p90.sessions, 2, "only the two near 88k count");
        assert!(p90.tokens >= 87_000, "got {}", p90.tokens);
    }

    #[test]
    fn with_nothing_near_a_cap_it_describes_how_the_user_works() {
        // Under 95% of the smallest common limit, so none of these looks like
        // a session that was cut short.
        let p90 =
            p90_limit(&history(&[5_000, 10_000, 15_000]), &P90Config::default()).expect("history");
        assert!(!p90.from_capped_sessions);
        assert_eq!(p90.sessions, 3);
    }

    #[test]
    fn no_completed_history_reports_nothing_at_all() {
        // A floor printed as though it were measured is exactly the invented
        // number this project refuses to print.
        assert!(p90_limit(&[], &P90Config::default()).is_none());
        let only_open = build(
            &[with_tokens(
                exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                50_000,
                0.0,
            )],
            Duration::hours(DEFAULT_SESSION_HOURS),
            at("2026-08-09T14:00:00Z"),
        );
        assert!(p90_limit(&only_open, &P90Config::default()).is_none());
    }

    #[test]
    fn a_light_first_week_does_not_produce_a_tiny_limit() {
        let p90 = p90_limit(&history(&[500, 900, 1_200]), &P90Config::default()).expect("history");
        assert_eq!(p90.tokens, P90Config::default().minimum);
    }

    #[test]
    fn gaps_and_the_open_window_are_never_sampled() {
        let blocks = build(
            &[
                with_tokens(
                    exchange("claude-sub", at("2026-08-01T13:00:00Z")),
                    100_000,
                    0.0,
                ),
                with_tokens(
                    exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                    999_999,
                    0.0,
                ),
            ],
            Duration::hours(DEFAULT_SESSION_HOURS),
            at("2026-08-09T14:00:00Z"),
        );
        assert!(blocks.iter().any(|b| b.is_gap));
        let p90 = p90_limit(&blocks, &P90Config::default()).expect("history");
        assert_eq!(p90.sessions, 1, "the gap and the open window are excluded");
        assert_eq!(p90.tokens, 100_000);
    }
}
