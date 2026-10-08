//! Feed S: the week rollup over the snapshots the user saved, stamped with
//! the store generation it was computed at.
//!
//! The saved snapshots are mapped onto the engine's counter rows here and
//! nowhere else. A snapshot is dated only by what its own records say; the
//! import time orders two overlapping imports and never places a session in
//! a week. Under feed S nothing is compared with another week: the
//! denominator is the files the user chose, so "vs last week" and "Your best
//! week" are typed as unavailable, never shown as a figure.
//!
//! # Generation
//!
//! The generation is a digest of which snapshots are saved and when each was
//! last imported, so any delete, replace or reimport changes it, whichever
//! process made the change. A cached rollup carries the generation it was
//! computed at; a read at any other generation recomputes. The in-process
//! mutations (`delete`, a saved `analyze`, `repair`) also recompute every
//! cached week for their store before they return.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::insights_usage_series::UsageSeries;

use super::cache_share::CacheShare;
use super::usage::NativeTokenCounts;
use super::week_rollup::{
    AnalyticsSource, CodexObserved, CoverageReason, CoverageState, DayTokens, Feed, ModelTokens,
    RollupCache, RollupCacheKey, SessionBody, SessionInput, Unavailable, UnknownReason,
    WeekCoverage, WeekRollup, local_week_start, week_rollup,
};
use super::{LocalInsight, LocalInsightStore, SourceFormat};

/// Weeks the overview's week picker offers, newest first. A bound on the
/// response, not an analytics constant.
pub const MAX_PICKER_WEEKS: usize = 26;

/// The widest UTC offset a request may carry, in seconds (18 hours either
/// way), the same bound `insights_glance` applies.
pub const MAX_TZ_OFFSET_SECS: i32 = 18 * 3_600;

/// The time zone a request's week is bucketed in: the shell's current UTC
/// offset in seconds east. `None` when it is out of range.
pub fn request_tz(seconds: i32) -> Option<FixedOffset> {
    if seconds.abs() > MAX_TZ_OFFSET_SECS {
        return None;
    }
    FixedOffset::east_opt(seconds)
}

/// Why a figure that feed T would carry is shown as "—" here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByProjectUnavailable {
    /// Owner decision D7, open: a project key is path-derived, and analyzed
    /// files carry none.
    NotAvailableForAnalyzedFiles,
}

/// A cache share with its parts, so a shell divides nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareFigure {
    pub numerator: u64,
    pub denominator: u64,
    /// Per mille, rounded half up.
    pub permille: u64,
}

impl From<CacheShare> for ShareFigure {
    fn from(share: CacheShare) -> Self {
        Self {
            numerator: share.numerator,
            denominator: share.denominator,
            permille: share.permille(),
        }
    }
}

/// One harness's line. Lines are never summed together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewSource {
    pub source: AnalyticsSource,
    pub sessions: u32,
    /// `None` when no session of this source has a known figure this week.
    pub tokens: Option<u64>,
    pub cache_share: Option<ShareFigure>,
    /// The largest session of this source by its own tokens.
    pub largest_session: Option<LargestSession>,
    /// "vs last week": always unavailable under feed S.
    pub change: Unavailable,
    /// "Your best week": always unavailable under feed S.
    pub best_week: Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargestSession {
    /// The snapshot ID. Never a path.
    pub session_ref: String,
    pub tokens: u64,
}

/// One harness under "By tool", in fixed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTokens {
    pub source: AnalyticsSource,
    pub tokens: Option<u64>,
}

/// The Overview tab's "This week" over feed S.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekOverview {
    pub feed: Feed,
    pub generation: u64,
    /// Monday and Sunday of the local ISO week.
    pub week_start: NaiveDate,
    pub week_end: NaiveDate,
    pub tz: i32,
    pub coverage: WeekCoverage,
    pub undated_sessions: u32,
    /// Sessions in the week, of any coverage state.
    pub sessions: u32,
    pub sources: Vec<OverviewSource>,
    /// Claude by local date, Monday first. `None` when no Claude turn is
    /// known this week.
    pub by_day: Option<Vec<DayTokens>>,
    pub codex_interval_tokens: Option<u64>,
    /// Alphabetical by declared label, unknown last. Never by value.
    pub by_model: Vec<ModelTokens>,
    pub by_tool: Vec<ToolTokens>,
    pub by_project: ByProjectUnavailable,
    /// Weeks with a dated saved session, newest first, for the week picker.
    pub weeks: Vec<NaiveDate>,
}

