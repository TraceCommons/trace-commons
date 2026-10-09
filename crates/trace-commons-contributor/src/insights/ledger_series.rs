//! Feed L: the proxy ledger's calls as counter rows, plus the two figures the
//! glance reads from them (today per tool, and the context tip). Pure: no
//! clock, no I/O and no copy; the daemon passes rows and `now` in.
//!
//! Gated on owner decision D3 (open): the daemon calls this only while
//! `insights_ledger_feed` is on.
//!
//! # Normalization per facade
//!
//! - **Anthropic** reports `input_tokens` without the cache, and cache reads
//!   and writes beside it, so input is uncached as recorded.
//! - **OpenAI** reports cached input as a subset of `input_tokens`, so
//!   uncached is input minus cache read. It reports no cache write; a nonzero
//!   one is a shape this client does not understand and the call is unknown.
//!
//! The proxy records one `cache_write_tokens`, so a known write is
//! [`CacheWrite::TotalOnly`], never a split with a zero one-hour part.
//!
//! # Every unknown is unknown
//!
//! A missing, negative or out-of-range counter, cache reads above input on
//! the OpenAI facade, or a facade this build does not know: each leaves the
//! call `usage_unknown`, never zero. Nothing here reads `cost_usd` (owner
//! decision D5, open) or keeps a session id, a body reference or a digest.

use chrono::{DateTime, Utc};
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, TurnRecord, UsageSeries, gap_between,
};

use super::analytics_constants::{
    CONTEXT_TIP_RATIO, CONTEXT_TIP_RECENT_CALLS, CONTEXT_TIP_WINDOW_SECS,
};
use super::cache_share::{CacheShare, LedgerFacade};
use crate::routing::RoutedExchange;

/// The facade a ledger row names, or `None` for one this build does not know.
#[must_use]
pub fn ledger_facade(label: &str) -> Option<LedgerFacade> {
    match label {
        "anthropic" => Some(LedgerFacade::Anthropic),
        "openai" => Some(LedgerFacade::OpenAi),
        _ => None,
    }
}

/// One call's counters as a turn, normalized for its facade. `gap_secs` and
/// `model_label_ix` are left for [`ledger_series`]; `msg_key` is always
/// `None`, because a ledger row carries no message id.
#[must_use]
pub fn ledger_turn(row: &RoutedExchange, ordinal: u32) -> TurnRecord {
    let counters = normalized_counters(row);
    TurnRecord {
        ordinal,
        at: Some(row.started_at),
        uncached_input: counters.uncached_input,
        cache_read: counters.cache_read,
        cache_write: counters.cache_write.map(CacheWrite::TotalOnly),
        output: counters.output,
        model_label_ix: None,
        gap_secs: None,
        msg_key: None,
    }
}

/// One session's calls as a series, oldest first, with each call's declared
/// model label resolved into `model_labels`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LedgerSeries {
    pub series: UsageSeries,
    /// Declared labels in first-seen order; `model_label_ix` indexes this.
    pub model_labels: Vec<String>,
}

/// The calls of one session as a series, ordered by `(started_at, id)`.
#[must_use]
pub fn ledger_series(rows: &[&RoutedExchange]) -> LedgerSeries {
    let mut ordered: Vec<&RoutedExchange> = rows.to_vec();
    ordered.sort_by_key(|row| (row.started_at, row.id));
    let mut built = LedgerSeries::default();
    let mut previous_at = None;
    for (ordinal, row) in ordered.into_iter().enumerate() {
        let Ok(ordinal) = u32::try_from(ordinal) else {
            built.series.truncated = true;
            break;
        };
        let mut turn = ledger_turn(row, ordinal);
        turn.gap_secs = if ordinal == 0 {
            None
        } else {
            gap_between(previous_at, turn.at)
        };
        previous_at = turn.at;
        turn.model_label_ix = declared_label(row).and_then(|label| {
            let ix = match built.model_labels.iter().position(|seen| *seen == label) {
                Some(ix) => ix,
                None => {
                    built.model_labels.push(label);
                    built.model_labels.len() - 1
                }
            };
            u16::try_from(ix).ok()
        });
        built.series.push_turn(turn);
    }
    built
}

/// One tool's routed calls today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerToolDay {
    /// The tool label `inference_calls` names, `unknown` included.
    pub tool: String,
    pub calls: u32,
    /// Calls whose four counters are all known.
    pub known_calls: u32,
    /// Every token of the known calls; `None` when no call is known.
    pub tokens: Option<u64>,
    /// Input read from cache over all input, over the known calls.
    pub cache_share: Option<CacheShare>,
}

