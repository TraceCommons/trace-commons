//! One completed local ISO week of token counters, with three-way coverage.
//!
//! Pure: the time zone is a parameter, and nothing reads a clock, a file or
//! a store. Claude and Codex are separate lines and are never summed. An
//! unknown figure is `None`; it is never shown as zero. Nothing is dated by
//! when it was imported.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{TurnRecord, UsageSeries};

use super::analytics_constants::{
    COMPARABLE_COVERAGE, COMPARABLE_MIN_SESSIONS, DEDUPE_TURNS_BY_MSG_KEY,
};
use super::cache_share::CacheShare;

/// Which counter rows a figure was computed from. Feeds are never blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feed {
    /// Feed S: files the user analyzed.
    Saved,
    /// Feed L: calls routed through the proxy.
    Ledger,
    /// Feed T: the daemon counter pass over watched folders.
    CounterPass,
}

/// Harnesses, in their fixed display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsSource {
    ClaudeCode,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Known,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageReason {
    // Partial.
    CodexBaselineExcluded,
    Truncated,
    SomeTurnsUnknown,
    SpansWeeks,
    // Unknown.
    NoUsageCounters,
    SourceUnsupported,
    Undated,
    ReimportOverlap,
    NotRouted,
    Stale,
}

impl CoverageReason {
    pub fn state(self) -> CoverageState {
        match self {
            Self::CodexBaselineExcluded
            | Self::Truncated
            | Self::SomeTurnsUnknown
            | Self::SpansWeeks => CoverageState::Partial,
            Self::NoUsageCounters
            | Self::SourceUnsupported
            | Self::Undated
            | Self::ReimportOverlap
            | Self::NotRouted
            | Self::Stale => CoverageState::Unknown,
        }
    }
}

/// Why a session has no counters at all, as its feed reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    NoUsageCounters,
    SourceUnsupported,
    NotRouted,
    Stale,
}

impl From<UnknownReason> for CoverageReason {
    fn from(reason: UnknownReason) -> Self {
        match reason {
            UnknownReason::NoUsageCounters => Self::NoUsageCounters,
            UnknownReason::SourceUnsupported => Self::SourceUnsupported,
            UnknownReason::NotRouted => Self::NotRouted,
            UnknownReason::Stale => Self::Stale,
        }
    }
}

/// Why a comparison or derived figure is not shown. Display-only reasons;
/// none of them is a session coverage reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    /// Week-to-week figures need feed T.
    NeedsCounterPass,
    BelowCoverageFloor,
    InsufficientHistory,
    /// The figure itself is unknown in one of the weeks, or its base is zero.
    NoFigure,
}