/// The figure a drill-down explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewCard {
    Tokens,
    CacheShare,
    Sessions,
}

/// One session's row in a drill-down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardSession {
    pub session_ref: String,
    pub source: Option<AnalyticsSource>,
    pub tokens: Option<u64>,
    pub cache_share: Option<ShareFigure>,
    pub state: CoverageState,
    pub reasons: Vec<CoverageReason>,
}

/// "What makes up this number".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardInputs {
    pub card: OverviewCard,
    pub feed: Feed,
    pub generation: u64,
    pub week_start: NaiveDate,
    pub tz: i32,
    /// Per source: tokens, and the cache share's numerator and denominator.
    pub sources: Vec<OverviewSource>,
    pub coverage: WeekCoverage,
    /// Every session in the week, of any coverage state, in the engine's
    /// order. Never sorted by value.
    pub sessions: Vec<CardSession>,
}

/// A digest of which snapshots are saved and when each was last imported.
pub fn saved_generation(reports: &[LocalInsight]) -> u64 {
    let mut entries: Vec<(&str, i64)> = reports
        .iter()
        .map(|insight| {
            (
                insight.id.as_str(),
                insight
                    .analyzed_at
                    .timestamp_nanos_opt()
                    .unwrap_or(i64::MAX),
            )
        })
        .collect();
    entries.sort_unstable();
    let mut hash = Sha256::new();
    for (id, at) in entries {
        hash.update(id.as_bytes());
        hash.update([0]);
        hash.update(at.to_be_bytes());
    }
    let digest = hash.finalize();
    u64::from_be_bytes(digest[..8].try_into().expect("eight bytes"))
}

fn zero(counts: &NativeTokenCounts) -> bool {
    match *counts {
        NativeTokenCounts::Codex {
            input,
            cached_input,
            output,
            reasoning_output,
            total,
        } => input == 0 && cached_input == 0 && output == 0 && reasoning_output == 0 && total == 0,
        NativeTokenCounts::ClaudeCode { .. } => false,
    }
}

fn codex_observed(insight: &LocalInsight) -> Option<CodexObserved> {
    let evidence = insight.usage_evidence.as_ref()?;
    evidence
        .validate_binding(
            insight.source_format,
            &insight.report.evidence.first()?.source_digest,
        )
        .ok()?;
    let interval = evidence.interval.as_ref()?;
    let NativeTokenCounts::Codex {
        input,
        cached_input,
        output,
        total,
        ..
    } = interval.observed_delta
    else {
        return None;
    };
    Some(CodexObserved {
        first_at: interval.baseline.recorded_at,
        last_at: interval.final_snapshot.recorded_at,
        input,
        cached_input,
        output,
        total,
        baseline_excluded: !zero(&interval.unpriced_prior),
    })
}

/// Map saved snapshots onto counter rows.
///
/// A Claude series whose digest key differs from the newest snapshot's has
/// its message digests dropped: digests under different keys never match, so
/// those snapshots fall back to the overlap rule instead of double counting.
pub fn saved_sessions(reports: &[LocalInsight]) -> Vec<SessionInput> {
    let mut ordered: Vec<&LocalInsight> = reports.iter().collect();
    // Import order only picks which of two overlapping imports is counted.
    ordered.sort_by(|a, b| a.analyzed_at.cmp(&b.analyzed_at).then(a.id.cmp(&b.id)));
    let current_key = ordered
        .iter()
        .rev()
        .find_map(|insight| insight.turn_series.as_ref())
        .map(|series| series.key_fingerprint);
    ordered
        .into_iter()
        .enumerate()
        .map(|(seq, insight)| {
            let placed_at = insight
                .time_evidence
                .as_ref()
                .and_then(|evidence| evidence.earliest.as_ref())
                .map(|earliest| earliest.recorded_at);
            let body = match insight.source_format {
                SourceFormat::ClaudeCode => match insight.turn_series.as_ref().filter(|series| {
                    series
                        .validate_binding(&insight.report.evidence[0].source_digest)
                        .is_ok()
                }) {
                    Some(evidence) => {
                        let mut series: UsageSeries = evidence.series.clone();
                        if Some(evidence.key_fingerprint) != current_key {
                            for turn in &mut series.turns {
                                turn.msg_key = None;
                            }
                        }
                        SessionBody::Claude {
                            series,
                            model_labels: evidence.model_labels.clone(),
                        }
                    }
                    // Saved before store version 13, or without a digest key:
                    // unknown until reimported, never zero.
                    None => SessionBody::Unknown {
                        source: Some(AnalyticsSource::ClaudeCode),
                        reason: UnknownReason::NoUsageCounters,
                    },
                },
                SourceFormat::Codex => SessionBody::Codex {
                    observed: codex_observed(insight),
                },
                SourceFormat::Trajectory => SessionBody::Unknown {
                    source: None,
                    reason: UnknownReason::SourceUnsupported,
                },
            };
            SessionInput {
                session_ref: insight.id.clone(),
                import_seq: seq as u64,
                placed_at,
                body,
            }
        })
        .collect()
}

