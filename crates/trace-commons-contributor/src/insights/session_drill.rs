//! Sessions (drill-in): one saved session turn by turn, with its lettered
//! markers.
//!
//! The header is the session's own first and last recorded event, its turn
//! count and its tokens. The span is the time between the first and last
//! event, never called active time. No project name is carried (owner
//! decision D7, open).
//!
//! The chart rows are the per-turn counters. A turn with any counter unknown
//! is unknown as a whole and is never drawn as zero. Markers fire once per
//! occurrence and are lettered in turn order; a re-read names its file only
//! by a letter and the extension (owner decision D8, open). No keyed digest,
//! message key or path leaves this module.
//!
//! Codex records no per-turn series that Insights accepts, so a Codex
//! session reads "not recorded" (owner decision D11, open:
//! `CODEX_PER_TURN_SERIES`). The what-if estimate is not computed here and
//! is not in the response (owner decision D2, open: `ADVICE_SHOWN`).

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use trace_commons_protocol::insights_usage_series::UsageSeries;

use super::LocalInsight;
use super::analytics_constants::LONG_CONTEXT_TOKENS;
use super::markers::{MarkerDetail, letter_label, lettered_markers, usage_markers};
use super::patterns::session_patterns;
use super::week_glance::saved_sessions;
use super::week_rollup::{
    AnalyticsSource, CoverageReason, CoverageState, Feed, SessionBody, UnknownReason,
};

/// Why a session has no per-turn chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesUnavailable {
    /// Codex: turn-by-turn usage is not recorded (owner decision D11, open).
    NotRecorded,
    /// Saved before per-turn counters were kept, or none were found:
    /// unknown until reimported.
    NoUsageCounters,
    /// The file format has no usage counters.
    SourceUnsupported,
}

/// How a marker was arrived at, for its derivation label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerBasis {
    /// "Inferred from usage counters."
    InferredFromCounters,
    /// "from counters".
    FromCounters,
    /// From the order of tool calls.
    FromToolCalls,
}

/// One turn of the chart. Every counter is `None` when any of the turn's
/// four counters is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTurn {
    pub ordinal: u32,
    pub uncached: Option<u32>,
    pub cache_read: Option<u32>,
    /// Both cache lifetimes together.
    pub cache_write: Option<u64>,
    pub output: Option<u32>,
    /// Input sent this turn: uncached + cache read + cache write.
    pub context: Option<u64>,
}

/// One lettered marker. Only the fields of its kind are present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillMarker {
    pub letter: String,
    pub turn_ordinal: u32,
    pub kind: super::markers::MarkerKind,
    pub basis: MarkerBasis,
    /// Cache written again: the pause before the turn, in whole minutes,
    /// rounded half up.
    pub pause_minutes: Option<u32>,
    /// Cache written again: tokens written to the cache on the turn.
    pub cache_write: Option<u64>,
    /// Context shrank: the context before the turn.
    pub context_from: Option<u64>,
    /// Context shrank or crossed: the context on the turn.
    pub context: Option<u64>,
    /// Re-read: the file's position label within the session.
    pub file_letter: Option<String>,
    /// Re-read: the extension with its dot (".rs"), when it has one.
    pub file_ext: Option<String>,
}

/// The Sessions tab for one saved session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDrill {
    pub feed: Feed,
    /// The snapshot ID. Never a path.
    pub session_ref: String,
    pub source: Option<AnalyticsSource>,
    /// Local date of the first recorded event.
    pub date: Option<NaiveDate>,
    pub tz: i32,
    /// Turns in the series. `None` when no series is recorded.
    pub turns: Option<u32>,
    /// Claude: the known turns' tokens; Codex: the change between the first
    /// and last counter. `None` when unknown, never zero.
    pub tokens: Option<u64>,
    /// Seconds between the first and last recorded event.
    pub span_secs: Option<u64>,
    pub state: CoverageState,
    pub reasons: Vec<CoverageReason>,
    /// Where the dashed long-context line is drawn.
    pub long_context_threshold: u64,
    /// `None` with `series_unavailable` saying why.
    pub series: Option<Vec<DrillTurn>>,
    pub series_unavailable: Option<SeriesUnavailable>,
    pub markers: Vec<DrillMarker>,
}

