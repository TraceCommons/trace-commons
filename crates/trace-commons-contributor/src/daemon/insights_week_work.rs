//! `insights_week`'s `work`: Claude Code and Codex spend at API list
//! prices, lines and edits, for the week on screen and the week before
//! (product owner, 2026-10-29: Claude and Codex spend shown at API list
//! prices). NEAR AI money stays in `near_ai_credit`.
//!
//! Spend is priced turn by turn from the stored per-turn series
//! (`session_pricing::price_turn_exact`), when the answer is built; nothing
//! is stored. Each turn is placed on its own local day, so a session that
//! runs past midnight or into another week is split by its turns. Claude
//! turns are the ones the week's rollup counts (`counted_claude_turns`), so
//! a turn a resumed transcript repeats is priced once. A Codex session with
//! no per-turn series (read before the per-turn reader, or trimmed by the
//! Codex budget) has no priced turn: it is unpriced, never $0.
//!
//! Lines and edits are the session's facts (`session_facts`), counted once,
//! in the week holding the session's first recorded activity. A resumed
//! Claude transcript repeats the one it continues, facts and all, so it
//! counts only what it holds beyond that one, in the week of its first new
//! turn (`claude_resumes`); one whose overlap cannot be untangled counts as
//! unknown, never twice.
//!
//! A session sent through NEAR AI is charged by NEAR AI, in
//! `near_ai_credit`; its list estimate is never added to the spend here
//! (owner decision D5: a session's charge and its list estimate never meet
//! in one figure). One sent wholly through the route is left out of the
//! spend and counted in `near_ai_sessions`; one sent partly through it has
//! a list-priced part that cannot be told apart, so it is unpriced.
//!
//! Every figure is a sum of what is known (`*_known`) beside a `*_partial`
//! flag: an unknown is never read as zero. Automated sessions are counted,
//! as the weekly token figures count them: they cost tokens.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use trace_commons_protocol::insights_pricing::PricingAccounting;
use trace_commons_protocol::insights_usage_series::TurnRecord;

use crate::insights::pricing_catalog::PricingCatalog;
use crate::insights::session_facts::SessionFacts;
use crate::insights::session_pricing::{
    PRICING_BASIS_API_LIST, PricingTableRef, TurnPrice, exact_to_micros, price_turn_exact,
};
use crate::insights::week_rollup::{
    AnalyticsSource, ClaudeResume, Feed, SessionBody, SessionInput, claude_resumes,
    counted_claude_turns, local_week_start,
};

const TOOL_CLAUDE_CODE: &str = "claude_code";
const TOOL_CODEX: &str = "codex";

/// One scope's figures: the week, a tool, or a folder.
#[derive(Debug, Default, Clone)]
struct Totals {
    sessions: u64,
    priced_sessions: u64,
    near_ai_sessions: u64,
    exact: u128,
    cache_savings_exact: u128,
    lines_added: u64,
    lines_removed: u64,
    lines_partial: bool,
    edits: u64,
    edits_undone: u64,
    edits_denied: u64,
    edits_count_partial: bool,
    edits_undone_partial: bool,
    edits_denied_partial: bool,
}

/// Spend on one model.
#[derive(Debug, Default)]
struct ModelSpend {
    sessions: u64,
    tokens: u64,
    exact: u128,
    partial: bool,
}

/// Spend on one local day.
#[derive(Debug, Default, Clone, Copy)]
struct DaySpend {
    exact: u128,
    partial: bool,
}

/// A count that may be only partly known: the known part, and whether
/// some of it is not.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Count {
    known: u64,
    partial: bool,
}

impl Count {
    fn of(count: Option<u32>) -> Self {
        Count {
            known: count.map_or(0, u64::from),
            partial: count.is_none(),
        }
    }

    fn unknown() -> Self {
        Count {
            known: 0,
            partial: true,
        }
    }

    fn add(&mut self, other: Count) {
        self.known += other.known;
        self.partial |= other.partial;
    }

    /// What `self` holds beyond `earlier`, which it repeats. When the
    /// repeated part is only partly known, so is what is left: known 0.
    fn beyond(self, earlier: Count) -> Self {
        if earlier.partial {
            return Count::unknown();
        }
        Count {
            known: self.known.saturating_sub(earlier.known),
            partial: self.partial,
        }
    }
}

/// A session's lines and edits, summed over its models.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct FactCounts {
    lines_added: Count,
    lines_removed: Count,
    edits: Count,
    edits_undone: Count,
    edits_denied: Count,
}

impl FactCounts {
    fn of(facts: Option<&SessionFacts>) -> Self {
        let Some(facts) = facts else {
            return FactCounts::unknown();
        };
        // Lines are known only as a pair.
        let lines_partial =
            facts.shape.lines_added.is_none() || facts.shape.lines_removed.is_none();
        let mut counts = FactCounts {
            lines_added: Count {
                partial: lines_partial,
                ..Count::of(facts.shape.lines_added)
            },
            lines_removed: Count {
                partial: lines_partial,
                ..Count::of(facts.shape.lines_removed)
            },
            ..FactCounts::default()
        };
        for model in &facts.models {
            counts.edits.add(Count::of(model.edits));
            counts.edits_undone.add(Count::of(model.undone));
            counts.edits_denied.add(Count::of(model.denied));
        }
        counts
    }