/// Weeks holding a dated record of any saved session, newest first.
fn dated_weeks(sessions: &[SessionInput], tz: &FixedOffset) -> Vec<NaiveDate> {
    let mut weeks = BTreeSet::new();
    let mut add = |at: &DateTime<Utc>| {
        weeks.insert(local_week_start(at, tz));
    };
    for session in sessions {
        match &session.body {
            SessionBody::Claude { series, .. } => series
                .turns
                .iter()
                .filter_map(|t| t.at.as_ref())
                .for_each(&mut add),
            SessionBody::Codex {
                observed: Some(observed),
            } => {
                add(&observed.first_at);
                add(&observed.last_at);
            }
            _ => {}
        }
        if let Some(at) = &session.placed_at {
            add(at);
        }
    }
    weeks.into_iter().rev().take(MAX_PICKER_WEEKS).collect()
}

/// The feed S rollup for one week, stamped with the generation of `reports`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekGlance {
    pub generation: u64,
    pub tz: i32,
    pub rollup: WeekRollup,
    pub weeks: Vec<NaiveDate>,
}

/// Compute without any cache.
pub fn compute(reports: &[LocalInsight], week_start: NaiveDate, tz: FixedOffset) -> WeekGlance {
    let sessions = saved_sessions(reports);
    WeekGlance {
        generation: saved_generation(reports),
        tz: tz.local_minus_utc(),
        rollup: week_rollup(Feed::Saved, &sessions, week_start, &tz),
        weeks: dated_weeks(&sessions, &tz),
    }
}

/// Generation-stamped rollups per store directory. The week picker list is
/// kept beside each entry, under the same stamp.
type StoreCache = (RollupCache, BTreeMap<RollupCacheKey, Vec<NaiveDate>>);
static CACHE: Mutex<BTreeMap<PathBuf, StoreCache>> = Mutex::new(BTreeMap::new());

fn cache_key(week_start: NaiveDate, tz: FixedOffset) -> RollupCacheKey {
    use chrono::Datelike;
    RollupCacheKey {
        feed: Feed::Saved,
        week_start: week_start
            - Duration::days(i64::from(week_start.weekday().num_days_from_monday())),
        tz: tz.to_string(),
    }
}

fn store(
    cache: &mut BTreeMap<PathBuf, StoreCache>,
    dir: &std::path::Path,
    key: RollupCacheKey,
    glance: &WeekGlance,
) {
    let (rollups, weeks) = cache.entry(dir.to_path_buf()).or_default();
    rollups.insert(key.clone(), glance.generation, glance.rollup.clone());
    weeks.insert(key, glance.weeks.clone());
}

/// The rollup for a saved store, from cache when its stamp matches the
/// store's current generation, recomputed otherwise.
pub fn week_glance(
    store_ref: &LocalInsightStore,
    week_start: NaiveDate,
    tz: FixedOffset,
) -> Result<WeekGlance> {
    let reports = store_ref.list()?;
    let generation = saved_generation(&reports);
    let key = cache_key(week_start, tz);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((rollups, weeks)) = cache.get(&store_ref.dir)
        && let Some(rollup) = rollups.get(&key, generation)
        && let Some(weeks) = weeks.get(&key)
    {
        return Ok(WeekGlance {
            generation,
            tz: tz.local_minus_utc(),
            rollup: rollup.clone(),
            weeks: weeks.clone(),
        });
    }
    let glance = compute(&reports, week_start, tz);
    store(&mut cache, &store_ref.dir, key, &glance);
    Ok(glance)
}