/// The drill-in over one session's counter rows. `first_at` and `last_at`
/// are its first and last recorded events.
pub fn session_drill(
    session_ref: &str,
    body: &SessionBody,
    first_at: Option<DateTime<Utc>>,
    last_at: Option<DateTime<Utc>>,
    tz: FixedOffset,
) -> SessionDrill {
    let mut reasons: Vec<CoverageReason> = Vec::new();
    let mut drill = SessionDrill {
        feed: Feed::Saved,
        session_ref: session_ref.to_string(),
        source: None,
        date: first_at.map(|at| at.with_timezone(&tz).date_naive()),
        tz: tz.local_minus_utc(),
        turns: None,
        tokens: None,
        span_secs: match (first_at, last_at) {
            (Some(first), Some(last)) => u64::try_from((last - first).num_seconds()).ok(),
            _ => None,
        },
        state: CoverageState::Known,
        reasons: Vec::new(),
        long_context_threshold: LONG_CONTEXT_TOKENS,
        series: None,
        series_unavailable: None,
        markers: Vec::new(),
    };
    match body {
        SessionBody::Claude { series, .. } => {
            drill.source = Some(AnalyticsSource::ClaudeCode);
            drill.turns = Some(series.turns.len() as u32);
            let mut known = 0u32;
            let mut unknown = 0u32;
            let mut tokens = 0u64;
            let rows = series
                .turns
                .iter()
                .map(|turn| match turn.known_usage() {
                    Some(usage) => {
                        known += 1;
                        tokens += usage.total();
                        DrillTurn {
                            ordinal: turn.ordinal,
                            uncached: Some(usage.uncached_input),
                            cache_read: Some(usage.cache_read),
                            cache_write: Some(usage.cache_write.total()),
                            output: Some(usage.output),
                            context: Some(usage.context()),
                        }
                    }
                    None => {
                        unknown += 1;
                        DrillTurn {
                            ordinal: turn.ordinal,
                            uncached: None,
                            cache_read: None,
                            cache_write: None,
                            output: None,
                            context: None,
                        }
                    }
                })
                .collect();
            if known == 0 {
                reasons.push(CoverageReason::NoUsageCounters);
            } else {
                drill.tokens = Some(tokens);
                if unknown > 0 {
                    reasons.push(CoverageReason::SomeTurnsUnknown);
                }
            }
            if series.truncated {
                reasons.push(CoverageReason::Truncated);
            }
            drill.series = Some(rows);
            drill.markers = lettered(series);
        }
        SessionBody::Codex { observed } => {
            drill.source = Some(AnalyticsSource::Codex);
            // Owner decision D11, open: Codex `last_token_usage` is not
            // accepted as a per-turn series, so there is no chart.
            drill.series_unavailable = Some(SeriesUnavailable::NotRecorded);
            match observed {
                Some(observed) => {
                    drill.tokens = Some(observed.total);
                    if observed.baseline_excluded {
                        reasons.push(CoverageReason::CodexBaselineExcluded);
                    }
                }
                None => reasons.push(CoverageReason::NoUsageCounters),
            }
        }
        SessionBody::Unknown { source, reason } => {
            drill.source = *source;
            drill.series_unavailable = Some(unknown_series(*reason));
            reasons.push((*reason).into());
        }
    }
    reasons.sort();
    reasons.dedup();
    drill.state = reasons
        .iter()
        .map(|reason| reason.state())
        .max()
        .unwrap_or(CoverageState::Known);
    drill.reasons = reasons;
    drill
}