    fn unknown() -> Self {
        FactCounts {
            lines_added: Count::unknown(),
            lines_removed: Count::unknown(),
            edits: Count::unknown(),
            edits_undone: Count::unknown(),
            edits_denied: Count::unknown(),
        }
    }

    fn beyond(self, earlier: FactCounts) -> Self {
        FactCounts {
            lines_added: self.lines_added.beyond(earlier.lines_added),
            lines_removed: self.lines_removed.beyond(earlier.lines_removed),
            edits: self.edits.beyond(earlier.edits),
            edits_undone: self.edits_undone.beyond(earlier.edits_undone),
            edits_denied: self.edits_denied.beyond(earlier.edits_denied),
        }
    }
}

/// The lines and edits a session adds, net of the transcript it resumes
/// (`claude_resumes`), so work a resumed transcript repeats counts once.
fn own_counts(inputs: &[SessionInput], resumes: &[ClaudeResume], index: usize) -> FactCounts {
    let own = FactCounts::of(inputs[index].facts.as_ref());
    let resume = resumes.get(index).copied().unwrap_or_default();
    if resume.tangled {
        return FactCounts::unknown();
    }
    match resume.continues {
        Some(earlier) => own.beyond(FactCounts::of(inputs[earlier].facts.as_ref())),
        None => own,
    }
}

/// When a session's lines and edits are dated: its first activity, or for
/// a resumed transcript its first dated turn the resumed one lacks.
fn facts_at(
    inputs: &[SessionInput],
    resumes: &[ClaudeResume],
    index: usize,
) -> Option<DateTime<Utc>> {
    let input = &inputs[index];
    let earlier = resumes
        .get(index)
        .and_then(|resume| resume.continues)
        .map(|earlier| &inputs[earlier].body);
    if let (
        SessionBody::Claude { series, .. },
        Some(SessionBody::Claude {
            series: earlier, ..
        }),
    ) = (&input.body, earlier)
    {
        let repeated: BTreeSet<_> = earlier
            .turns
            .iter()
            .filter_map(|turn| turn.msg_key)
            .collect();
        if let Some(at) = series
            .turns
            .iter()
            .filter(|turn| turn.msg_key.is_none_or(|key| !repeated.contains(&key)))
            .filter_map(|turn| turn.at)
            .min()
        {
            return Some(at);
        }
    }
    first_activity(input)
}

/// One session's figures in one week.
#[derive(Debug, Default)]
struct SessionWork {
    priced: bool,
    /// Sent wholly through NEAR AI: its spend is NEAR AI's charge.
    near_ai: bool,
    exact: u128,
    cache_savings_exact: u128,
    /// The session's own lines and edits, when they are dated in the week.
    facts: Option<FactCounts>,
}

impl Totals {
    fn add(&mut self, session: &SessionWork) {
        self.sessions += 1;
        if session.near_ai {
            self.near_ai_sessions += 1;
        } else if session.priced {
            self.priced_sessions += 1;
        }
        self.exact = self.exact.saturating_add(session.exact);
        self.cache_savings_exact = self
            .cache_savings_exact
            .saturating_add(session.cache_savings_exact);
        let Some(facts) = session.facts else {
            return;
        };
        self.lines_added += facts.lines_added.known;
        self.lines_removed += facts.lines_removed.known;
        self.lines_partial |= facts.lines_added.partial || facts.lines_removed.partial;
        self.edits += facts.edits.known;
        self.edits_count_partial |= facts.edits.partial;
        self.edits_undone += facts.edits_undone.known;
        self.edits_undone_partial |= facts.edits_undone.partial;
        self.edits_denied += facts.edits_denied.known;
        self.edits_denied_partial |= facts.edits_denied.partial;
    }

    fn value(&self) -> serde_json::Value {
        let unpriced = self.sessions - self.priced_sessions - self.near_ai_sessions;
        serde_json::json!({
            "sessions": self.sessions,
            "priced_sessions": self.priced_sessions,
            "near_ai_sessions": self.near_ai_sessions,
            "unpriced_sessions": unpriced,
            "usd_micros_known": micros(self.exact),
            "usd_partial": unpriced > 0,
            "cache_savings_usd_micros_known": micros(self.cache_savings_exact),
            "lines_added_known": self.lines_added,
            "lines_removed_known": self.lines_removed,
            "lines_partial": self.lines_partial,
            "edits_known": self.edits,
            "edits_undone_known": self.edits_undone,
            "edits_denied_known": self.edits_denied,
            // Partial when any of the three counts is unknown; the three
            // flags after it say which.
            "edits_partial": self.edits_count_partial
                || self.edits_undone_partial
                || self.edits_denied_partial,
            "edits_count_partial": self.edits_count_partial,
            "edits_undone_partial": self.edits_undone_partial,
            "edits_denied_partial": self.edits_denied_partial,
        })
    }
}

/// An exact numerator as USD micros; past `u64` (no real spend), the
/// largest figure rather than a wrapped one.
fn micros(exact: u128) -> u64 {
    exact_to_micros(exact).unwrap_or(u64::MAX)
}