/// Today's calls per tool, in fixed order: Claude Code, Codex, any other
/// label alphabetically, `unknown` last. Never sorted by value, and no two
/// tools are summed.
#[must_use]
pub fn today_by_tool(rows: &[(&str, &RoutedExchange)]) -> Vec<LedgerToolDay> {
    let mut days: Vec<LedgerToolDay> = Vec::new();
    for (tool, row) in rows {
        let ix = match days.iter().position(|day| day.tool == *tool) {
            Some(ix) => ix,
            None => {
                days.push(LedgerToolDay {
                    tool: (*tool).to_string(),
                    calls: 0,
                    known_calls: 0,
                    tokens: None,
                    cache_share: None,
                });
                days.len() - 1
            }
        };
        let day = &mut days[ix];
        day.calls = day.calls.saturating_add(1);
        let Some(usage) = ledger_turn(row, 0).known_usage() else {
            continue;
        };
        day.known_calls = day.known_calls.saturating_add(1);
        day.tokens = Some(day.tokens.unwrap_or(0).saturating_add(usage.total()));
        if let Some(share) = CacheShare::claude(
            u64::from(usage.uncached_input),
            u64::from(usage.cache_read),
            usage.cache_write.total(),
        ) {
            match day.cache_share.as_mut() {
                Some(held) => held.merge(share),
                None => day.cache_share = Some(share),
            }
        }
    }
    days.sort_by(|a, b| tool_rank(&a.tool).cmp(&tool_rank(&b.tool)));
    days
}

/// The context tip's state for the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTip {
    /// Advice is held (owner decision D2, open). Nothing else is computed.
    Held,
    /// The user has not set a threshold; the tip has no default.
    ThresholdUnset,
    /// No call with a session id in the window.
    NoSession,
    /// The current session's recent calls carry no known context.
    NoFigure,
    /// Context is below the tip ratio of the threshold.
    Quiet,
    /// Context reached the tip ratio of the threshold.
    Lit { context: u64, threshold: u32 },
}

/// The tip for the session of the newest call that carries a session id in
/// the last [`CONTEXT_TIP_WINDOW_SECS`]: the maximum known context over that
/// session's newest [`CONTEXT_TIP_RECENT_CALLS`] calls in the window, against
/// the user's own threshold. A call without a session id makes no tip and is
/// never read as zero.
#[must_use]
pub fn context_tip(
    rows: &[RoutedExchange],
    now: DateTime<Utc>,
    threshold: Option<u32>,
    advice_shown: bool,
) -> ContextTip {
    if !advice_shown {
        return ContextTip::Held;
    }
    let Some(threshold) = threshold else {
        return ContextTip::ThresholdUnset;
    };
    let since = now - chrono::Duration::seconds(CONTEXT_TIP_WINDOW_SECS);
    let mut recent: Vec<&RoutedExchange> = rows
        .iter()
        .filter(|row| row.started_at >= since && session_of(row).is_some())
        .collect();
    recent.sort_by_key(|row| std::cmp::Reverse((row.started_at, row.id)));
    let Some(current) = recent.first().and_then(|row| session_of(row)) else {
        return ContextTip::NoSession;
    };
    let context = recent
        .iter()
        .filter(|row| session_of(row) == Some(current))
        .take(CONTEXT_TIP_RECENT_CALLS)
        .filter_map(|row| ledger_turn(row, 0).context())
        .max();
    match context {
        None => ContextTip::NoFigure,
        Some(context) if CONTEXT_TIP_RATIO.reached_by(context, u64::from(threshold)) => {
            ContextTip::Lit { context, threshold }
        }
        Some(_) => ContextTip::Quiet,
    }
}

/// A ledger row's four counters after per-facade normalization; `None` for
/// each one not known.
struct Counters {
    uncached_input: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
    output: Option<u32>,
}

/// The session id a row carries; an empty one is none.
fn session_of(row: &RoutedExchange) -> Option<&str> {
    row.client_session_id
        .as_deref()
        .filter(|session| !session.is_empty())
}

/// A recorded count as a `u32`, or `None` when missing, negative or too large.
fn count(recorded: Option<i64>) -> Option<u32> {
    u32::try_from(recorded?).ok()
}

