//! What the local ledger says about how fast capacity is going.
//!
//! Ported from [`Maciek-roboblog/Claude-Code-Usage-Monitor`][monitor], which
//! solved this against real Claude Code transcripts: five-hour session
//! windows, burn rate, projection to the window's close, and a ninetieth
//! percentile taken over the user's own past windows. The algorithms are
//! theirs; the source of the data and the rules about what may be claimed are
//! IronWire's.
//!
//! # Why this does not violate "never invent a number"
//!
//! `AGENTS.md` rule 2 is about **quota**: a provider's remaining capacity is
//! reported or it is [`Unknown`], and [`Headroom`] has no variant for a guess.
//! Nothing in this crate changes that. [`Headroom`] gains no variant, no value
//! computed here reaches routing, and `ironwire status` keeps printing
//! `unknown` for any window the provider has not spoken about.
//!
//! What this crate measures is IronWire's own traffic — tokens it watched go
//! past, recorded in [`ironwire_ledger`]. "You have sent 44.2k tokens through
//! this backend in the last two hours, at 1.2k a minute" is an observation,
//! not an inference about a provider's books. Everything derived from it
//! carries a [`Basis`] saying which it is, so the screen can be explicit about
//! the difference:
//!
//! * [`Basis::Measured`] — summed from the ledger. Happened.
//! * [`Basis::Projected`] — a measured rate multiplied by the time left.
//! * [`Basis::SelfCalibrated`] — the user's own past windows ([`p90`]).
//! * [`Basis::Declared`] — a limit the user wrote in their config ([`plan`]).
//!
//! There is no fifth variant, and in particular none meaning "a limit we
//! assumed". A backend with no history yields no estimate at all rather than a
//! plausible one.
//!
//! [monitor]: https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor
//! [`Unknown`]: https://docs.rs/ironwire_core/latest/ironwire_core/quota/enum.Headroom.html
//! [`Headroom`]: https://docs.rs/ironwire_core/latest/ironwire_core/quota/enum.Headroom.html
#![warn(missing_docs)]

pub mod blocks;
pub mod burn;
pub mod p90;
pub mod plan;

use chrono::{DateTime, Duration, Utc};
use ironwire_ledger::Exchange;
use serde::{Deserialize, Serialize};

pub use blocks::{DEFAULT_SESSION_HOURS, SessionBlock, TokenCounts};
pub use burn::{BurnRate, Projection};
pub use p90::{DEFAULT_HISTORY_HOURS, P90, P90Config};
pub use plan::{Plan, PlanLimits};

/// Where a number came from. Travels with every figure this crate produces so
/// that a screen never has to decide for itself how much to believe one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Summed from the ledger. This happened.
    Measured,
    /// A measured rate carried forward. Will only hold if nothing changes.
    Projected,
    /// The user's own completed windows.
    SelfCalibrated,
    /// A limit the user declared in their config.
    Declared,
}

/// A ceiling to compare the open window against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ceiling {
    /// Tokens.
    pub tokens: i64,
    /// Where it came from — never [`Basis::Measured`], because IronWire has
    /// never measured a provider's limit.
    pub basis: Basis,
    /// How to describe it in one phrase, e.g. `your own p90 over 14 sessions`
    /// or `the Max 5× limit you declared`.
    pub description: String,
    /// Whether even the source of the figure calls it unverified.
    pub unverified: bool,
}

/// How to compute an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Options {
    /// Length of a session window.
    pub session_hours: i64,
    /// How far back to look for completed windows.
    pub history_hours: i64,
    /// The plan the user declared, if any. There is no default: an undeclared
    /// plan means the ceiling comes from their own history or not at all.
    pub plan: Option<Plan>,
    /// Percentile knobs.
    pub p90: P90Config,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            session_hours: DEFAULT_SESSION_HOURS,
            history_hours: DEFAULT_HISTORY_HOURS,
            plan: None,
            p90: P90Config::default(),
        }
    }
}

impl Options {
    /// The window length, as a [`Duration`].
    #[must_use]
    pub fn session(&self) -> Duration {
        Duration::hours(self.session_hours.max(1))
    }

    /// How far back a caller should query the ledger.
    #[must_use]
    pub fn history(&self) -> Duration {
        Duration::hours(self.history_hours.max(self.session_hours))
    }
}