/// Codex first-to-last observed counter change. Cached input is a subset of
/// input, and `total` is input + output; nothing is added again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexObserved {
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
    pub total: u64,
    /// A nonzero baseline was present before the first observation.
    pub baseline_excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionBody {
    Claude {
        series: UsageSeries,
        /// Declared labels the turns' `model_label_ix` index into.
        model_labels: Vec<String>,
    },
    Codex {
        observed: Option<CodexObserved>,
    },
    Unknown {
        source: Option<AnalyticsSource>,
        reason: UnknownReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInput {
    /// An opaque snapshot ID or hashed session ID. Never a path.
    pub session_ref: String,
    /// Order the snapshot was saved in; larger is newer. Used only to pick
    /// which of two overlapping imports is counted, never to date anything.
    pub import_seq: u64,
    /// The session's first recorded event, for placing a session that has no
    /// dated turns. Never the import time.
    pub placed_at: Option<DateTime<Utc>>,
    pub body: SessionBody,
}

impl SessionInput {
    fn source(&self) -> Option<AnalyticsSource> {
        match &self.body {
            SessionBody::Claude { .. } => Some(AnalyticsSource::ClaudeCode),
            SessionBody::Codex { .. } => Some(AnalyticsSource::Codex),
            SessionBody::Unknown { source, .. } => *source,
        }
    }

    /// The dated span used for the overlap rule.
    fn dated_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        match &self.body {
            SessionBody::Claude { series, .. } => {
                let mut dates = series.turns.iter().filter_map(|turn| turn.at);
                let first = dates.next()?;
                Some(dates.fold((first, first), |(low, high), at| {
                    (low.min(at), high.max(at))
                }))
            }
            SessionBody::Codex { observed } => observed.map(|o| (o.first_at, o.last_at)),
            SessionBody::Unknown { .. } => None,
        }
    }

    /// Deduplicated by keyed message digest, rather than by the overlap rule.
    fn keyed(&self) -> bool {
        match &self.body {
            SessionBody::Claude { series, .. } => {
                DEDUPE_TURNS_BY_MSG_KEY
                    && !series.turns.is_empty()
                    && series.turns.iter().all(|turn| turn.msg_key.is_some())
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekCoverage {
    pub known: u32,
    pub partial: u32,
    pub unknown: u32,
    /// Sessions per reason. A session may carry more than one.
    pub reasons: BTreeMap<CoverageReason, u32>,
}

impl WeekCoverage {
    pub fn sessions(&self) -> u32 {
        self.known + self.partial + self.unknown
    }
}

/// One harness's line for the week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceWeek {
    pub source: AnalyticsSource,
    pub sessions: u32,
    /// `None` when no session of this source has a known figure this week.
    pub tokens: Option<u64>,
    pub largest_session_tokens: Option<u64>,
    pub cache_share: Option<CacheShare>,
}

/// One session's row in the drill-down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWeek {
    pub session_ref: String,
    pub source: Option<AnalyticsSource>,
    pub tokens: Option<u64>,
    pub state: CoverageState,
    pub reasons: Vec<CoverageReason>,
    /// This session's own share of input read from cache, normalized for its
    /// source. `None` when its figure is unknown.
    pub cache_share: Option<CacheShare>,
}

/// Claude tokens on one local date, stacked. Cache write is its own series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTokens {
    pub date: NaiveDate,
    pub uncached: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

/// Tokens under one declared model label. `label: None` is "Unknown label".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokens {
    pub label: Option<String>,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekRollup {
    pub feed: Feed,
    /// Monday of the local ISO week.
    pub week_start: NaiveDate,
    pub coverage: WeekCoverage,
    /// Sessions with no recorded date at all; they are in no week.
    pub undated_sessions: u32,
    /// Present harnesses, in fixed order. Never summed together.
    pub sources: Vec<SourceWeek>,
    /// Claude by local date, Monday first. `None` when no Claude turn is
    /// known this week.
    pub by_day: Option<Vec<DayTokens>>,
    /// Codex deltas, which are intervals and not by day.
    pub codex_interval_tokens: Option<u64>,
    /// Alphabetical by declared label, unknown last. Never by value.
    pub by_model: Vec<ModelTokens>,
    pub sessions: Vec<SessionWeek>,
}

/// Monday of the local ISO week holding `at`.
pub fn local_week_start<Tz: TimeZone>(at: &DateTime<Utc>, tz: &Tz) -> NaiveDate {
    monday_of(at.with_timezone(tz).date_naive())
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Per-session outcome of the reimport rules.
struct Prepared<'a> {
    input: &'a SessionInput,
    /// Turns still counted here (Claude only).
    kept_turns: Vec<&'a TurnRecord>,
    reimport_overlap: bool,
}

fn prepare(sessions: &[SessionInput]) -> Vec<Prepared<'_>> {
    let mut order: Vec<usize> = (0..sessions.len()).collect();
    order.sort_by_key(|&index| (sessions[index].import_seq, index));
    let mut prepared: Vec<Option<Prepared<'_>>> = (0..sessions.len()).map(|_| None).collect();
    let mut seen = BTreeSet::new();
    for &index in &order {
        let input = &sessions[index];
        let mut kept_turns = Vec::new();
        let mut reimport_overlap = false;
        if let SessionBody::Claude { series, .. } = &input.body {
            if input.keyed() {
                // Earliest snapshot holding a turn keeps it.
                for turn in &series.turns {
                    if seen.insert(turn.msg_key.expect("keyed")) {
                        kept_turns.push(turn);
                    }
                }
                reimport_overlap = kept_turns.is_empty();
            } else {
                kept_turns = series.turns.iter().collect();
            }
        }
        prepared[index] = Some(Prepared {
            input,
            kept_turns,
            reimport_overlap,
        });
    }
    // The overlap rule, for any pair where either side is not keyed: the
    // older of two overlapping snapshots of one harness is not counted.
    for (rank, &older) in order.iter().enumerate() {
        let older_input = &sessions[older];
        let Some((older_low, older_high)) = older_input.dated_range() else {
            continue;
        };
        let overlapped = order[rank + 1..].iter().any(|&newer| {
            let newer_input = &sessions[newer];
            newer_input.source() == older_input.source()
                && (!older_input.keyed() || !newer_input.keyed())
                && newer_input
                    .dated_range()
                    .is_some_and(|(low, high)| low <= older_high && older_low <= high)
        });
        if overlapped {
            prepared[older].as_mut().expect("prepared").reimport_overlap = true;
        }
    }
    prepared.into_iter().map(|p| p.expect("prepared")).collect()
}

/// Per-session contribution to one week.
#[derive(Default)]
struct Contribution {
    tokens: Option<u64>,
    reasons: BTreeSet<CoverageReason>,
    uncached: u64,
    cache_read: u64,
    cache_write: u64,
    codex: Option<CodexObserved>,
}

/// Roll up the completed local ISO week starting at `week_start`.
pub fn week_rollup<Tz: TimeZone>(
    feed: Feed,
    sessions: &[SessionInput],
    week_start: NaiveDate,
    tz: &Tz,
) -> WeekRollup {
    let week_start = monday_of(week_start);
    let in_week = |at: &DateTime<Utc>| local_week_start(at, tz) == week_start;
    let mut coverage = WeekCoverage::default();
    let mut undated_sessions = 0;
    let mut rows = Vec::new();
    let mut lines: BTreeMap<AnalyticsSource, SourceWeek> = BTreeMap::new();
    let mut days: Vec<DayTokens> = (0..7)
        .map(|offset| DayTokens {
            date: week_start + Duration::days(offset),
            uncached: 0,
            cache_read: 0,
            cache_write: 0,
            output: 0,
        })
        .collect();
    let mut any_claude_known = false;
    let mut codex_interval: Option<u64> = None;
    let mut models: BTreeMap<String, u64> = BTreeMap::new();
    let mut unknown_label_tokens: Option<u64> = None;

    for prepared in prepare(sessions) {
        let input = prepared.input;
        let placed_in_week = input.placed_at.as_ref().map(&in_week);
        let mut contribution = Contribution::default();
        let present = match &input.body {
            SessionBody::Claude {
                series,
                model_labels,
            } => {
                let any_dated = series.turns.iter().any(|turn| turn.at.is_some());
                let touches = |turns: &mut dyn Iterator<Item = &TurnRecord>| {
                    turns.filter_map(|turn| turn.at.as_ref()).any(&in_week)
                };
                if prepared.reimport_overlap {
                    if touches(&mut series.turns.iter())
                        || (!any_dated && placed_in_week == Some(true))
                    {
                        contribution.reasons.insert(CoverageReason::ReimportOverlap);
                        true
                    } else {
                        if !any_dated && placed_in_week.is_none() {
                            undated_sessions += 1;
                        }
                        false
                    }
                } else if !any_dated {
                    match placed_in_week {
                        Some(true) => {
                            contribution.reasons.insert(if series.turns.is_empty() {
                                CoverageReason::NoUsageCounters
                            } else {
                                CoverageReason::Undated
                            });
                            true
                        }
                        Some(false) => false,
                        None => {
                            undated_sessions += 1;
                            false
                        }
                    }
                } else if !touches(&mut prepared.kept_turns.iter().copied()) {
                    // Every turn this week was counted in an earlier snapshot.
                    false
                } else {
                    let mut known = 0u32;
                    let mut unknown = 0u32;
                    let mut tokens = 0u64;
                    for turn in &prepared.kept_turns {
                        let Some(when) = turn.at else {
                            unknown += 1;
                            continue;
                        };
                        if !in_week(&when) {
                            continue;
                        }
                        let Some(usage) = turn.known_usage() else {
                            unknown += 1;
                            continue;
                        };
                        known += 1;
                        tokens += usage.total();
                        contribution.uncached += u64::from(usage.uncached_input);
                        contribution.cache_read += u64::from(usage.cache_read);
                        contribution.cache_write += usage.cache_write.total();
                        let local = when.with_timezone(tz).date_naive();
                        let day = &mut days[(local - week_start).num_days() as usize];
                        day.uncached += u64::from(usage.uncached_input);
                        day.cache_read += u64::from(usage.cache_read);
                        day.cache_write += usage.cache_write.total();
                        day.output += u64::from(usage.output);
                        let label = turn
                            .model_label_ix
                            .and_then(|ix| model_labels.get(usize::from(ix)));
                        match label {
                            Some(label) => {
                                *models.entry(label.clone()).or_default() += usage.total()
                            }
                            None => {
                                *unknown_label_tokens.get_or_insert(0) += usage.total();
                            }
                        }
                    }
                    if known == 0 {
                        contribution.reasons.insert(CoverageReason::NoUsageCounters);
                    } else {
                        any_claude_known = true;
                        contribution.tokens = Some(tokens);
                        if unknown > 0 {
                            contribution
                                .reasons
                                .insert(CoverageReason::SomeTurnsUnknown);
                        }
                    }
                    if series.truncated {
                        contribution.reasons.insert(CoverageReason::Truncated);
                    }
                    true
                }
            }
            SessionBody::Codex { observed } => match observed {
                Some(observed) => {
                    let first = local_week_start(&observed.first_at, tz);
                    let last = local_week_start(&observed.last_at, tz);
                    if week_start < first || last < week_start {
                        false
                    } else if prepared.reimport_overlap {
                        contribution.reasons.insert(CoverageReason::ReimportOverlap);
                        true
                    } else if first != last {
                        contribution.reasons.insert(CoverageReason::SpansWeeks);
                        true
                    } else {
                        contribution.tokens = Some(observed.total);
                        contribution.codex = Some(*observed);
                        *codex_interval.get_or_insert(0) += observed.total;
                        if observed.baseline_excluded {
                            contribution
                                .reasons
                                .insert(CoverageReason::CodexBaselineExcluded);
                        }
                        true
                    }
                }
                None => match placed_in_week {
                    Some(true) => {
                        contribution.reasons.insert(CoverageReason::NoUsageCounters);
                        true
                    }
                    Some(false) => false,
                    None => {
                        undated_sessions += 1;
                        false
                    }
                },
            },
            SessionBody::Unknown { reason, .. } => match placed_in_week {
                Some(true) => {
                    contribution.reasons.insert((*reason).into());
                    true
                }
                Some(false) => false,
                None => {
                    undated_sessions += 1;
                    false
                }
            },
        };
        if !present {
            continue;
        }

        let state = contribution
            .reasons
            .iter()
            .map(|reason| reason.state())
            .max()
            .unwrap_or(CoverageState::Known);
        // An unknown session contributes no figure, whatever it carried.
        if state == CoverageState::Unknown {
            contribution.tokens = None;
        }
        match state {
            CoverageState::Known => coverage.known += 1,
            CoverageState::Partial => coverage.partial += 1,
            CoverageState::Unknown => coverage.unknown += 1,
        }
        for reason in &contribution.reasons {
            *coverage.reasons.entry(*reason).or_default() += 1;
        }
        let mut session_share = None;
        if let Some(source) = input.source() {
            let line = lines.entry(source).or_insert(SourceWeek {
                source,
                sessions: 0,
                tokens: None,
                largest_session_tokens: None,
                cache_share: None,
            });
            line.sessions += 1;
            if let Some(tokens) = contribution.tokens {
                *line.tokens.get_or_insert(0) += tokens;
                line.largest_session_tokens = line.largest_session_tokens.max(Some(tokens));
                let share = match source {
                    AnalyticsSource::ClaudeCode => CacheShare::claude(
                        contribution.uncached,
                        contribution.cache_read,
                        contribution.cache_write,
                    ),
                    AnalyticsSource::Codex => contribution
                        .codex
                        .and_then(|o| CacheShare::codex(o.input, o.cached_input)),
                };
                session_share = share;
                if let Some(share) = share {
                    match &mut line.cache_share {
                        Some(total) => total.merge(share),
                        None => line.cache_share = Some(share),
                    }
                }
            }
        }
        rows.push(SessionWeek {
            session_ref: input.session_ref.clone(),
            source: input.source(),
            tokens: contribution.tokens,
            state,
            reasons: contribution.reasons.into_iter().collect(),
            cache_share: session_share,
        });
    }

    let mut by_model: Vec<ModelTokens> = models
        .into_iter()
        .map(|(label, tokens)| ModelTokens {
            label: Some(label),
            tokens,
        })
        .collect();
    if let Some(tokens) = unknown_label_tokens {
        by_model.push(ModelTokens {
            label: None,
            tokens,
        });
    }

    WeekRollup {
        feed,
        week_start,
        coverage,
        undated_sessions,
        sources: lines.into_values().collect(),
        by_day: any_claude_known.then_some(days),
        codex_interval_tokens: codex_interval,
        by_model,
        sessions: rows,
    }
}

/// Per session, in input order: for a Claude session the ordinals of the
/// turns this week counts, which are its turns dated in the week and not
/// already counted in an earlier snapshot. `None` for any other session, and
/// for a Claude snapshot excluded as a reimport overlap or with no such turn.
/// The Patterns figures read tool calls on these turns only, so a reimported
/// turn's calls are counted once, like its tokens.
pub fn counted_claude_turns<Tz: TimeZone>(
    sessions: &[SessionInput],
    week_start: NaiveDate,
    tz: &Tz,
) -> Vec<Option<BTreeSet<u32>>> {
    let week_start = monday_of(week_start);
    prepare(sessions)
        .into_iter()
        .map(|prepared| {
            if prepared.reimport_overlap
                || !matches!(prepared.input.body, SessionBody::Claude { .. })
            {
                return None;
            }
            let counted: BTreeSet<u32> = prepared
                .kept_turns
                .iter()
                .filter(|turn| {
                    turn.at
                        .as_ref()
                        .is_some_and(|at| local_week_start(at, tz) == week_start)
                })
                .map(|turn| turn.ordinal)
                .collect();
            (!counted.is_empty()).then_some(counted)
        })
        .collect()
}

/// Whether a week may be compared with another: feed T only, known sessions
/// at least [`COMPARABLE_COVERAGE`] of all, and at least
/// [`COMPARABLE_MIN_SESSIONS`] sessions.
pub fn comparable(rollup: &WeekRollup) -> Result<(), Unavailable> {
    if rollup.feed != Feed::CounterPass {
        return Err(Unavailable::NeedsCounterPass);
    }
    let sessions = rollup.coverage.sessions();
    if sessions < COMPARABLE_MIN_SESSIONS
        || !COMPARABLE_COVERAGE.reached_by(u64::from(rollup.coverage.known), u64::from(sessions))
    {
        return Err(Unavailable::BelowCoverageFloor);
    }
    Ok(())
}

/// (this - last) / last per mille, rounded half away from zero. `None` when
/// `last` is zero.
pub fn change_permille(this: u64, last: u64) -> Option<i64> {
    if last == 0 {
        return None;
    }
    let numerator = (i128::from(this) - i128::from(last)) * 1_000;
    let denominator = i128::from(last);
    let magnitude = (numerator.abs() * 2 + denominator) / (denominator * 2);
    i64::try_from(magnitude * numerator.signum()).ok()
}

/// Change in one source's tokens against the previous week.
pub fn change_vs_last_week(
    this: &WeekRollup,
    last: &WeekRollup,
    source: AnalyticsSource,
) -> Result<i64, Unavailable> {
    comparable(this)?;
    comparable(last)?;
    let tokens = |rollup: &WeekRollup| {
        rollup
            .sources
            .iter()
            .find(|line| line.source == source)
            .and_then(|line| line.tokens)
    };
    let (Some(now), Some(before)) = (tokens(this), tokens(last)) else {
        return Err(Unavailable::NoFigure);
    };
    change_permille(now, before).ok_or(Unavailable::NoFigure)
}

/// What a cached rollup was computed for, apart from the store generation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RollupCacheKey {
    pub feed: Feed,
    pub week_start: NaiveDate,
    /// The time-zone name the week was bucketed in.
    pub tz: String,
}

/// Rollups stamped with the store generation they were computed at. A read
/// at any other generation misses, so a delete or replace that advances the
/// generation can never be answered from a stale value.
#[derive(Debug, Clone, Default)]
pub struct RollupCache {
    entries: BTreeMap<RollupCacheKey, (u64, WeekRollup)>,
}

impl RollupCache {
    pub fn get(&self, key: &RollupCacheKey, generation: u64) -> Option<&WeekRollup> {
        self.entries
            .get(key)
            .filter(|(stamped, _)| *stamped == generation)
            .map(|(_, rollup)| rollup)
    }

    pub fn insert(&mut self, key: RollupCacheKey, generation: u64, rollup: WeekRollup) {
        self.entries.insert(key, (generation, rollup));
    }

    /// Every key held, at whatever generation it was stamped.
    pub fn keys(&self) -> impl Iterator<Item = &RollupCacheKey> {
        self.entries.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::{
        CacheWrite, KeyedDigest, TurnRecord, UsageSeries,
    };

    // Monday 2026-10-05.
    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    fn turn(ordinal: u32, when: Option<DateTime<Utc>>, model: Option<u16>) -> TurnRecord {
        TurnRecord {
            ordinal,
            at: when,
            uncached_input: Some(100),
            cache_read: Some(600),
            cache_write: Some(CacheWrite::Split { m5: 300, h1: 0 }),
            output: Some(50),
            model_label_ix: model,
            gap_secs: None,
            msg_key: None,
        }
    }

    const TURN_TOTAL: u64 = 100 + 600 + 300 + 50;

    fn claude(name: &str, seq: u64, turns: Vec<TurnRecord>) -> SessionInput {
        SessionInput {
            session_ref: name.into(),
            import_seq: seq,
            placed_at: None,
            body: SessionBody::Claude {
                series: UsageSeries {
                    turns,
                    tool_calls: vec![],
                    truncated: false,
                },
                model_labels: vec!["zeta-model".into(), "alpha-model".into()],
            },
        }
    }

    fn codex(name: &str, seq: u64, first: DateTime<Utc>, last: DateTime<Utc>) -> SessionInput {
        SessionInput {
            session_ref: name.into(),
            import_seq: seq,
            placed_at: None,
            body: SessionBody::Codex {
                observed: Some(CodexObserved {
                    first_at: first,
                    last_at: last,
                    input: 1_000,
                    cached_input: 400,
                    output: 200,
                    total: 1_200,
                    baseline_excluded: false,
                }),
            },
        }
    }

    fn unknown(name: &str, reason: UnknownReason, placed: Option<DateTime<Utc>>) -> SessionInput {
        SessionInput {
            session_ref: name.into(),
            import_seq: 0,
            placed_at: placed,
            body: SessionBody::Unknown {
                source: Some(AnalyticsSource::ClaudeCode),
                reason,
            },
        }
    }

    fn roll(sessions: &[SessionInput]) -> WeekRollup {
        week_rollup(Feed::Saved, sessions, monday(), &Utc)
    }

    fn source(rollup: &WeekRollup, which: AnalyticsSource) -> &SourceWeek {
        rollup
            .sources
            .iter()
            .find(|line| line.source == which)
            .unwrap()
    }

    fn session<'a>(rollup: &'a WeekRollup, name: &str) -> &'a SessionWeek {
        rollup
            .sessions
            .iter()
            .find(|s| s.session_ref == name)
            .unwrap()
    }

    #[test]
    fn claude_tokens_sum_dated_known_turns_in_the_week_only() {
        let rollup = roll(&[claude(
            "a",
            1,
            vec![
                turn(0, Some(at(5, 9)), None),
                turn(1, Some(at(11, 23)), None),
                turn(2, Some(at(12, 0)), None),
            ],
        )]);
        let line = source(&rollup, AnalyticsSource::ClaudeCode);
        assert_eq!(line.tokens, Some(2 * TURN_TOTAL));
        assert_eq!(line.sessions, 1);
        assert_eq!(rollup.coverage.known, 1);
    }

    #[test]
    fn claude_and_codex_are_separate_lines_in_fixed_order() {
        let rollup = roll(&[
            codex("c", 1, at(6, 1), at(6, 2)),
            claude("a", 2, vec![turn(0, Some(at(6, 9)), None)]),
        ]);
        let order: Vec<_> = rollup.sources.iter().map(|line| line.source).collect();
        assert_eq!(
            order,
            vec![AnalyticsSource::ClaudeCode, AnalyticsSource::Codex]
        );
        assert_eq!(source(&rollup, AnalyticsSource::Codex).tokens, Some(1_200));
        assert_eq!(
            source(&rollup, AnalyticsSource::ClaudeCode).tokens,
            Some(TURN_TOTAL)
        );
    }

    #[test]
    fn a_codex_delta_that_spans_weeks_is_partial_with_no_tokens() {
        let rollup = roll(&[codex("c", 1, at(4, 22), at(5, 2))]);
        let entry = session(&rollup, "c");
        assert_eq!(entry.state, CoverageState::Partial);
        assert_eq!(entry.reasons, vec![CoverageReason::SpansWeeks]);
        assert_eq!(entry.tokens, None);
        assert_eq!(source(&rollup, AnalyticsSource::Codex).tokens, None);
    }

    #[test]
    fn a_codex_excluded_baseline_is_partial_but_counted() {
        let mut input = codex("c", 1, at(6, 1), at(6, 2));
        if let SessionBody::Codex {
            observed: Some(observed),
        } = &mut input.body
        {
            observed.baseline_excluded = true;
        }
        let rollup = roll(&[input]);
        let entry = session(&rollup, "c");
        assert_eq!(entry.state, CoverageState::Partial);
        assert_eq!(entry.reasons, vec![CoverageReason::CodexBaselineExcluded]);
        assert_eq!(entry.tokens, Some(1_200));
    }

    #[test]
    fn some_unknown_turns_make_a_session_partial() {
        let mut unknown_turn = turn(1, Some(at(6, 10)), None);
        unknown_turn.output = None;
        let rollup = roll(&[claude(
            "a",
            1,
            vec![turn(0, Some(at(6, 9)), None), unknown_turn],
        )]);
        let entry = session(&rollup, "a");
        assert_eq!(entry.state, CoverageState::Partial);
        assert_eq!(entry.reasons, vec![CoverageReason::SomeTurnsUnknown]);
        assert_eq!(entry.tokens, Some(TURN_TOTAL));
    }

    #[test]
    fn an_undated_turn_in_a_dated_session_makes_it_partial() {
        let rollup = roll(&[claude(
            "a",
            1,
            vec![turn(0, Some(at(6, 9)), None), turn(1, None, None)],
        )]);
        let entry = session(&rollup, "a");
        assert_eq!(entry.reasons, vec![CoverageReason::SomeTurnsUnknown]);
        assert_eq!(entry.tokens, Some(TURN_TOTAL));
    }

    #[test]
    fn all_unknown_turns_are_unknown_never_zero() {
        let mut only = turn(0, Some(at(6, 9)), None);
        only.cache_read = None;
        let rollup = roll(&[claude("a", 1, vec![only])]);
        let entry = session(&rollup, "a");
        assert_eq!(entry.state, CoverageState::Unknown);
        assert_eq!(entry.reasons, vec![CoverageReason::NoUsageCounters]);
        assert_eq!(entry.tokens, None);
        assert_eq!(source(&rollup, AnalyticsSource::ClaudeCode).tokens, None);
        assert_eq!(
            source(&rollup, AnalyticsSource::ClaudeCode).cache_share,
            None
        );
        assert_eq!(rollup.by_day, None);
    }

    #[test]
    fn a_truncated_series_is_partial() {
        let mut input = claude("a", 1, vec![turn(0, Some(at(6, 9)), None)]);
        if let SessionBody::Claude { series, .. } = &mut input.body {
            series.truncated = true;
        }
        let entry_rollup = roll(&[input]);
        let entry = session(&entry_rollup, "a");
        assert_eq!(entry.state, CoverageState::Partial);
        assert_eq!(entry.reasons, vec![CoverageReason::Truncated]);
    }

    #[test]
    fn an_undated_session_is_in_no_week_unless_placed() {
        let rollup = roll(&[claude("a", 1, vec![turn(0, None, None)])]);
        assert!(rollup.sessions.is_empty());
        assert_eq!(rollup.undated_sessions, 1);

        let mut placed = claude("b", 1, vec![turn(0, None, None)]);
        placed.placed_at = Some(at(7, 3));
        let rollup = roll(&[placed]);
        let entry = session(&rollup, "b");
        assert_eq!(entry.state, CoverageState::Unknown);
        assert_eq!(entry.reasons, vec![CoverageReason::Undated]);
        assert_eq!(rollup.undated_sessions, 0);
    }

    #[test]
    fn unknown_sessions_carry_their_reason() {
        let rollup = roll(&[
            unknown("u1", UnknownReason::SourceUnsupported, Some(at(6, 1))),
            unknown("u2", UnknownReason::NotRouted, Some(at(6, 1))),
            unknown("u3", UnknownReason::Stale, Some(at(6, 1))),
            unknown("u4", UnknownReason::NoUsageCounters, Some(at(6, 1))),
            unknown("u5", UnknownReason::Stale, None),
        ]);
        assert_eq!(rollup.coverage.unknown, 4);
        assert_eq!(
            rollup.coverage.reasons[&CoverageReason::SourceUnsupported],
            1
        );
        assert_eq!(rollup.coverage.reasons[&CoverageReason::NotRouted], 1);
        assert_eq!(rollup.coverage.reasons[&CoverageReason::Stale], 1);
        assert_eq!(rollup.coverage.reasons[&CoverageReason::NoUsageCounters], 1);
        assert_eq!(rollup.undated_sessions, 1);
    }

    fn keyed(ordinal: u32, when: DateTime<Utc>, key: u8) -> TurnRecord {
        let mut t = turn(ordinal, Some(when), None);
        t.msg_key = Some(KeyedDigest([key; 32]));
        t
    }

    #[test]
    fn reimported_turns_count_once_in_the_earliest_snapshot() {
        let first = claude(
            "old",
            1,
            vec![keyed(0, at(6, 9), 1), keyed(1, at(6, 10), 2)],
        );
        // The regrown file: the same two turns plus one more.
        let regrown = claude(
            "new",
            2,
            vec![
                keyed(0, at(6, 9), 1),
                keyed(1, at(6, 10), 2),
                keyed(2, at(6, 11), 3),
            ],
        );
        let rollup = roll(&[regrown, first]);
        assert_eq!(session(&rollup, "old").tokens, Some(2 * TURN_TOTAL));
        assert_eq!(session(&rollup, "new").tokens, Some(TURN_TOTAL));
        assert_eq!(
            source(&rollup, AnalyticsSource::ClaudeCode).tokens,
            Some(3 * TURN_TOTAL)
        );
    }

    #[test]
    fn a_snapshot_whose_turns_this_week_were_all_counted_earlier_is_not_listed() {
        let old = claude("old", 1, vec![keyed(0, at(6, 9), 1)]);
        let regrown = claude(
            "new",
            2,
            vec![keyed(0, at(6, 9), 1), keyed(1, at(13, 9), 2)],
        );
        let rollup = roll(&[old, regrown]);
        assert_eq!(rollup.sessions.len(), 1);
        assert_eq!(rollup.coverage.known, 1);
        assert_eq!(rollup.coverage.unknown, 0);
    }

    #[test]
    fn a_wholly_reimported_snapshot_is_a_reimport_overlap() {
        let one = claude("old", 1, vec![keyed(0, at(6, 9), 1)]);
        let two = claude("new", 2, vec![keyed(0, at(6, 9), 1)]);
        let rollup = roll(&[one, two]);
        let entry = session(&rollup, "new");
        assert_eq!(entry.state, CoverageState::Unknown);
        assert_eq!(entry.reasons, vec![CoverageReason::ReimportOverlap]);
        assert_eq!(
            source(&rollup, AnalyticsSource::ClaudeCode).tokens,
            Some(TURN_TOTAL)
        );
        const { assert!(DEDUPE_TURNS_BY_MSG_KEY) };
    }

    #[test]
    fn overlapping_codex_snapshots_keep_the_newer() {
        let older = codex("older", 1, at(6, 1), at(6, 5));
        let newer = codex("newer", 2, at(6, 4), at(6, 8));
        let apart = codex("apart", 3, at(8, 1), at(8, 2));
        let rollup = roll(&[newer, older, apart]);
        assert_eq!(
            session(&rollup, "older").reasons,
            vec![CoverageReason::ReimportOverlap]
        );
        assert_eq!(session(&rollup, "newer").state, CoverageState::Known);
        assert_eq!(session(&rollup, "apart").state, CoverageState::Known);
        assert_eq!(
            source(&rollup, AnalyticsSource::Codex).tokens,
            Some(2 * 1_200)
        );
    }

    #[test]
    fn unkeyed_claude_snapshots_fall_back_to_range_overlap() {
        let older = claude(
            "older",
            1,
            vec![turn(0, Some(at(6, 1)), None), turn(1, Some(at(6, 5)), None)],
        );
        let newer = claude("newer", 2, vec![turn(0, Some(at(6, 5)), None)]);
        let rollup = roll(&[older, newer]);
        assert_eq!(
            session(&rollup, "older").reasons,
            vec![CoverageReason::ReimportOverlap]
        );
        assert_eq!(session(&rollup, "newer").state, CoverageState::Known);
    }

    #[test]
    fn different_harnesses_never_overlap_each_other() {
        let rollup = roll(&[
            codex("c", 1, at(6, 1), at(6, 5)),
            claude("a", 2, vec![turn(0, Some(at(6, 2)), None)]),
        ]);
        assert_eq!(rollup.coverage.known, 2);
    }

    #[test]
    fn the_week_is_local_monday_to_sunday() {
        let tz = chrono_tz::America::New_York;
        // Monday 2026-10-12 03:30 UTC is Sunday 23:30 in New York.
        let late_sunday = Utc.with_ymd_and_hms(2026, 10, 12, 3, 30, 0).unwrap();
        let input = [claude("a", 1, vec![turn(0, Some(late_sunday), None)])];
        let this = week_rollup(Feed::Saved, &input, monday(), &tz);
        assert_eq!(this.coverage.known, 1);
        let next = week_rollup(Feed::Saved, &input, monday() + Duration::days(7), &tz);
        assert_eq!(next.coverage.known, 0);
        assert_eq!(local_week_start(&late_sunday, &tz), monday());
        assert_eq!(
            local_week_start(&late_sunday, &Utc),
            monday() + Duration::days(7)
        );
    }

    #[test]
    fn a_daylight_saving_week_is_bucketed_by_local_date() {
        let tz = chrono_tz::America::New_York;
        // US clocks fall back on Sunday 2026-11-01. Week of Monday 10-26.
        let week = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        // Sunday 11-01 23:30 EST is Monday 11-02 04:30 UTC.
        let sunday_night = Utc.with_ymd_and_hms(2026, 11, 2, 4, 30, 0).unwrap();
        // Monday 10-26 00:30 EDT is Monday 10-26 04:30 UTC.
        let monday_morning = Utc.with_ymd_and_hms(2026, 10, 26, 4, 30, 0).unwrap();
        let input = [claude(
            "a",
            1,
            vec![
                turn(0, Some(monday_morning), None),
                turn(1, Some(sunday_night), None),
            ],
        )];
        let rollup = week_rollup(Feed::Saved, &input, week, &tz);
        assert_eq!(session(&rollup, "a").tokens, Some(2 * TURN_TOTAL));
        let days = rollup.by_day.unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[0].date, week);
        assert_eq!(days[0].output, 50);
        assert_eq!(days[6].output, 50);
    }

    #[test]
    fn a_week_start_that_is_not_monday_is_normalized() {
        let rollup = week_rollup(Feed::Saved, &[], monday() + Duration::days(3), &Utc);
        assert_eq!(rollup.week_start, monday());
    }

    #[test]
    fn by_day_stacks_the_four_series() {
        let rollup = roll(&[claude(
            "a",
            1,
            vec![
                turn(0, Some(at(6, 9)), None),
                turn(1, Some(at(6, 10)), None),
            ],
        )]);
        let days = rollup.by_day.unwrap();
        let tuesday = &days[1];
        assert_eq!(
            (
                tuesday.uncached,
                tuesday.cache_read,
                tuesday.cache_write,
                tuesday.output
            ),
            (200, 1_200, 600, 100)
        );
        assert_eq!(days[0].output, 0);
    }

    #[test]
    fn codex_goes_on_its_own_interval_row() {
        let rollup = roll(&[codex("c", 1, at(6, 1), at(6, 2))]);
        assert_eq!(rollup.codex_interval_tokens, Some(1_200));
        assert_eq!(rollup.by_day, None);
    }

    #[test]
    fn largest_session_is_per_source() {
        let rollup = roll(&[
            claude("small", 1, vec![turn(0, Some(at(6, 1)), None)]),
            claude(
                "big",
                2,
                vec![turn(0, Some(at(7, 1)), None), turn(1, Some(at(7, 2)), None)],
            ),
        ]);
        let line = source(&rollup, AnalyticsSource::ClaudeCode);
        assert_eq!(line.largest_session_tokens, Some(2 * TURN_TOTAL));
        assert_eq!(line.sessions, 2);
    }

    #[test]
    fn each_session_row_carries_its_own_cache_share_for_the_drill_down() {
        let rollup = roll(&[
            claude("a", 1, vec![turn(0, Some(at(6, 9)), None)]),
            codex("c", 2, at(7, 9), at(7, 10)),
            unknown("u", UnknownReason::NoUsageCounters, Some(at(8, 9))),
        ]);
        assert_eq!(
            session(&rollup, "a").cache_share,
            CacheShare::claude(100, 600, 300)
        );
        assert_eq!(
            session(&rollup, "c").cache_share,
            CacheShare::codex(1_000, 400)
        );
        assert_eq!(session(&rollup, "u").cache_share, None);
    }

    #[test]
    fn the_cache_lists_the_keys_it_holds() {
        let mut cache = RollupCache::default();
        let key = RollupCacheKey {
            feed: Feed::Saved,
            week_start: monday(),
            tz: "+00:00".into(),
        };
        cache.insert(key.clone(), 3, roll(&[]));
        assert_eq!(cache.keys().cloned().collect::<Vec<_>>(), vec![key]);
    }

    #[test]
    fn cache_share_is_per_source() {
        let rollup = roll(&[
            claude("a", 1, vec![turn(0, Some(at(6, 1)), None)]),
            codex("c", 2, at(6, 1), at(6, 2)),
        ]);
        assert_eq!(
            source(&rollup, AnalyticsSource::ClaudeCode)
                .cache_share
                .unwrap()
                .permille(),
            600
        );
        assert_eq!(
            source(&rollup, AnalyticsSource::Codex)
                .cache_share
                .unwrap()
                .permille(),
            400
        );
    }

    fn by_model_labels(rollup: &WeekRollup) -> Vec<Option<String>> {
        rollup.by_model.iter().map(|m| m.label.clone()).collect()
    }

    #[test]
    fn by_model_order_is_alphabetical_with_unknown_last() {
        let rollup = roll(&[claude(
            "a",
            1,
            vec![
                turn(0, Some(at(6, 1)), None),
                turn(1, Some(at(6, 2)), Some(0)),
                turn(2, Some(at(6, 3)), Some(1)),
                turn(3, Some(at(6, 4)), Some(9)),
            ],
        )]);
        assert_eq!(
            by_model_labels(&rollup),
            vec![Some("alpha-model".into()), Some("zeta-model".into()), None]
        );
        // Index 9 is outside the label table: it counts as unknown.
        assert_eq!(rollup.by_model[2].tokens, 2 * TURN_TOTAL);
    }

    #[test]
    fn by_model_order_does_not_depend_on_the_figures() {
        let heavy = |ix: u16| {
            let mut t = turn(0, Some(at(6, 1)), Some(ix));
            t.output = Some(1_000_000);
            t
        };
        let light = |ix: u16, ordinal: u32| turn(ordinal, Some(at(6, 2)), Some(ix));
        let zeta_heavy = roll(&[claude("a", 1, vec![heavy(0), light(1, 1)])]);
        let alpha_heavy = roll(&[claude("a", 1, vec![heavy(1), light(0, 1)])]);
        assert_ne!(
            zeta_heavy.by_model[0].tokens,
            alpha_heavy.by_model[0].tokens
        );
        assert_eq!(by_model_labels(&zeta_heavy), by_model_labels(&alpha_heavy));
    }

    fn week_with(feed: Feed, known: u32, unknown_count: u32) -> WeekRollup {
        let mut sessions = Vec::new();
        for i in 0..known {
            sessions.push(claude(
                &format!("k{i}"),
                u64::from(i),
                vec![turn(0, Some(at(6, i % 20)), None)],
            ));
        }
        for i in 0..unknown_count {
            sessions.push(unknown(
                &format!("u{i}"),
                UnknownReason::Stale,
                Some(at(6, 1)),
            ));
        }
        week_rollup(feed, &sessions, monday(), &Utc)
    }

    #[test]
    fn comparisons_need_the_counter_pass_feed() {
        assert_eq!(
            comparable(&week_with(Feed::Saved, 10, 0)),
            Err(Unavailable::NeedsCounterPass)
        );
        assert_eq!(
            comparable(&week_with(Feed::Ledger, 10, 0)),
            Err(Unavailable::NeedsCounterPass)
        );
        assert_eq!(comparable(&week_with(Feed::CounterPass, 10, 0)), Ok(()));
    }

    #[test]
    fn comparable_needs_four_fifths_known() {
        assert_eq!(comparable(&week_with(Feed::CounterPass, 8, 2)), Ok(()));
        assert_eq!(
            comparable(&week_with(Feed::CounterPass, 7, 2)),
            Err(Unavailable::BelowCoverageFloor)
        );
    }

    #[test]
    fn comparable_needs_five_sessions() {
        assert_eq!(COMPARABLE_MIN_SESSIONS, 5);
        assert_eq!(comparable(&week_with(Feed::CounterPass, 5, 0)), Ok(()));
        assert_eq!(
            comparable(&week_with(Feed::CounterPass, 4, 0)),
            Err(Unavailable::BelowCoverageFloor)
        );
    }

    #[test]
    fn change_is_permille_rounded_half_away_from_zero() {
        assert_eq!(change_permille(1_000, 1_000), Some(0));
        assert_eq!(change_permille(1_500, 1_000), Some(500));
        assert_eq!(change_permille(500, 1_000), Some(-500));
        // 1/2000 = 0.5 permille: rounds away from zero both ways.
        assert_eq!(change_permille(2_001, 2_000), Some(1));
        assert_eq!(change_permille(1_999, 2_000), Some(-1));
        assert_eq!(change_permille(3_001, 3_000), Some(0));
        assert_eq!(change_permille(5, 0), None);
    }

    #[test]
    fn change_vs_last_week_needs_both_weeks_comparable() {
        let this = week_with(Feed::CounterPass, 6, 0);
        let last = week_with(Feed::CounterPass, 5, 0);
        assert_eq!(
            change_vs_last_week(&this, &last, AnalyticsSource::ClaudeCode),
            Ok(200)
        );
        let thin = week_with(Feed::CounterPass, 4, 0);
        assert_eq!(
            change_vs_last_week(&this, &thin, AnalyticsSource::ClaudeCode),
            Err(Unavailable::BelowCoverageFloor)
        );
        assert_eq!(
            change_vs_last_week(&this, &last, AnalyticsSource::Codex),
            Err(Unavailable::NoFigure)
        );
    }

    #[test]
    fn a_cached_rollup_is_used_only_at_its_generation() {
        let mut cache = RollupCache::default();
        let key = RollupCacheKey {
            feed: Feed::Saved,
            week_start: monday(),
            tz: "UTC".into(),
        };
        cache.insert(key.clone(), 7, roll(&[]));
        assert!(cache.get(&key, 7).is_some());
        assert!(cache.get(&key, 8).is_none());
        let other_week = RollupCacheKey {
            week_start: monday() + Duration::days(7),
            ..key.clone()
        };
        assert!(cache.get(&other_week, 7).is_none());
    }

    #[test]
    fn coverage_reasons_split_into_partial_and_unknown() {
        for reason in [
            CoverageReason::CodexBaselineExcluded,
            CoverageReason::Truncated,
            CoverageReason::SomeTurnsUnknown,
            CoverageReason::SpansWeeks,
        ] {
            assert_eq!(reason.state(), CoverageState::Partial, "{reason:?}");
        }
        for reason in [
            CoverageReason::NoUsageCounters,
            CoverageReason::SourceUnsupported,
            CoverageReason::Undated,
            CoverageReason::ReimportOverlap,
            CoverageReason::NotRouted,
            CoverageReason::Stale,
        ] {
            assert_eq!(reason.state(), CoverageState::Unknown, "{reason:?}");
        }
    }
}