/// Usage markers and re-reads, lettered together in turn order, with each
/// re-read's file named by its position letter and extension only.
fn lettered(series: &UsageSeries) -> Vec<DrillMarker> {
    let found = session_patterns(series);
    lettered_markers(&usage_markers(&series.turns), &found.reread_events)
        .into_iter()
        .map(|lettered| {
            let marker = lettered.marker;
            let mut drill = DrillMarker {
                letter: lettered.letter,
                turn_ordinal: marker.turn_ordinal,
                kind: marker.kind,
                basis: MarkerBasis::InferredFromCounters,
                pause_minutes: None,
                cache_write: None,
                context_from: None,
                context: None,
                file_letter: None,
                file_ext: None,
            };
            match marker.detail {
                MarkerDetail::CacheWrittenAgain {
                    gap_secs,
                    cache_write,
                } => {
                    drill.pause_minutes = Some(pause_minutes(gap_secs));
                    drill.cache_write = Some(cache_write);
                }
                MarkerDetail::ContextShrank { from, to } => {
                    drill.context_from = Some(from);
                    drill.context = Some(to);
                }
                MarkerDetail::CrossedLongContext { context } => {
                    drill.basis = MarkerBasis::FromCounters;
                    drill.context = Some(context);
                }
                MarkerDetail::ReRead { file_index } => {
                    drill.basis = MarkerBasis::FromToolCalls;
                    drill.file_letter = Some(letter_label(file_index));
                    drill.file_ext = found
                        .reread_files
                        .get(file_index)
                        .and_then(|file| file.path_ext.as_ref())
                        .map(|ext| format!(".{ext}"));
                }
            }
            drill
        })
        .collect()
}

/// Feed S: the drill-in for one saved snapshot.
pub fn saved_session_drill(insight: &LocalInsight, tz: FixedOffset) -> SessionDrill {
    let input = saved_sessions(std::slice::from_ref(insight))
        .pop()
        .expect("one session per snapshot");
    let extremum = |pick: fn(
        &super::time_evidence::RecordedTimeEvidence,
    ) -> Option<&super::time_evidence::TimestampExtremum>| {
        insight
            .time_evidence
            .as_ref()
            .and_then(pick)
            .map(|extremum| extremum.recorded_at)
    };
    session_drill(
        &input.session_ref,
        &input.body,
        extremum(|evidence| evidence.earliest.as_ref()),
        extremum(|evidence| evidence.latest.as_ref()),
        tz,
    )
}

/// Whole minutes, rounded half up.
fn pause_minutes(gap_secs: u32) -> u32 {
    (gap_secs + 30) / 60
}