/// One backend's open window, and what it is on course for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    /// Backend id.
    pub backend: String,
    /// When the window opened.
    pub started_at: DateTime<Utc>,
    /// When it closes. IronWire's own five-hour boundary, from the ledger —
    /// **not** the provider's reset time, which only the provider can state
    /// and which `ironwire status` prints separately when it does.
    pub closes_at: DateTime<Utc>,
    /// Minutes since it opened.
    pub elapsed_minutes: f64,
    /// Minutes until it closes.
    pub remaining_minutes: f64,
    /// Exchanges in it.
    pub exchanges: i64,
    /// Exchanges whose usage the provider never reported — the tokens below
    /// are missing theirs, so a window with many of these is understated.
    pub without_usage: i64,
    /// Tokens so far.
    pub tokens: TokenCounts,
    /// Cost so far, at metered rates.
    pub cost_usd: f64,
    /// Models seen, first use first.
    pub models: Vec<String>,
    /// Rate over the worked part of the window. `None` when there is nothing
    /// to divide — never a fabricated zero.
    pub burn: Option<BurnRate>,
    /// Rate over the last hour, across every window that overlapped it.
    pub hourly_tokens_per_minute: Option<f64>,
    /// Where this window ends up at the current rate.
    pub projection: Option<Projection>,
    /// What to compare it against, when anything is available.
    pub ceiling: Option<Ceiling>,
    /// Percent of `ceiling` consumed. `None` without a ceiling.
    pub used_pct: Option<f64>,
    /// Minutes until the ceiling is reached at the current rate. `None` when
    /// there is no ceiling, no rate, or nothing left of it.
    pub exhausts_in_minutes: Option<f64>,
}

impl SessionUsage {
    /// Whether the ceiling is reached before the window closes — the one
    /// question this screen exists to answer.
    #[must_use]
    pub fn exhausts_before_close(&self) -> bool {
        self.exhausts_in_minutes
            .is_some_and(|minutes| minutes < self.remaining_minutes)
    }
}

/// Everything the status screen needs about usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageReport {
    /// Open windows, one per backend with traffic, busiest first.
    pub sessions: Vec<SessionUsage>,
    /// Completed windows the percentile was taken over. Zero means there is
    /// no history yet, which is why a session may carry no ceiling.
    pub completed_sessions: usize,
    /// The user's own ninetieth percentile, when there is history for one.
    pub p90: Option<P90>,
    /// Hours of ledger this was computed from.
    pub history_hours: i64,
    /// Length of a window, in hours.
    pub session_hours: i64,
}

/// Build the report from raw ledger rows.
///
/// `exchanges` should be everything since `now - options.history()`; order
/// does not matter. An empty ledger yields an empty report rather than a
/// zeroed one — "we have not recorded anything" and "you have used nothing"
/// are different sentences and only the caller knows which is true.
#[must_use]
pub fn report(exchanges: &[Exchange], now: DateTime<Utc>, options: &Options) -> UsageReport {
    let blocks = blocks::build(exchanges, options.session(), now);
    let p90 = p90::p90_limit(&blocks, &options.p90);
    let completed_sessions = blocks.iter().filter(|b| b.is_complete()).count();
    let hourly = burn::hourly_burn_rate(&blocks, now);

    let mut sessions: Vec<SessionUsage> = blocks
        .iter()
        .filter(|block| block.is_active && block.exchanges > 0)
        .map(|block| session(block, now, p90.as_ref(), hourly, options))
        .collect();
    // Busiest first: on a machine with several backends the one being spent
    // is the one worth the top of the screen.
    sessions.sort_by(|a, b| {
        b.tokens
            .total()
            .cmp(&a.tokens.total())
            .then_with(|| a.backend.cmp(&b.backend))
    });

    UsageReport {
        sessions,
        completed_sessions,
        p90,
        history_hours: options.history_hours,
        session_hours: options.session_hours,
    }
}

fn session(
    block: &SessionBlock,
    now: DateTime<Utc>,
    p90: Option<&P90>,
    hourly: Option<f64>,
    options: &Options,
) -> SessionUsage {
    let burn = burn::block_burn_rate(block);
    let ceiling = ceiling(p90, options.plan);
    let used = block.tokens.total();
    let used_pct = ceiling
        .as_ref()
        .filter(|c| c.tokens > 0)
        .map(|c| (used as f64 / c.tokens as f64) * 100.0);
    let exhausts_in_minutes = ceiling
        .as_ref()
        .zip(burn)
        .and_then(|(c, rate)| burn::minutes_until(c.tokens - used, rate));

    SessionUsage {
        backend: block.backend.clone(),
        started_at: block.start,
        closes_at: block.end,
        elapsed_minutes: block.elapsed_minutes(now),
        remaining_minutes: block.remaining_minutes(now),
        exchanges: block.exchanges,
        without_usage: block.without_usage,
        tokens: block.tokens,
        cost_usd: block.cost_usd,
        models: block.models.clone(),
        burn,
        hourly_tokens_per_minute: hourly,
        projection: burn::project(block, now),
        ceiling,
        used_pct,
        exhausts_in_minutes,
    }
}