/// One call's tokens as a single figure: uncached input, cache reads, cache
/// writes and output, normalized per facade as above, so OpenAI's cached
/// input is not counted twice. `None` when any part is unknown, never a
/// partial sum. For the route tally `daemon::insights_route_tally` keeps.
#[must_use]
pub(crate) fn call_tokens(row: &RoutedExchange) -> Option<u64> {
    let counters = normalized_counters(row);
    Some(
        u64::from(counters.uncached_input?)
            + u64::from(counters.cache_read?)
            + u64::from(counters.cache_write?)
            + u64::from(counters.output?),
    )
}

fn normalized_counters(row: &RoutedExchange) -> Counters {
    let unknown = Counters {
        uncached_input: None,
        cache_read: None,
        cache_write: None,
        output: None,
    };
    let Some(facade) = ledger_facade(&row.facade) else {
        return unknown;
    };
    let input = count(row.input_tokens);
    let cache_read = count(row.cache_read_tokens);
    let cache_write = count(row.cache_write_tokens);
    let output = count(row.output_tokens);
    match facade {
        LedgerFacade::Anthropic => Counters {
            uncached_input: input,
            cache_read,
            cache_write,
            output,
        },
        LedgerFacade::OpenAi => {
            // Cached input is inside input, and this facade has no cache
            // write: anything else is a shape this build cannot read.
            let uncached = match (input, cache_read) {
                (Some(input), Some(read)) => input.checked_sub(read),
                _ => None,
            };
            if uncached.is_none() || cache_write != Some(0) {
                return Counters { output, ..unknown };
            }
            Counters {
                uncached_input: uncached,
                cache_read,
                cache_write,
                output,
            }
        }
    }
}

/// The declared model label, or `None` for the unknown bucket. The same
/// shape rule `inference_calls` applies to the label it shows.
fn declared_label(row: &RoutedExchange) -> Option<String> {
    let label = crate::daemon::inference_map::model_label(
        row.served_model
            .as_deref()
            .or(row.requested_model.as_deref()),
    );
    (label != crate::daemon::inference_map::UNKNOWN).then_some(label)
}