/// Recompute every cached week of this store at its current generation.
/// Called by each in-process snapshot mutation before it returns.
pub fn refresh_after_mutation(store_ref: &LocalInsightStore) -> Result<()> {
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some((rollups, _)) = cache.get(&store_ref.dir) else {
        return Ok(());
    };
    let keys: Vec<RollupCacheKey> = rollups.keys().cloned().collect();
    if keys.is_empty() {
        return Ok(());
    }
    let reports = store_ref.list()?;
    for key in keys {
        let Some(tz) = key
            .tz
            .parse::<FixedOffset>()
            .ok()
            .and_then(|tz| request_tz(tz.local_minus_utc()))
        else {
            continue;
        };
        let glance = compute(&reports, key.week_start, tz);
        store(&mut cache, &store_ref.dir, key, &glance);
    }
    Ok(())
}

/// The generation a store's cached week is stamped at, if it is cached.
#[cfg(test)]
pub(crate) fn cached_generation(
    store_ref: &LocalInsightStore,
    week_start: NaiveDate,
    tz: FixedOffset,
) -> Option<u64> {
    let key = cache_key(week_start, tz);
    let cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (rollups, _) = cache.get(&store_ref.dir)?;
    let reports = store_ref.list().ok()?;
    let generation = saved_generation(&reports);
    rollups.get(&key, generation).map(|_| generation)
}

fn sources(rollup: &WeekRollup) -> Vec<OverviewSource> {
    rollup
        .sources
        .iter()
        .map(|line| {
            let largest_session = rollup
                .sessions
                .iter()
                .filter(|row| row.source == Some(line.source))
                .filter_map(|row| row.tokens.map(|tokens| (tokens, &row.session_ref)))
                // Ties go to the first in engine order; never re-sorted.
                .fold(
                    None::<(u64, &String)>,
                    |best, (tokens, session)| match best {
                        Some((most, _)) if most >= tokens => best,
                        _ => Some((tokens, session)),
                    },
                )
                .map(|(tokens, session_ref)| LargestSession {
                    session_ref: session_ref.clone(),
                    tokens,
                });
            OverviewSource {
                source: line.source,
                sessions: line.sessions,
                tokens: line.tokens,
                cache_share: line.cache_share.map(ShareFigure::from),
                largest_session,
                change: Unavailable::NeedsCounterPass,
                best_week: Unavailable::NeedsCounterPass,
            }
        })
        .collect()
}

pub fn overview(glance: &WeekGlance) -> WeekOverview {
    let rollup = &glance.rollup;
    let by_tool = [AnalyticsSource::ClaudeCode, AnalyticsSource::Codex]
        .into_iter()
        .filter_map(|source| {
            rollup
                .sources
                .iter()
                .find(|line| line.source == source)
                .map(|line| ToolTokens {
                    source,
                    tokens: line.tokens,
                })
        })
        .collect();
    WeekOverview {
        feed: rollup.feed,
        generation: glance.generation,
        week_start: rollup.week_start,
        week_end: rollup.week_start + Duration::days(6),
        tz: glance.tz,
        coverage: rollup.coverage.clone(),
        undated_sessions: rollup.undated_sessions,
        sessions: rollup.coverage.sessions(),
        sources: sources(rollup),
        by_day: rollup.by_day.clone(),
        codex_interval_tokens: rollup.codex_interval_tokens,
        by_model: rollup.by_model.clone(),
        by_tool,
        by_project: ByProjectUnavailable::NotAvailableForAnalyzedFiles,
        weeks: glance.weeks.clone(),
    }
}