/// One week's figures.
#[derive(Debug)]
struct WeekWork {
    totals: Totals,
    by_tool: BTreeMap<&'static str, Totals>,
    /// By folder label; `None` is the unknown folder.
    by_folder: BTreeMap<Option<String>, Totals>,
    by_model: BTreeMap<(String, &'static str), ModelSpend>,
    by_day: [DaySpend; 7],
}

impl WeekWork {
    fn by_day_value(&self, week_start: NaiveDate) -> serde_json::Value {
        let days: Vec<serde_json::Value> = self
            .by_day
            .iter()
            .enumerate()
            .map(|(offset, day)| {
                serde_json::json!({
                    "date": (week_start + Duration::days(offset as i64)).to_string(),
                    "usd_micros_known": micros(day.exact),
                    "usd_partial": day.partial,
                })
            })
            .collect();
        serde_json::Value::Array(days)
    }
}

fn tool_of(input: &SessionInput) -> Option<&'static str> {
    let source = match &input.body {
        SessionBody::Claude { .. } => Some(AnalyticsSource::ClaudeCode),
        SessionBody::Codex { .. } => Some(AnalyticsSource::Codex),
        SessionBody::Unknown { source, .. } => *source,
    };
    source.map(|source| match source {
        AnalyticsSource::ClaudeCode => TOOL_CLAUDE_CODE,
        AnalyticsSource::Codex => TOOL_CODEX,
    })
}

/// The session's first recorded activity: its earliest dated turn, else
/// Codex's first observed counter, else its placement.
fn first_activity(input: &SessionInput) -> Option<DateTime<Utc>> {
    let turns = match &input.body {
        SessionBody::Claude { series, .. } => Some(&series.turns),
        SessionBody::Codex {
            series: Some(evidence),
            ..
        } => Some(&evidence.series.turns),
        _ => None,
    };
    let observed = match &input.body {
        SessionBody::Codex {
            observed: Some(observed),
            ..
        } => Some(observed.first_at),
        _ => None,
    };
    turns
        .and_then(|turns| turns.iter().filter_map(|turn| turn.at).min())
        .or(observed)
        .or(input.placed_at)
}

/// What one session holds for a week: the turns priced in it, whether some
/// of its usage could not be priced at all, and the day that is marked.
struct InWeek<'a> {
    accounting: PricingAccounting,
    labels: &'a [String],
    turns: Vec<&'a TurnRecord>,
    /// Usage with no priced figure possible: no series, or undated turns.
    unpriceable: bool,
}

/// The session's share of the week starting `week_start`, or `None` when
/// it has none. `counted` is `counted_claude_turns`' entry for it.
fn in_week<'a>(
    input: &'a SessionInput,
    counted: Option<&BTreeSet<u32>>,
    week_start: NaiveDate,
    tz: &FixedOffset,
) -> Option<InWeek<'a>> {
    let this_week = |at: &DateTime<Utc>| local_week_start(at, tz) == week_start;
    let anchored = first_activity(input).is_some_and(|at| this_week(&at));
    match &input.body {
        SessionBody::Claude {
            series,
            model_labels,
        } => {
            let any_undated = series.turns.iter().any(|turn| turn.at.is_none());
            if let Some(counted) = counted {
                Some(InWeek {
                    accounting: PricingAccounting::ClaudeCode,
                    labels: model_labels,
                    turns: series
                        .turns
                        .iter()
                        .filter(|turn| counted.contains(&turn.ordinal))
                        .collect(),
                    unpriceable: any_undated,
                })
            } else if series.turns.iter().all(|turn| turn.at.is_none()) && anchored {
                Some(InWeek {
                    accounting: PricingAccounting::ClaudeCode,
                    labels: model_labels,
                    turns: Vec::new(),
                    unpriceable: true,
                })
            } else {
                // No turn counted this week: none dated in it, or every one
                // already counted in another transcript.
                None
            }
        }
        SessionBody::Codex {
            series: Some(evidence),
            ..
        } if evidence.series.turns.iter().any(|turn| turn.at.is_some()) => {
            let turns: Vec<&TurnRecord> = evidence
                .series
                .turns
                .iter()
                .filter(|turn| turn.at.as_ref().is_some_and(this_week))
                .collect();
            if turns.is_empty() {
                return None;
            }
            Some(InWeek {
                accounting: PricingAccounting::Codex,
                labels: &evidence.model_labels,
                turns,
                unpriceable: evidence.series.turns.iter().any(|turn| turn.at.is_none()),
            })
        }
        // No priced series: unpriced in every week its observed interval
        // reaches, so no such week reads as exact.
        SessionBody::Codex {
            observed: Some(observed),
            ..
        } => (local_week_start(&observed.first_at, tz) <= week_start
            && week_start <= local_week_start(&observed.last_at, tz))
        .then_some(InWeek {
            accounting: PricingAccounting::Codex,
            labels: &[],
            turns: Vec::new(),
            unpriceable: true,
        }),
        SessionBody::Codex { .. } | SessionBody::Unknown { .. } => anchored.then_some(InWeek {
            accounting: PricingAccounting::Codex,
            labels: &[],
            turns: Vec::new(),
            unpriceable: true,
        }),
    }
}