/// Claude Code, Codex, any other label alphabetically, then `unknown`.
fn tool_rank(tool: &str) -> (u8, &str) {
    match tool {
        crate::source::SOURCE_CLAUDE_CODE => (0, ""),
        crate::source::SOURCE_CODEX => (1, ""),
        crate::daemon::inference_map::UNKNOWN => (3, ""),
        other => (2, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn row(facade: &str, input: i64, read: i64, write: i64, output: i64) -> RoutedExchange {
        RoutedExchange {
            id: Some(1),
            started_at: at(0),
            client_session_id: Some("SESSION-A".to_string()),
            facade: facade.to_string(),
            backend: "b".to_string(),
            rung: "full".to_string(),
            attempts: 1,
            served_model: Some("model-a".to_string()),
            input_tokens: Some(input),
            cache_read_tokens: Some(read),
            cache_write_tokens: Some(write),
            output_tokens: Some(output),
            cost_usd: Some(12.5),
            status: 200,
            ..Default::default()
        }
    }

    #[test]
    fn facades_are_named_exactly() {
        assert_eq!(ledger_facade("anthropic"), Some(LedgerFacade::Anthropic));
        assert_eq!(ledger_facade("openai"), Some(LedgerFacade::OpenAi));
        assert_eq!(ledger_facade("Anthropic"), None);
        assert_eq!(ledger_facade("gemini"), None);
    }

    #[test]
    fn anthropic_input_is_uncached_as_recorded() {
        let turn = ledger_turn(&row("anthropic", 100, 900, 50, 20), 3);
        assert_eq!(turn.ordinal, 3);
        assert_eq!(turn.at, Some(at(0)));
        assert_eq!(turn.uncached_input, Some(100));
        assert_eq!(turn.cache_read, Some(900));
        assert_eq!(turn.cache_write, Some(CacheWrite::TotalOnly(50)));
        assert_eq!(turn.output, Some(20));
        assert_eq!(turn.context(), Some(1_050));
        assert_eq!(turn.msg_key, None);
        assert_eq!(turn.gap_secs, None);
    }

    #[test]
    fn openai_cached_input_is_taken_out_of_input() {
        let turn = ledger_turn(&row("openai", 1_000, 600, 0, 20), 0);
        assert_eq!(turn.uncached_input, Some(400));
        assert_eq!(turn.cache_read, Some(600));
        assert_eq!(turn.cache_write, Some(CacheWrite::TotalOnly(0)));
        assert_eq!(turn.context(), Some(1_000), "input is counted once");
    }

    #[test]
    fn a_known_write_is_never_a_split() {
        for facade in ["anthropic", "openai"] {
            let turn = ledger_turn(&row(facade, 10, 0, 0, 1), 0);
            assert!(
                matches!(turn.cache_write, Some(CacheWrite::TotalOnly(0))),
                "{facade}: {:?}",
                turn.cache_write
            );
            assert!(!turn.cache_write.unwrap().has_one_hour_write());
        }
    }

    #[test]
    fn every_doubtful_counter_is_unknown_never_zero() {
        let mut missing = row("anthropic", 1, 1, 1, 1);
        missing.cache_write_tokens = None;
        assert!(ledger_turn(&missing, 0).usage_unknown());
        assert_eq!(ledger_turn(&missing, 0).cache_write, None);

        let negative = row("anthropic", -1, 1, 1, 1);
        assert_eq!(ledger_turn(&negative, 0).uncached_input, None);

        let huge = row("anthropic", i64::from(u32::MAX) + 1, 1, 1, 1);
        assert_eq!(ledger_turn(&huge, 0).uncached_input, None);

        let reads_above_input = row("openai", 10, 11, 0, 1);
        assert!(ledger_turn(&reads_above_input, 0).usage_unknown());

        let openai_write = row("openai", 10, 5, 3, 1);
        assert!(ledger_turn(&openai_write, 0).usage_unknown());

        let unknown_facade = row("gemini", 10, 5, 0, 1);
        let turn = ledger_turn(&unknown_facade, 0);
        assert!(turn.usage_unknown());
        assert_eq!(
            (
                turn.uncached_input,
                turn.cache_read,
                turn.cache_write,
                turn.output
            ),
            (None, None, None, None)
        );
    }

    #[test]
    fn a_series_is_ordered_with_gaps_and_declared_labels() {
        let mut first = row("anthropic", 10, 0, 0, 1);
        first.id = Some(2);
        first.started_at = at(0);
        let mut second = row("anthropic", 10, 0, 0, 1);
        second.id = Some(1);
        second.started_at = at(90);
        second.served_model = Some("model-b".to_string());
        let mut third = row("anthropic", 10, 0, 0, 1);
        third.id = Some(3);
        third.started_at = at(100);
        third.served_model = None;
        third.requested_model = Some("has spaces, not a label".to_string());

        let built = ledger_series(&[&third, &first, &second]);
        assert!(built.series.validate().is_ok());
        let ordinals: Vec<u32> = built.series.turns.iter().map(|t| t.ordinal).collect();
        assert_eq!(ordinals, vec![0, 1, 2]);
        let gaps: Vec<Option<u32>> = built.series.turns.iter().map(|t| t.gap_secs).collect();
        assert_eq!(gaps, vec![None, Some(90), Some(10)]);
        assert_eq!(built.model_labels, vec!["model-a", "model-b"]);
        let labels: Vec<Option<u16>> = built
            .series
            .turns
            .iter()
            .map(|t| t.model_label_ix)
            .collect();
        assert_eq!(
            labels,
            vec![Some(0), Some(1), None],
            "a shapeless label is unknown"
        );
        assert!(built.series.tool_calls.is_empty());
        assert!(!built.series.truncated);
    }

    #[test]
    fn today_is_per_tool_in_fixed_order_and_never_summed() {
        let claude = row("anthropic", 100, 800, 100, 50);
        let codex = row("openai", 1_000, 250, 0, 10);
        let mut unknown_usage = row("anthropic", 1, 1, 1, 1);
        unknown_usage.output_tokens = None;
        let other = row("openai", 10, 0, 0, 1);
        let rows = [
            ("unknown", &other),
            ("codex", &codex),
            ("zeta-tool", &other),
            ("claude-code", &claude),
            ("claude-code", &unknown_usage),
            ("alpha-tool", &other),
        ];
        let days = today_by_tool(&rows);
        let tools: Vec<&str> = days.iter().map(|d| d.tool.as_str()).collect();
        assert_eq!(
            tools,
            vec!["claude-code", "codex", "alpha-tool", "zeta-tool", "unknown"]
        );
        let claude_day = &days[0];
        assert_eq!(claude_day.calls, 2);
        assert_eq!(claude_day.known_calls, 1);
        assert_eq!(claude_day.tokens, Some(1_050));
        let share = claude_day.cache_share.unwrap();
        assert_eq!((share.numerator, share.denominator), (800, 1_000));
        let codex_day = &days[1];
        assert_eq!(codex_day.tokens, Some(1_010));
        let share = codex_day.cache_share.unwrap();
        assert_eq!((share.numerator, share.denominator), (250, 1_000));
    }

    #[test]
    fn a_tool_with_no_known_call_has_no_figure() {
        let mut unknown_usage = row("anthropic", 1, 1, 1, 1);
        unknown_usage.input_tokens = None;
        let days = today_by_tool(&[("claude-code", &unknown_usage)]);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].calls, 1);
        assert_eq!(days[0].known_calls, 0);
        assert_eq!(days[0].tokens, None);
        assert_eq!(days[0].cache_share, None);
        assert!(today_by_tool(&[]).is_empty());
    }

    #[test]
    fn the_order_does_not_depend_on_the_figures() {
        let small = row("anthropic", 1, 0, 0, 1);
        let large = row("anthropic", 1_000_000, 0, 0, 1);
        let a = today_by_tool(&[("codex", &large), ("claude-code", &small)]);
        let b = today_by_tool(&[("codex", &small), ("claude-code", &large)]);
        let order =
            |days: &[LedgerToolDay]| days.iter().map(|d| d.tool.clone()).collect::<Vec<_>>();
        assert_eq!(order(&a), order(&b));
    }

    fn call(session: Option<&str>, secs_ago: i64, context: i64) -> RoutedExchange {
        let mut r = row("anthropic", context, 0, 0, 1);
        r.client_session_id = session.map(str::to_string);
        r.started_at = now() - Duration::seconds(secs_ago);
        r
    }

    fn now() -> DateTime<Utc> {
        at(10_000)
    }

    #[test]
    fn the_tip_is_held_while_advice_is_held() {
        let rows = [call(Some("s"), 10, 190_000)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), false),
            ContextTip::Held
        );
    }

    #[test]
    fn the_tip_has_no_default_threshold() {
        let rows = [call(Some("s"), 10, 190_000)];
        assert_eq!(
            context_tip(&rows, now(), None, true),
            ContextTip::ThresholdUnset
        );
    }

    #[test]
    fn the_tip_lights_at_nine_tenths_of_the_threshold() {
        let rows = [call(Some("s"), 10, 180_000)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Lit {
                context: 180_000,
                threshold: 200_000
            }
        );
        let rows = [call(Some("s"), 10, 179_999)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Quiet
        );
    }

    #[test]
    fn the_tip_reads_the_max_over_the_last_five_calls_in_the_window() {
        // A small side call after a large one does not drop the figure.
        let rows = [call(Some("s"), 60, 190_000), call(Some("s"), 30, 2_000)];
        assert!(matches!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Lit {
                context: 190_000,
                ..
            }
        ));
        // A large call six calls back is out of the five.
        let mut rows = vec![call(Some("s"), 70, 190_000)];
        rows.extend((1..=5).map(|i| call(Some("s"), 60 - i, 1_000)));
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Quiet
        );
        // A large call older than the window is out of it.
        let rows = [
            call(Some("s"), CONTEXT_TIP_WINDOW_SECS + 1, 190_000),
            call(Some("s"), 5, 1_000),
        ];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Quiet
        );
    }

    #[test]
    fn the_tip_follows_the_newest_session_and_never_reads_a_missing_id_as_zero() {
        let rows = [
            call(Some("old"), 100, 190_000),
            call(Some("new"), 50, 1_000),
            call(None, 5, 195_000),
        ];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::Quiet,
            "the session is the newest with an id; the id-less call adds nothing"
        );
        let rows = [call(None, 5, 195_000)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::NoSession
        );
        let rows = [call(Some(""), 5, 195_000)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::NoSession,
            "an empty id is no id"
        );
        let rows = [call(Some("s"), CONTEXT_TIP_WINDOW_SECS + 1, 195_000)];
        assert_eq!(
            context_tip(&rows, now(), Some(200_000), true),
            ContextTip::NoSession,
            "no call in ten minutes resolves the tip"
        );
    }

    #[test]
    fn the_tip_has_no_figure_when_no_recent_call_is_known() {
        let mut unknown = call(Some("s"), 5, 195_000);
        unknown.output_tokens = None;
        assert_eq!(
            context_tip(&[unknown], now(), Some(200_000), true),
            ContextTip::NoFigure
        );
    }

    #[test]
    fn nothing_reads_the_priced_cost() {
        // Two rows that differ only in `cost_usd` give the same everything.
        let a = row("anthropic", 100, 900, 50, 20);
        let mut b = a.clone();
        b.cost_usd = None;
        assert_eq!(ledger_turn(&a, 0), ledger_turn(&b, 0));
        assert_eq!(
            today_by_tool(&[("claude-code", &a)]),
            today_by_tool(&[("claude-code", &b)])
        );
    }
}