pub fn card_inputs(glance: &WeekGlance, card: OverviewCard) -> CardInputs {
    let rollup = &glance.rollup;
    CardInputs {
        card,
        feed: rollup.feed,
        generation: glance.generation,
        week_start: rollup.week_start,
        tz: glance.tz,
        sources: sources(rollup),
        coverage: rollup.coverage.clone(),
        sessions: rollup
            .sessions
            .iter()
            .map(|row| CardSession {
                session_ref: row.session_ref.clone(),
                source: row.source,
                tokens: row.tokens,
                cache_share: row.cache_share.map(ShareFigure::from),
                state: row.state,
                reasons: row.reasons.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::service::{LocalInsightsOperation, LocalInsightsRequest, execute};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;

    const CLAUDE_FIXTURE: &[u8] =
        include_bytes!("../../fixtures/insights/claude-turn-series/session.jsonl");

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Codex rollout whose observed interval is on 2026-09-11 (week of
    /// 2026-09-07). `final_input` changes the bytes.
    fn codex_source(path: &Path, final_input: u64) {
        let token = |time: &str, input: u64, cached: u64, output: u64| {
            json!({
                "type":"event_msg", "timestamp":time,
                "payload":{"type":"token_count","info":{"total_token_usage":{
                    "input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,
                    "reasoning_output_tokens":0,"total_tokens":input + output
                }}}
            })
        };
        let rows = [
            json!({"type":"session_meta","timestamp":"2026-09-11T00:00:00Z","payload":{"id":"PRIVATE_SESSION_ID","model_provider":"openai"}}),
            json!({"type":"turn_context","timestamp":"2026-09-11T00:00:00Z","payload":{"model":"fixture-model"}}),
            token("2026-09-11T00:00:01Z", 100, 20, 20),
            json!({"type":"response_item","timestamp":"2026-09-11T00:00:02Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"PRIVATE_BODY"}]}}),
            token("2026-09-11T00:00:03Z", final_input, 40, 30),
        ];
        fs::write(
            path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }

    fn line(overview: &WeekOverview, source: AnalyticsSource) -> Option<&OverviewSource> {
        overview.sources.iter().find(|line| line.source == source)
    }

    #[test]
    fn saved_snapshots_land_in_the_week_their_own_records_date_them() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let codex = root.path().join("codex.jsonl");
        codex_source(&codex, 150);
        let claude = root.path().join("claude.jsonl");
        fs::write(&claude, CLAUDE_FIXTURE).unwrap();
        store.import(SourceFormat::Codex, &codex).unwrap();
        let saved = store.import(SourceFormat::ClaudeCode, &claude).unwrap();
        assert!(saved.turn_series.is_some());
        let reports = store.list().unwrap();

        let codex_week = overview(&compute(&reports, date(2026, 9, 11), utc()));
        assert_eq!(codex_week.week_start, date(2026, 9, 7));
        assert_eq!(codex_week.week_end, date(2026, 9, 13));
        let codex_line = line(&codex_week, AnalyticsSource::Codex).unwrap();
        // The observed delta, input + output, nothing added again.
        assert_eq!(codex_line.tokens, Some((150 - 100) + (30 - 20)));
        assert_eq!(codex_line.change, Unavailable::NeedsCounterPass);
        assert_eq!(codex_line.best_week, Unavailable::NeedsCounterPass);
        assert!(line(&codex_week, AnalyticsSource::ClaudeCode).is_none());
        assert_eq!(codex_week.feed, Feed::Saved);
        assert_eq!(
            codex_week.by_project,
            ByProjectUnavailable::NotAvailableForAnalyzedFiles
        );

        let claude_week = overview(&compute(&reports, date(2026, 9, 14), utc()));
        let claude_line = line(&claude_week, AnalyticsSource::ClaudeCode).unwrap();
        assert!(claude_line.tokens.is_some_and(|tokens| tokens > 0));
        assert!(claude_line.cache_share.is_some());
        assert_eq!(
            claude_line.largest_session.as_ref().unwrap().session_ref,
            saved.id
        );
        assert!(claude_week.by_day.is_some());
        assert!(line(&claude_week, AnalyticsSource::Codex).is_none());
        assert_eq!(claude_week.weeks, vec![date(2026, 9, 14), date(2026, 9, 7)]);
        // Nothing is dated by the import time.
        let today = overview(&compute(&reports, Utc::now().date_naive(), utc()));
        assert_eq!(today.sessions, 0);
    }

    #[test]
    fn a_claude_snapshot_without_a_series_is_unknown_and_never_zero() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let claude = root.path().join("claude.jsonl");
        fs::write(&claude, CLAUDE_FIXTURE).unwrap();
        store.import(SourceFormat::ClaudeCode, &claude).unwrap();
        let mut reports = store.list().unwrap();
        reports[0].turn_series = None;
        let week = overview(&compute(&reports, date(2026, 9, 14), utc()));
        assert_eq!(week.coverage.unknown, 1);
        assert_eq!(
            week.coverage.reasons.get(&CoverageReason::NoUsageCounters),
            Some(&1)
        );
        let claude_line = line(&week, AnalyticsSource::ClaudeCode).unwrap();
        assert_eq!(claude_line.tokens, None);
        assert_eq!(claude_line.cache_share, None);
        assert!(week.by_day.is_none());
        let inputs = card_inputs(
            &compute(&reports, date(2026, 9, 14), utc()),
            OverviewCard::Tokens,
        );
        assert_eq!(inputs.sessions[0].tokens, None);
        assert_eq!(inputs.sessions[0].state, CoverageState::Unknown);
    }

    #[test]
    fn a_trajectory_snapshot_is_source_unsupported() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let reports = store.list().unwrap();
        assert!(saved_sessions(&reports).is_empty());
        let mut claude = {
            let path = root.path().join("claude.jsonl");
            fs::write(&path, CLAUDE_FIXTURE).unwrap();
            store.import(SourceFormat::ClaudeCode, &path).unwrap()
        };
        claude.source_format = SourceFormat::Trajectory;
        let sessions = saved_sessions(&[claude]);
        assert!(matches!(
            sessions[0].body,
            SessionBody::Unknown {
                source: None,
                reason: UnknownReason::SourceUnsupported
            }
        ));
    }

    #[test]
    fn drill_down_rows_carry_their_parts() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let codex = root.path().join("codex.jsonl");
        codex_source(&codex, 150);
        let saved = store.import(SourceFormat::Codex, &codex).unwrap();
        let glance = compute(&store.list().unwrap(), date(2026, 9, 7), utc());
        let inputs = card_inputs(&glance, OverviewCard::CacheShare);
        assert_eq!(inputs.card, OverviewCard::CacheShare);
        assert_eq!(inputs.sessions.len(), 1);
        assert_eq!(inputs.sessions[0].session_ref, saved.id);
        let share = inputs.sessions[0].cache_share.unwrap();
        // cached_input / input over the observed delta: (40-20)/(150-100).
        assert_eq!((share.numerator, share.denominator), (20, 50));
        assert_eq!(share.permille, 400);
        assert_eq!(inputs.sources[0].cache_share, Some(share));
    }

    #[test]
    fn the_week_is_bucketed_in_the_requested_offset() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let codex = root.path().join("codex.jsonl");
        codex_source(&codex, 150);
        store.import(SourceFormat::Codex, &codex).unwrap();
        let reports = store.list().unwrap();
        // 2026-09-11T00:00Z is still Thursday 2026-09-10 at UTC-5, the same
        // week; at UTC+14 it is Friday. Both in the week of 09-07.
        for offset in [-5 * 3_600, 14 * 3_600] {
            let tz = request_tz(offset).unwrap();
            let week = overview(&compute(&reports, date(2026, 9, 9), tz));
            assert_eq!(week.tz, offset);
            assert_eq!(week.sessions, 1);
        }
        assert!(request_tz(19 * 3_600).is_none());
    }

    #[test]
    fn the_generation_changes_on_delete_replace_and_reimport() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let path = root.path().join("codex.jsonl");
        codex_source(&path, 150);
        let empty = saved_generation(&store.list().unwrap());
        let first = store.import(SourceFormat::Codex, &path).unwrap();
        let saved = saved_generation(&store.list().unwrap());
        assert_ne!(saved, empty);
        // Reimporting the same bytes keeps one session but is a new import.
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.import(SourceFormat::Codex, &path).unwrap();
        let reimported = saved_generation(&store.list().unwrap());
        assert_ne!(reimported, saved);
        assert_eq!(store.list().unwrap().len(), 1);
        codex_source(&path, 160);
        let replaced = store.import(SourceFormat::Codex, &path).unwrap();
        assert_ne!(replaced.id, first.id);
        let after_replace = saved_generation(&store.list().unwrap());
        assert_ne!(after_replace, reimported);
        store.delete(&replaced.id).unwrap();
        assert_eq!(saved_generation(&store.list().unwrap()), empty);
    }

    #[test]
    fn a_cached_week_from_an_older_generation_is_never_served() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let path = root.path().join("codex.jsonl");
        codex_source(&path, 150);
        let saved = store.import(SourceFormat::Codex, &path).unwrap();
        let week = date(2026, 9, 7);
        let before = week_glance(&store, week, utc()).unwrap();
        assert_eq!(before.rollup.coverage.sessions(), 1);
        // A change made outside this module, as another process would.
        store.delete(&saved.id).unwrap();
        let after = week_glance(&store, week, utc()).unwrap();
        assert_ne!(after.generation, before.generation);
        assert_eq!(after.rollup.coverage.sessions(), 0);
    }

    fn request(store: &Path, operation: LocalInsightsOperation) -> LocalInsightsRequest {
        LocalInsightsRequest {
            store_dir: Some(store.to_path_buf()),
            operation,
        }
    }

    /// Prime the cache for the Codex week, then return the store handle.
    fn primed(store_dir: &Path) -> LocalInsightStore {
        let store = LocalInsightStore::open(store_dir).unwrap();
        week_glance(&store, date(2026, 9, 7), utc()).unwrap();
        store
    }

    fn cached_sessions(store: &LocalInsightStore) -> Option<u32> {
        cached_generation(store, date(2026, 9, 7), utc())?;
        Some(
            week_glance(store, date(2026, 9, 7), utc())
                .unwrap()
                .rollup
                .coverage
                .sessions(),
        )
    }

    #[test]
    fn a_saved_delete_recomputes_the_cached_week_before_it_returns() {
        let root = tempfile::tempdir().unwrap();
        let store_dir = root.path().join("store");
        let path = root.path().join("codex.jsonl");
        codex_source(&path, 150);
        let saved = LocalInsightStore::open(&store_dir)
            .unwrap()
            .import(SourceFormat::Codex, &path)
            .unwrap();
        let store = primed(&store_dir);
        assert_eq!(cached_sessions(&store), Some(1));
        execute(request(
            &store_dir,
            LocalInsightsOperation::Delete { id: saved.id },
        ))
        .unwrap();
        // Stamped at the new generation already: a read hits the cache.
        assert_eq!(cached_sessions(&store), Some(0));
    }

    #[test]
    fn a_saved_replace_recomputes_the_cached_week_before_it_returns() {
        let root = tempfile::tempdir().unwrap();
        let store_dir = root.path().join("store");
        let path = root.path().join("codex.jsonl");
        codex_source(&path, 150);
        let analyze = || LocalInsightsOperation::Analyze {
            source: SourceFormat::Codex,
            file: path.clone(),
            save: true,
        };
        execute(request(&store_dir, analyze())).unwrap();
        let store = primed(&store_dir);
        let before = week_glance(&store, date(2026, 9, 7), utc()).unwrap();
        codex_source(&path, 160);
        execute(request(&store_dir, analyze())).unwrap();
        assert_eq!(cached_sessions(&store), Some(1));
        let after = week_glance(&store, date(2026, 9, 7), utc()).unwrap();
        assert_ne!(after.generation, before.generation);
        assert_eq!(after.rollup.sources[0].tokens, Some(60 + 10));
    }

    #[test]
    fn a_saved_reimport_recomputes_the_cached_week_before_it_returns() {
        let root = tempfile::tempdir().unwrap();
        let store_dir = root.path().join("store");
        let path = root.path().join("codex.jsonl");
        codex_source(&path, 150);
        let analyze = || LocalInsightsOperation::Analyze {
            source: SourceFormat::Codex,
            file: path.clone(),
            save: true,
        };
        execute(request(&store_dir, analyze())).unwrap();
        let store = primed(&store_dir);
        let before = week_glance(&store, date(2026, 9, 7), utc()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        execute(request(&store_dir, analyze())).unwrap();
        assert_eq!(cached_sessions(&store), Some(1));
        let after = week_glance(&store, date(2026, 9, 7), utc()).unwrap();
        assert_ne!(after.generation, before.generation);
        assert_eq!(
            after.rollup.sources[0].tokens,
            before.rollup.sources[0].tokens
        );
    }
}