/// How a session went through NEAR AI, by its route tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NearAiRoute {
    /// Every labelled call through the route.
    Wholly,
    /// Labelled calls both through the route and outside it.
    Partly,
}

/// Roll up the week starting `week_start`.
fn week_work(
    inputs: &[SessionInput],
    folders: &BTreeMap<String, Option<String>>,
    catalog: &PricingCatalog,
    week_start: NaiveDate,
    tz: &FixedOffset,
    (resumes, routed): (&[ClaudeResume], &BTreeMap<String, NearAiRoute>),
    tables: &mut BTreeSet<PricingTableRef>,
) -> WeekWork {
    let counted = counted_claude_turns(Feed::CounterPass, inputs, week_start, tz);
    let mut week = WeekWork {
        totals: Totals::default(),
        by_tool: BTreeMap::new(),
        by_folder: BTreeMap::new(),
        by_model: BTreeMap::new(),
        by_day: [DaySpend::default(); 7],
    };
    let day_of = |at: &DateTime<Utc>| {
        let offset = (at.with_timezone(tz).date_naive() - week_start).num_days();
        usize::try_from(offset).ok().filter(|offset| *offset < 7)
    };
    for (index, input) in inputs.iter().enumerate() {
        let Some(tool) = tool_of(input) else {
            continue;
        };
        let Some(mut scope) = in_week(
            input,
            counted.get(index).and_then(Option::as_ref),
            week_start,
            tz,
        ) else {
            continue;
        };
        let route = routed.get(&input.session_ref).copied();
        if route.is_some() {
            // NEAR AI's charge, not a list estimate (D5).
            scope.turns.clear();
            scope.unpriceable = route == Some(NearAiRoute::Partly);
        }
        let mut session = SessionWork {
            priced: !scope.unpriceable && !scope.turns.is_empty(),
            near_ai: route == Some(NearAiRoute::Wholly),
            ..SessionWork::default()
        };
        let mut models: BTreeSet<String> = BTreeSet::new();
        for turn in &scope.turns {
            let day = turn.at.as_ref().and_then(day_of);
            let tokens = turn.known_usage().map(|usage| usage.total());
            match price_turn_exact(catalog, scope.accounting, scope.labels, turn) {
                TurnPrice::Skipped => {}
                TurnPrice::Priced {
                    model,
                    exact,
                    cache_savings_exact,
                    table,
                } => {
                    tables.insert(table);
                    session.exact = session.exact.saturating_add(exact);
                    session.cache_savings_exact = session
                        .cache_savings_exact
                        .saturating_add(cache_savings_exact.unwrap_or(0));
                    if let Some(day) = day {
                        week.by_day[day].exact = week.by_day[day].exact.saturating_add(exact);
                    }
                    let spend = week.by_model.entry((model.clone(), tool)).or_default();
                    spend.exact = spend.exact.saturating_add(exact);
                    spend.tokens += tokens.unwrap_or(0);
                    models.insert(model);
                }
                TurnPrice::Unpriced { model, .. } => {
                    session.priced = false;
                    if let Some(day) = day {
                        week.by_day[day].partial = true;
                    }
                    if let Some(model) = model {
                        let spend = week.by_model.entry((model.clone(), tool)).or_default();
                        spend.partial = true;
                        spend.tokens += tokens.unwrap_or(0);
                        models.insert(model);
                    }
                }
            }
        }
        for model in models {
            if let Some(spend) = week.by_model.get_mut(&(model, tool)) {
                spend.sessions += 1;
            }
        }
        if !session.priced && !session.near_ai {
            // The day the unpriced usage is marked on: its first dated turn
            // in the week, else its first activity, else the week's first
            // day.
            let day = scope
                .turns
                .iter()
                .filter_map(|turn| turn.at.as_ref().and_then(day_of))
                .min()
                .or_else(|| first_activity(input).as_ref().and_then(day_of))
                .unwrap_or(0);
            week.by_day[day].partial = true;
        }
        if facts_at(inputs, resumes, index)
            .is_some_and(|at| local_week_start(&at, tz) == week_start)
        {
            session.facts = Some(own_counts(inputs, resumes, index));
        }
        let folder = folders.get(&input.session_ref).cloned().flatten();
        week.totals.add(&session);
        week.by_tool.entry(tool).or_default().add(&session);
        week.by_folder.entry(folder).or_default().add(&session);
    }
    week
}