fn unknown_series(reason: UnknownReason) -> SeriesUnavailable {
    match reason {
        UnknownReason::SourceUnsupported => SeriesUnavailable::SourceUnsupported,
        _ => SeriesUnavailable::NoUsageCounters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::markers::MarkerKind;
    use crate::insights::markers::tests::{ctx, t};
    use crate::insights::week_rollup::CodexObserved;
    use chrono::TimeZone;
    use trace_commons_protocol::insights_usage_series::{
        CacheWrite, KeyedDigest, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
    };

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 3, hour, minute, 0).unwrap()
    }

    fn read(turn: u32, path: u8, ext: Option<&str>) -> ToolCallRecord {
        ToolCallRecord {
            turn_ordinal: turn,
            tool: ToolKind::Read,
            args_key: KeyedDigest([path; 32]),
            path_key: Some(KeyedDigest([path; 32])),
            path_ext: ext.map(str::to_string),
            result_bytes: Some(4_000),
            success: Some(true),
            paired: true,
        }
    }

    fn claude(turns: Vec<TurnRecord>, calls: Vec<ToolCallRecord>) -> SessionBody {
        SessionBody::Claude {
            series: UsageSeries {
                turns,
                tool_calls: calls,
                truncated: false,
            },
            model_labels: vec![],
        }
    }

    /// Two cache rewrites after pauses (turns 3 and 5), a long-context
    /// crossing (turn 6), and file 7 read again on turn 4.
    fn rewritten_twice() -> SessionBody {
        let none = CacheWrite::Split { m5: 0, h1: 0 };
        let mut turns = vec![
            t(1, 1_000, 0, CacheWrite::Split { m5: 60_000, h1: 0 }, None),
            t(2, 1_000, 60_000, none, Some(10)),
            t(
                3,
                1_000,
                0,
                CacheWrite::Split { m5: 62_000, h1: 0 },
                Some(840),
            ),
            t(4, 1_000, 62_000, none, Some(10)),
            t(
                5,
                1_000,
                0,
                CacheWrite::Split { m5: 66_000, h1: 0 },
                Some(600),
            ),
            ctx(6, 210_000),
        ];
        for (index, turn) in turns.iter_mut().enumerate() {
            let mut key = [0u8; 32];
            key[0] = index as u8 + 1;
            turn.msg_key = Some(KeyedDigest(key));
        }
        claude(turns, vec![read(2, 7, Some("rs")), read(4, 7, Some("rs"))])
    }

    #[test]
    fn markers_fire_per_occurrence_and_are_lettered_in_turn_order() {
        let drill = session_drill("snap", &rewritten_twice(), None, None, utc());
        let shape: Vec<(&str, u32, MarkerKind)> = drill
            .markers
            .iter()
            .map(|marker| (marker.letter.as_str(), marker.turn_ordinal, marker.kind))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("A", 3, MarkerKind::CacheWrittenAgain),
                ("B", 4, MarkerKind::ReRead),
                ("C", 5, MarkerKind::CacheWrittenAgain),
                ("D", 6, MarkerKind::CrossedLongContext),
            ]
        );
        let first = &drill.markers[0];
        assert_eq!(first.basis, MarkerBasis::InferredFromCounters);
        // 840 seconds is 14 minutes; 600 is 10.
        assert_eq!(first.pause_minutes, Some(14));
        assert_eq!(first.cache_write, Some(62_000));
        assert_eq!(drill.markers[2].pause_minutes, Some(10));
        let reread = &drill.markers[1];
        assert_eq!(reread.basis, MarkerBasis::FromToolCalls);
        assert_eq!(reread.file_letter.as_deref(), Some("A"));
        assert_eq!(reread.file_ext.as_deref(), Some(".rs"));
        let crossed = &drill.markers[3];
        assert_eq!(crossed.basis, MarkerBasis::FromCounters);
        assert_eq!(crossed.context, Some(210_000));
        assert_eq!(drill.long_context_threshold, 200_000);
    }

    #[test]
    fn a_pause_rounds_half_up_to_whole_minutes() {
        assert_eq!(pause_minutes(300), 5);
        assert_eq!(pause_minutes(329), 5);
        assert_eq!(pause_minutes(330), 6);
    }

    #[test]
    fn the_header_is_the_sessions_own_events_and_its_known_turns() {
        let drill = session_drill(
            "snap",
            &rewritten_twice(),
            Some(at(9, 0)),
            Some(at(11, 10)),
            FixedOffset::east_opt(-10 * 3_600).unwrap(),
        );
        assert_eq!(drill.source, Some(AnalyticsSource::ClaudeCode));
        // 09:00 UTC is the day before in UTC-10.
        assert_eq!(drill.date, NaiveDate::from_ymd_opt(2026, 10, 2));
        assert_eq!(drill.tz, -36_000);
        assert_eq!(drill.span_secs, Some(7_800));
        assert_eq!(drill.turns, Some(6));
        let series = drill.series.as_ref().unwrap();
        assert_eq!(series.len(), 6);
        assert_eq!(series[0].cache_write, Some(60_000));
        assert_eq!(series[0].context, Some(61_000));
        let expected: u64 = series
            .iter()
            .map(|turn| turn.context.unwrap() + u64::from(turn.output.unwrap()))
            .sum();
        assert_eq!(drill.tokens, Some(expected));
        assert_eq!(drill.state, CoverageState::Known);
        assert!(drill.reasons.is_empty());
        assert_eq!(drill.series_unavailable, None);
    }

    #[test]
    fn an_unknown_turn_is_unknown_never_zero_and_marks_the_session_partial() {
        let mut gap = ctx(2, 100);
        gap.cache_read = None;
        let body = claude(vec![ctx(1, 100), gap, ctx(3, 100)], vec![]);
        let drill = session_drill("snap", &body, None, None, utc());
        let series = drill.series.unwrap();
        assert_eq!(series[1].uncached, None);
        assert_eq!(series[1].cache_read, None);
        assert_eq!(series[1].cache_write, None);
        assert_eq!(series[1].output, None);
        assert_eq!(series[1].context, None);
        assert_eq!(drill.tokens, Some(400));
        assert_eq!(drill.state, CoverageState::Partial);
        assert_eq!(drill.reasons, vec![CoverageReason::SomeTurnsUnknown]);
        // No span without both events.
        assert_eq!(drill.span_secs, None);
        assert_eq!(drill.date, None);

        let mut only = ctx(1, 100);
        only.output = None;
        let drill = session_drill("snap", &claude(vec![only], vec![]), None, None, utc());
        assert_eq!(drill.tokens, None);
        assert_eq!(drill.state, CoverageState::Unknown);
        assert_eq!(drill.reasons, vec![CoverageReason::NoUsageCounters]);
    }

    #[test]
    fn a_truncated_series_is_partial() {
        let SessionBody::Claude { mut series, .. } = claude(vec![ctx(1, 100)], vec![]) else {
            unreachable!()
        };
        series.truncated = true;
        let body = SessionBody::Claude {
            series,
            model_labels: vec![],
        };
        let drill = session_drill("snap", &body, None, None, utc());
        assert_eq!(drill.state, CoverageState::Partial);
        assert_eq!(drill.reasons, vec![CoverageReason::Truncated]);
    }

    #[test]
    fn codex_reads_not_recorded_with_its_observed_change() {
        // Owner decision D11, open (`CODEX_PER_TURN_SERIES`): no Codex
        // per-turn series.
        let body = SessionBody::Codex {
            observed: Some(CodexObserved {
                first_at: at(9, 0),
                last_at: at(9, 30),
                input: 900,
                cached_input: 400,
                output: 100,
                total: 1_000,
                baseline_excluded: true,
            }),
        };
        let drill = session_drill("snap", &body, Some(at(9, 0)), Some(at(9, 30)), utc());
        assert_eq!(drill.source, Some(AnalyticsSource::Codex));
        assert_eq!(drill.series, None);
        assert_eq!(
            drill.series_unavailable,
            Some(SeriesUnavailable::NotRecorded)
        );
        assert_eq!(drill.turns, None);
        assert!(drill.markers.is_empty());
        assert_eq!(drill.tokens, Some(1_000));
        assert_eq!(drill.span_secs, Some(1_800));
        assert_eq!(drill.state, CoverageState::Partial);
        assert_eq!(drill.reasons, vec![CoverageReason::CodexBaselineExcluded]);

        let drill = session_drill(
            "snap",
            &SessionBody::Codex { observed: None },
            None,
            None,
            utc(),
        );
        assert_eq!(drill.tokens, None);
        assert_eq!(
            drill.series_unavailable,
            Some(SeriesUnavailable::NotRecorded)
        );
        assert_eq!(drill.state, CoverageState::Unknown);
        assert_eq!(drill.reasons, vec![CoverageReason::NoUsageCounters]);
    }

    #[test]
    fn a_session_without_counters_is_typed_unknown() {
        for (reason, expected) in [
            (
                UnknownReason::NoUsageCounters,
                SeriesUnavailable::NoUsageCounters,
            ),
            (
                UnknownReason::SourceUnsupported,
                SeriesUnavailable::SourceUnsupported,
            ),
        ] {
            let body = SessionBody::Unknown {
                source: None,
                reason,
            };
            let drill = session_drill("snap", &body, None, None, utc());
            assert_eq!(drill.series, None);
            assert_eq!(drill.series_unavailable, Some(expected));
            assert_eq!(drill.tokens, None);
            assert_eq!(drill.state, CoverageState::Unknown);
            assert_eq!(drill.reasons, vec![CoverageReason::from(reason)]);
        }
    }

    #[test]
    fn no_digest_or_what_if_reaches_the_wire() {
        let drill = session_drill("snap", &rewritten_twice(), None, None, utc());
        let wire = serde_json::to_string(&drill).unwrap();
        // Owner decision D2, open (`ADVICE_SHOWN`): no what-if field at all.
        for private in ["msg_key", "args_key", "path_key", "what_if", "07070707"] {
            assert!(!wire.contains(private), "{private}");
        }
    }
}