/// A plan the user declared wins over their history: they told us, and a
/// measured percentile of *their own* sessions is a description of how they
/// work rather than of where the provider stops them.
fn ceiling(p90: Option<&P90>, plan: Option<Plan>) -> Option<Ceiling> {
    if let Some(plan) = plan {
        let limits = plan.limits();
        return Some(Ceiling {
            tokens: limits.tokens,
            basis: Basis::Declared,
            description: format!("the {} limit you declared", limits.display_name),
            unverified: limits.unverified,
        });
    }
    let p90 = p90?;
    let sessions = p90.sessions;
    Some(Ceiling {
        tokens: p90.tokens,
        basis: Basis::SelfCalibrated,
        description: if p90.from_capped_sessions {
            format!("your own p90 over {sessions} session(s) that ran into a limit")
        } else {
            format!("your own p90 over {sessions} past session(s)")
        },
        unverified: false,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use chrono::{DateTime, Utc};
    use ironwire_ledger::Exchange;

    /// Parse an RFC 3339 instant, for readable test fixtures.
    pub(crate) fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("a valid fixture timestamp")
            .with_timezone(&Utc)
    }

    /// An exchange with no usage reported — the provider said nothing.
    pub(crate) fn exchange(backend: &str, started_at: DateTime<Utc>) -> Exchange {
        Exchange {
            id: None,
            model_alias_resolved: None,
            started_at,
            ttfb_ms: Some(400),
            total_ms: Some(9_100),
            facade: "anthropic".into(),
            path: "/v1/messages".into(),
            conversation: "c-1".into(),
            client_session_id: None,
            backend: backend.into(),
            requested_model: Some("claude-opus-4-6".into()),
            served_model: Some("claude-opus-4-6".into()),
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "preferred".into(),
            attempts: 1,
            input_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            cost_usd: None,
            substitutions: None,
            status: 200,
            error: None,
            confidence: None,
            proof: None,
        }
    }

    /// The same, with `total` tokens split the way a coding agent's turn
    /// actually splits them — nearly all of it cache reads.
    pub(crate) fn with_tokens(mut exchange: Exchange, total: i64, cost_usd: f64) -> Exchange {
        let output = (total / 20).max(1);
        exchange.output_tokens = Some(output);
        exchange.input_tokens = Some(0);
        exchange.cache_write_tokens = Some(0);
        exchange.cache_read_tokens = Some(total - output);
        exchange.cost_usd = Some(cost_usd);
        exchange
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::{at, exchange, with_tokens};

    fn ledger(rows: &[(&str, &str, i64, f64)]) -> Vec<Exchange> {
        rows.iter()
            .map(|(backend, when, tokens, cost)| {
                with_tokens(exchange(backend, at(when)), *tokens, *cost)
            })
            .collect()
    }

    #[test]
    fn an_empty_ledger_reports_nothing_rather_than_zeroes() {
        // "We have recorded nothing" is not "you have used nothing", and a
        // screen that cannot tell them apart is the one that stops being read.
        let report = report(&[], at("2026-08-09T14:00:00Z"), &Options::default());
        assert!(report.sessions.is_empty());
        assert!(report.p90.is_none());
        assert_eq!(report.completed_sessions, 0);
    }

    #[test]
    fn the_open_window_carries_its_rate_and_its_projection() {
        let now = at("2026-08-09T14:00:00Z");
        let report = report(
            &ledger(&[
                ("claude-sub", "2026-08-09T13:00:00Z", 50_000, 1.0),
                ("claude-sub", "2026-08-09T13:10:00Z", 50_000, 1.0),
            ]),
            now,
            &Options::default(),
        );
        let session = &report.sessions[0];
        assert_eq!(session.backend, "claude-sub");
        assert_eq!(session.tokens.total(), 100_000);
        assert!((session.burn.expect("a rate").tokens_per_minute - 10_000.0).abs() < 1e-6);
        assert!(session.projection.expect("a projection").total_tokens > 100_000);
        assert!((session.remaining_minutes - 240.0).abs() < 1e-6);
    }

    #[test]
    fn with_no_history_there_is_no_ceiling_and_so_no_percentage() {
        // The alternative is printing a percentage of a limit nobody stated,
        // which is the fabrication the whole status surface avoids.
        let now = at("2026-08-09T14:00:00Z");
        let report = report(
            &ledger(&[("claude-sub", "2026-08-09T13:00:00Z", 50_000, 1.0)]),
            now,
            &Options::default(),
        );
        assert!(report.sessions[0].ceiling.is_none());
        assert!(report.sessions[0].used_pct.is_none());
        assert!(report.sessions[0].exhausts_in_minutes.is_none());
    }

    #[test]
    fn past_windows_become_the_ceiling_and_it_says_so() {
        let now = at("2026-08-09T14:00:00Z");
        let report = report(
            &ledger(&[
                ("claude-sub", "2026-08-01T09:00:00Z", 100_000, 2.0),
                ("claude-sub", "2026-08-03T09:00:00Z", 100_000, 2.0),
                ("claude-sub", "2026-08-09T13:00:00Z", 50_000, 1.0),
            ]),
            now,
            &Options::default(),
        );
        let ceiling = report.sessions[0].ceiling.as_ref().expect("a ceiling");
        assert_eq!(ceiling.basis, Basis::SelfCalibrated);
        assert!(ceiling.description.contains("your own p90"));
        assert_eq!(ceiling.tokens, 100_000);
        assert!((report.sessions[0].used_pct.expect("a percentage") - 50.0).abs() < 1e-6);
    }

    #[test]
    fn a_declared_plan_is_labelled_as_the_users_claim_not_ours() {
        let now = at("2026-08-09T14:00:00Z");
        let options = Options {
            plan: Some(Plan::Max5),
            ..Options::default()
        };
        let report = report(
            &ledger(&[("claude-sub", "2026-08-09T13:00:00Z", 44_000, 1.0)]),
            now,
            &options,
        );
        let ceiling = report.sessions[0].ceiling.as_ref().expect("a ceiling");
        assert_eq!(ceiling.basis, Basis::Declared);
        assert!(ceiling.description.contains("you declared"), "{ceiling:?}");
        assert_eq!(ceiling.tokens, 88_000);
        assert!((report.sessions[0].used_pct.expect("a percentage") - 50.0).abs() < 1e-6);
    }

    #[test]
    fn a_window_on_course_to_run_out_early_says_so() {
        // 100k tokens in ten minutes against a 200k ceiling: another ten
        // minutes of this and it is gone, with hours of window left.
        let now = at("2026-08-09T13:10:00Z");
        let options = Options {
            plan: Some(Plan::Max20),
            ..Options::default()
        };
        let report = report(
            &ledger(&[
                ("claude-sub", "2026-08-09T13:00:00Z", 50_000, 1.0),
                ("claude-sub", "2026-08-09T13:10:00Z", 50_000, 1.0),
            ]),
            now,
            &options,
        );
        let session = &report.sessions[0];
        assert!(session.exhausts_before_close(), "{session:?}");
        assert!((session.exhausts_in_minutes.expect("a time") - 12.0).abs() < 1e-6);
    }

    #[test]
    fn a_window_that_will_last_does_not_raise_the_alarm() {
        let now = at("2026-08-09T14:00:00Z");
        let options = Options {
            plan: Some(Plan::Max20),
            ..Options::default()
        };
        let report = report(
            &ledger(&[
                ("claude-sub", "2026-08-09T13:00:00Z", 1_000, 0.1),
                ("claude-sub", "2026-08-09T13:30:00Z", 1_000, 0.1),
            ]),
            now,
            &options,
        );
        assert!(report.sessions[0].burn.is_some(), "a measured rate");
        assert!(!report.sessions[0].exhausts_before_close());
    }

    #[test]
    fn each_backend_gets_its_own_window_busiest_first() {
        let now = at("2026-08-09T14:00:00Z");
        let report = report(
            &ledger(&[
                ("openai-key", "2026-08-09T13:00:00Z", 10_000, 0.5),
                ("claude-sub", "2026-08-09T13:00:00Z", 90_000, 1.0),
            ]),
            now,
            &Options::default(),
        );
        assert_eq!(report.sessions.len(), 2);
        assert_eq!(report.sessions[0].backend, "claude-sub");
    }

    #[test]
    fn a_closed_window_is_history_and_not_shown_as_open() {
        let report = report(
            &ledger(&[("claude-sub", "2026-08-09T01:00:00Z", 90_000, 1.0)]),
            at("2026-08-09T14:00:00Z"),
            &Options::default(),
        );
        assert!(report.sessions.is_empty());
        assert_eq!(report.completed_sessions, 1);
    }

    #[test]
    fn a_window_whose_provider_went_quiet_says_how_much_is_missing() {
        let now = at("2026-08-09T14:00:00Z");
        let mut rows = ledger(&[("claude-sub", "2026-08-09T13:00:00Z", 50_000, 1.0)]);
        rows.push(exchange("claude-sub", at("2026-08-09T13:05:00Z")));
        let report = report(&rows, now, &Options::default());
        assert_eq!(report.sessions[0].exchanges, 2);
        assert_eq!(report.sessions[0].without_usage, 1);
    }
}