/// `insights_week`'s `work`, for the week starting `week_start`. `folders`
/// is each session's folder label by its engine reference (`None`: the
/// folder is unknown). `oldest_kept` is the first day of the oldest week
/// the store keeps: a week before it has no figures, so `last_week` is
/// `null` rather than zero. `routed` is each session sent through NEAR AI,
/// by its engine reference.
pub(crate) fn work_value(
    inputs: &[SessionInput],
    folders: &BTreeMap<String, Option<String>>,
    catalog: &PricingCatalog,
    week_start: NaiveDate,
    tz: FixedOffset,
    oldest_kept: NaiveDate,
    routed: &BTreeMap<String, NearAiRoute>,
) -> serde_json::Value {
    let mut tables = BTreeSet::new();
    let resumes = claude_resumes(inputs);
    let this = week_work(
        inputs,
        folders,
        catalog,
        week_start,
        &tz,
        (&resumes, routed),
        &mut tables,
    );
    let last_start = week_start - Duration::days(7);
    let last = (last_start >= oldest_kept).then(|| {
        week_work(
            inputs,
            folders,
            catalog,
            last_start,
            &tz,
            (&resumes, routed),
            &mut tables,
        )
    });
    let by_tool: Vec<serde_json::Value> = [TOOL_CLAUDE_CODE, TOOL_CODEX]
        .iter()
        .filter_map(|tool| {
            let mut value = this.by_tool.get(tool)?.value();
            value["key"] = serde_json::json!(tool);
            Some(value)
        })
        .collect();
    let mut models: Vec<(&(String, &'static str), &ModelSpend)> = this.by_model.iter().collect();
    models.sort_by(|(a_key, a), (b_key, b)| b.exact.cmp(&a.exact).then(a_key.cmp(b_key)));
    let by_model: Vec<serde_json::Value> = models
        .into_iter()
        .map(|((model, tool), spend)| {
            serde_json::json!({
                "key": model,
                "tool": tool,
                "sessions": spend.sessions,
                "tokens": spend.tokens,
                "usd_micros_known": micros(spend.exact),
                "usd_partial": spend.partial,
            })
        })
        .collect();
    let mut folders: Vec<(&Option<String>, &Totals)> = this.by_folder.iter().collect();
    // Most spend first; then by label, the unknown folder last.
    folders.sort_by(|(a_key, a), (b_key, b)| {
        b.exact
            .cmp(&a.exact)
            .then(a_key.is_none().cmp(&b_key.is_none()))
            .then(a_key.cmp(b_key))
    });
    let by_folder: Vec<serde_json::Value> = folders
        .into_iter()
        .map(|(folder, totals)| {
            let mut value = totals.value();
            value["key"] = serde_json::json!(folder);
            value
        })
        .collect();
    let prices_as_of = tables.iter().map(|table| table.retrieved_at).min();
    let tables: Vec<serde_json::Value> = tables
        .iter()
        .map(|table| serde_json::json!({"table_id": table.table_id, "version": table.version}))
        .collect();
    serde_json::json!({
        "basis": PRICING_BASIS_API_LIST,
        "tables": tables,
        "prices_as_of": prices_as_of.map(|at| at.to_rfc3339()),
        "week": this.totals.value(),
        "last_week": last.as_ref().map(|last| last.totals.value()),
        "by_tool": by_tool,
        "by_model": by_model,
        "by_folder": by_folder,
        "by_day": this.by_day_value(week_start),
        "last_week_by_day": last.as_ref().map(|last| last.by_day_value(last_start)),
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use trace_commons_protocol::insights_usage_series::{CacheWrite, UsageSeries};

    use super::*;
    use crate::insights::codex_series::CodexTurnSeriesEvidence;
    use crate::insights::pricing_catalog::production_pricing_catalog;
    use crate::insights::session_facts::{
        EditMode, ExtensionClassCounts, InferredTask, ModelFacts, SessionShape,
    };
    use crate::insights::week_rollup::CodexObserved;

    const MILLION: u32 = 1_000_000;
    /// One `claude_turn` at the shipped claude-3-5-haiku list prices: 1M
    /// uncached at $0.80, 1M read at $0.08 and 1M output at $4.
    const HAIKU_TURN_MICROS: u64 = 4_880_000;
    /// Its 1M read tokens at $0.80 rather than $0.08.
    const HAIKU_TURN_SAVINGS: u64 = 720_000;

    /// Monday of the week on screen.
    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    fn claude_turn(ordinal: u32, when: DateTime<Utc>) -> TurnRecord {
        TurnRecord {
            ordinal,
            at: Some(when),
            uncached_input: Some(MILLION),
            cache_read: Some(MILLION),
            cache_write: Some(CacheWrite::Split { m5: 0, h1: 0 }),
            output: Some(MILLION),
            model_label_ix: Some(0),
            gap_secs: None,
            msg_key: None,
        }
    }

    fn facts(lines: Option<u32>, edits: Option<u32>) -> SessionFacts {
        SessionFacts {
            models: vec![ModelFacts {
                label_ix: 0,
                edits,
                denied: Some(1),
                undone: Some(0),
                interrupted: None,
            }],
            shape: SessionShape {
                edited_files: ExtensionClassCounts::default(),
                renamed_files: 0,
                lines_added: lines,
                lines_removed: lines.map(|lines| lines / 2),
                failed_before_first_edit_then_passed: 0,
            },
            task: InferredTask::Other,
            classifier_version: 0,
            active_secs: None,
            finished: None,
            user_turns: None,
            edit_mode: EditMode::Unknown,
            capped: false,
            delegated_unread: false,
            reviewer: false,
        }
    }

    fn claude(
        reference: &str,
        turns: Vec<TurnRecord>,
        facts: Option<SessionFacts>,
    ) -> SessionInput {
        SessionInput {
            session_ref: reference.to_owned(),
            import_seq: 0,
            placed_at: None,
            harness_session: None,
            automated: false,
            body: SessionBody::Claude {
                series: UsageSeries {
                    turns,
                    tool_calls: Vec::new(),
                    truncated: false,
                },
                model_labels: vec!["claude-3-5-haiku-20241022".to_owned()],
            },
            facts,
        }
    }

    /// A Codex session read before the per-turn reader: no series.
    fn codex_without_series(reference: &str, when: DateTime<Utc>) -> SessionInput {
        SessionInput {
            session_ref: reference.to_owned(),
            import_seq: 0,
            placed_at: None,
            harness_session: None,
            automated: false,
            body: SessionBody::Codex {
                observed: Some(CodexObserved {
                    first_at: when,
                    last_at: when,
                    input: 10,
                    cached_input: 0,
                    output: 10,
                    total: 20,
                    baseline_excluded: false,
                }),
                series: None,
            },
            facts: None,
        }
    }

    fn codex_with_series(reference: &str, when: DateTime<Utc>) -> SessionInput {
        let mut input = codex_without_series(reference, when);
        input.body = SessionBody::Codex {
            observed: None,
            series: Some(CodexTurnSeriesEvidence {
                model_labels: vec!["gpt-5".to_owned()],
                series: UsageSeries {
                    turns: vec![TurnRecord {
                        cache_write: Some(CacheWrite::TotalOnly(0)),
                        ..claude_turn(0, when)
                    }],
                    tool_calls: Vec::new(),
                    truncated: false,
                },
            }),
        };
        input
    }

    fn work(inputs: &[SessionInput], folders: &[(&str, Option<&str>)]) -> serde_json::Value {
        work_in(inputs, folders, utc())
    }

    fn work_in(
        inputs: &[SessionInput],
        folders: &[(&str, Option<&str>)],
        tz: FixedOffset,
    ) -> serde_json::Value {
        let folders = folders
            .iter()
            .map(|(reference, folder)| ((*reference).to_owned(), folder.map(str::to_owned)))
            .collect();
        work_value(
            inputs,
            &folders,
            &production_pricing_catalog(),
            monday(),
            tz,
            monday() - Duration::weeks(12),
            &BTreeMap::new(),
        )
    }

    #[test]
    fn priced_claude_sessions_sum_at_list_prices() {
        let value = work(
            &[
                claude("t1", vec![claude_turn(0, at(26, 9))], None),
                claude("t2", vec![claude_turn(0, at(27, 9))], None),
            ],
            &[],
        );
        assert_eq!(value["basis"], "api_list");
        assert_eq!(value["tables"].as_array().unwrap().len(), 1);
        assert!(value["tables"][0]["table_id"].is_string());
        let week = &value["week"];
        assert_eq!(week["sessions"], 2);
        assert_eq!(week["priced_sessions"], 2);
        assert_eq!(week["unpriced_sessions"], 0);
        assert_eq!(week["usd_micros_known"], 2 * HAIKU_TURN_MICROS);
        assert_eq!(week["usd_partial"], false);
        assert_eq!(
            week["cache_savings_usd_micros_known"],
            2 * HAIKU_TURN_SAVINGS
        );
        assert_eq!(value["by_tool"].as_array().unwrap().len(), 1);
        assert_eq!(value["by_tool"][0]["key"], "claude_code");
        assert_eq!(
            value["by_tool"][0]["usd_micros_known"],
            2 * HAIKU_TURN_MICROS
        );
        let model = &value["by_model"][0];
        assert_eq!(model["key"], "claude-3-5-haiku");
        assert_eq!(model["tool"], "claude_code");
        assert_eq!(model["sessions"], 2);
        assert_eq!(model["tokens"], 6 * u64::from(MILLION));
        assert_eq!(model["usd_micros_known"], 2 * HAIKU_TURN_MICROS);
        assert_eq!(model["usd_partial"], false);
    }

    #[test]
    fn an_unpriced_codex_session_makes_the_spend_partial_never_zero() {
        let value = work(
            &[
                claude("t1", vec![claude_turn(0, at(26, 9))], None),
                codex_without_series("t2", at(28, 9)),
            ],
            &[],
        );
        let week = &value["week"];
        assert_eq!(week["sessions"], 2);
        assert_eq!(week["priced_sessions"], 1);
        assert_eq!(week["unpriced_sessions"], 1);
        assert_eq!(week["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(week["usd_partial"], true);
        let codex = &value["by_tool"][1];
        assert_eq!(codex["key"], "codex");
        assert_eq!(codex["usd_micros_known"], 0);
        assert_eq!(codex["usd_partial"], true);
        assert_eq!(value["by_day"][2]["usd_partial"], true, "Wednesday");
        assert_eq!(value["by_day"][0]["usd_partial"], false);
        // A Codex session with a per-turn series prices: 1M uncached at
        // $1.25, 1M cached at $0.125, 1M output at $10.
        let priced = work(&[codex_with_series("t3", at(28, 9))], &[]);
        assert_eq!(priced["week"]["usd_micros_known"], 11_375_000);
        assert_eq!(priced["week"]["usd_partial"], false);
        assert_eq!(priced["by_model"][0]["key"], "gpt-5");
        assert_eq!(priced["by_model"][0]["tool"], "codex");
    }

    #[test]
    fn unknown_lines_and_edits_are_partial_never_zero() {
        let value = work(
            &[
                claude(
                    "t1",
                    vec![claude_turn(0, at(26, 9))],
                    Some(facts(Some(40), Some(3))),
                ),
                claude(
                    "t2",
                    vec![claude_turn(0, at(27, 9))],
                    Some(facts(None, None)),
                ),
            ],
            &[],
        );
        let week = &value["week"];
        assert_eq!(week["lines_added_known"], 40);
        assert_eq!(week["lines_removed_known"], 20);
        assert_eq!(week["lines_partial"], true);
        assert_eq!(week["edits_known"], 3);
        assert_eq!(week["edits_denied_known"], 2);
        assert_eq!(week["edits_undone_known"], 0);
        assert_eq!(week["edits_partial"], true);
        assert_eq!(week["edits_count_partial"], true);
        assert_eq!(week["edits_denied_partial"], false);
        let known = work(
            &[claude(
                "t1",
                vec![claude_turn(0, at(26, 9))],
                Some(facts(Some(40), Some(3))),
            )],
            &[],
        );
        assert_eq!(known["week"]["lines_partial"], false);
        assert_eq!(known["week"]["edits_partial"], false);
        // No facts at all: unknown, not none.
        let none = work(&[claude("t1", vec![claude_turn(0, at(26, 9))], None)], &[]);
        assert_eq!(none["week"]["lines_added_known"], 0);
        assert_eq!(none["week"]["lines_partial"], true);
        assert_eq!(none["week"]["edits_partial"], true);
    }

    #[test]
    fn folders_group_by_label_with_the_unknown_folder_last() {
        let value = work(
            &[
                claude("t1", vec![claude_turn(0, at(26, 9))], None),
                claude(
                    "t2",
                    vec![claude_turn(0, at(27, 9)), claude_turn(1, at(27, 10))],
                    None,
                ),
                claude("t3", vec![claude_turn(0, at(28, 9))], None),
                claude("t4", vec![claude_turn(0, at(28, 9))], None),
            ],
            &[
                ("t1", Some("alpha")),
                ("t2", Some("beta")),
                ("t3", Some("alpha")),
            ],
        );
        let folders = value["by_folder"].as_array().unwrap();
        assert_eq!(folders.len(), 3);
        assert_eq!(folders[0]["key"], "alpha");
        assert_eq!(folders[0]["sessions"], 2);
        assert_eq!(folders[0]["usd_micros_known"], 2 * HAIKU_TURN_MICROS);
        assert_eq!(folders[1]["key"], "beta");
        assert_eq!(folders[1]["sessions"], 1);
        assert_eq!(folders[1]["usd_micros_known"], 2 * HAIKU_TURN_MICROS);
        assert_eq!(folders[2]["key"], serde_json::Value::Null);
        assert_eq!(folders[2]["sessions"], 1);
    }

    #[test]
    fn last_week_is_totalled_apart_and_null_before_the_kept_weeks() {
        let value = work(
            &[
                claude(
                    "t1",
                    vec![claude_turn(0, at(26, 9))],
                    Some(facts(Some(10), Some(1))),
                ),
                claude(
                    "t2",
                    vec![claude_turn(0, at(20, 9)), claude_turn(1, at(21, 9))],
                    Some(facts(Some(30), Some(2))),
                ),
            ],
            &[],
        );
        assert_eq!(value["week"]["sessions"], 1);
        assert_eq!(value["week"]["lines_added_known"], 10);
        let last = &value["last_week"];
        assert_eq!(last["sessions"], 1);
        assert_eq!(last["usd_micros_known"], 2 * HAIKU_TURN_MICROS);
        assert_eq!(last["lines_added_known"], 30);
        assert_eq!(last["edits_known"], 2);
        let last_days = value["last_week_by_day"].as_array().unwrap();
        assert_eq!(last_days.len(), 7);
        assert_eq!(last_days[0]["date"], "2026-10-19");
        assert_eq!(last_days[1]["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(last_days[2]["usd_micros_known"], HAIKU_TURN_MICROS);
        // A session across the week boundary: its spend by its turns, its
        // lines once, in the week it began.
        let across = work(
            &[claude(
                "t1",
                vec![claude_turn(0, at(25, 23)), claude_turn(1, at(26, 1))],
                Some(facts(Some(8), Some(1))),
            )],
            &[],
        );
        assert_eq!(across["week"]["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(across["last_week"]["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(across["week"]["lines_added_known"], 0);
        assert_eq!(across["week"]["lines_partial"], false);
        assert_eq!(across["last_week"]["lines_added_known"], 8);
        // The oldest kept week has no week before it in the store.
        let folders = BTreeMap::new();
        let oldest = work_value(
            &[claude("t1", vec![claude_turn(0, at(26, 9))], None)],
            &folders,
            &production_pricing_catalog(),
            monday(),
            utc(),
            monday(),
            &BTreeMap::new(),
        );
        assert_eq!(oldest["last_week"], serde_json::Value::Null);
        assert_eq!(oldest["last_week_by_day"], serde_json::Value::Null);
    }

    #[test]
    fn spend_lands_on_the_local_day_of_each_turn() {
        // 23:00 UTC on Monday is 01:00 Tuesday two hours east.
        let east = FixedOffset::east_opt(2 * 3600).unwrap();
        let value = work_in(
            &[claude(
                "t1",
                vec![claude_turn(0, at(26, 23)), claude_turn(1, at(29, 9))],
                None,
            )],
            &[],
            east,
        );
        let days = value["by_day"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[0]["date"], "2026-10-26");
        assert_eq!(days[6]["date"], "2026-11-01");
        let spend: Vec<u64> = days
            .iter()
            .map(|day| day["usd_micros_known"].as_u64().unwrap())
            .collect();
        assert_eq!(spend, [0, HAIKU_TURN_MICROS, 0, HAIKU_TURN_MICROS, 0, 0, 0]);
        assert!(days.iter().all(|day| day["usd_partial"] == false));
    }

    #[test]
    fn a_turn_repeated_by_a_resumed_transcript_is_priced_once() {
        let mut first = claude_turn(0, at(26, 9));
        first.msg_key = Some(trace_commons_protocol::insights_usage_series::KeyedDigest(
            [7; 32],
        ));
        let mut resumed = claude("t2", vec![first.clone()], None);
        resumed.import_seq = 1;
        let value = work(&[claude("t1", vec![first], None), resumed], &[]);
        assert_eq!(value["week"]["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(value["week"]["sessions"], 1);
    }

    #[test]
    fn a_resumed_transcript_counts_only_its_new_lines_and_edits() {
        let key = |byte: u8| {
            Some(trace_commons_protocol::insights_usage_series::KeyedDigest(
                [byte; 32],
            ))
        };
        let mut old = claude_turn(0, at(19, 9));
        old.msg_key = key(1);
        let mut new = claude_turn(1, at(27, 9));
        new.msg_key = key(2);
        // Begun last week; resumed this week, repeating it facts and all.
        let first = claude("t1", vec![old.clone()], Some(facts(Some(10), Some(2))));
        let mut resumed = claude("t2", vec![old, new], Some(facts(Some(16), Some(3))));
        resumed.import_seq = 1;
        let value = work(&[first, resumed], &[]);
        assert_eq!(value["last_week"]["lines_added_known"], 10);
        assert_eq!(value["last_week"]["edits_known"], 2);
        assert_eq!(value["week"]["lines_added_known"], 6);
        assert_eq!(value["week"]["lines_removed_known"], 3);
        assert_eq!(value["week"]["edits_known"], 1);
        assert_eq!(value["week"]["lines_partial"], false);
        assert_eq!(value["week"]["edits_partial"], false);
    }

    #[test]
    fn a_resumed_transcript_whose_repeat_is_unknown_is_partial_never_twice() {
        let key = |byte: u8| {
            Some(trace_commons_protocol::insights_usage_series::KeyedDigest(
                [byte; 32],
            ))
        };
        let mut old = claude_turn(0, at(26, 9));
        old.msg_key = key(1);
        let mut new = claude_turn(1, at(27, 9));
        new.msg_key = key(2);
        let first = claude("t1", vec![old.clone()], Some(facts(None, None)));
        let mut resumed = claude("t2", vec![old, new], Some(facts(Some(16), Some(3))));
        resumed.import_seq = 1;
        let value = work(&[first, resumed], &[]);
        assert_eq!(value["week"]["lines_added_known"], 0);
        assert_eq!(value["week"]["lines_partial"], true);
        assert_eq!(value["week"]["edits_known"], 0);
        assert_eq!(value["week"]["edits_count_partial"], true);
    }

    #[test]
    fn a_session_sent_through_near_ai_is_never_list_priced() {
        let inputs = [
            claude("own", vec![claude_turn(0, at(26, 9))], None),
            claude("routed", vec![claude_turn(0, at(27, 9))], None),
            claude("mixed", vec![claude_turn(0, at(28, 9))], None),
        ];
        let routed = BTreeMap::from([
            ("routed".to_owned(), NearAiRoute::Wholly),
            ("mixed".to_owned(), NearAiRoute::Partly),
        ]);
        let value = work_value(
            &inputs,
            &BTreeMap::new(),
            &production_pricing_catalog(),
            monday(),
            utc(),
            monday() - Duration::weeks(12),
            &routed,
        );
        let week = &value["week"];
        assert_eq!(week["usd_micros_known"], HAIKU_TURN_MICROS);
        assert_eq!(week["sessions"], 3);
        assert_eq!(week["priced_sessions"], 1);
        assert_eq!(week["near_ai_sessions"], 1);
        assert_eq!(week["unpriced_sessions"], 1);
        assert_eq!(week["usd_partial"], true);
        let days: Vec<bool> = value["by_day"]
            .as_array()
            .unwrap()
            .iter()
            .map(|day| day["usd_partial"] == true)
            .collect();
        assert_eq!(days, [false, false, true, false, false, false, false]);
    }
}
