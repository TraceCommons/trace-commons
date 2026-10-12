//! Per-occurrence session markers inferred from usage counters, lettered in
//! turn order. Pure: no I/O, no clock, no copy.
//!
//! A marker that needs a counter or a gap that is unknown does not fire; an
//! unknown is never read as zero.

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{CacheWrite, TurnRecord};

use super::analytics_constants::{
    CACHE_GAP_ONE_HOUR_SECS, CACHE_GAP_SECS, CACHE_REWRITE_READ_RATIO, CACHE_REWRITE_WRITE_RATIO,
    LONG_CONTEXT_TOKENS, SHRINK_FLOOR_TOKENS, SHRINK_RATIO,
};

/// Marker kinds, in the order they are lettered within one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    /// Inferred: the cache was written again after a pause.
    CacheWrittenAgain,
    /// Inferred: context fell below half of the previous turn's.
    ContextShrank,
    /// Context reached the long-context threshold from below.
    CrossedLongContext,
    /// A file was read again with no edit tool call to it in between.
    ReRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkerDetail {
    CacheWrittenAgain {
        gap_secs: u32,
        cache_write: u64,
    },
    ContextShrank {
        from: u64,
        to: u64,
    },
    CrossedLongContext {
        context: u64,
    },
    /// `file_index` is the session's re-read file position, labelled with
    /// [`letter_label`]; no path is held.
    ReRead {
        file_index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageMarker {
    pub turn_ordinal: u32,
    pub kind: MarkerKind,
    pub detail: MarkerDetail,
}

/// A re-read found by the patterns pass, for lettering beside usage markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereadEvent {
    pub turn_ordinal: u32,
    pub file_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LetteredMarker {
    pub letter: String,
    pub marker: UsageMarker,
}

/// Markers inferred from consecutive turns of one session, one per
/// occurrence, in turn order. Re-reads come from the patterns pass and are
/// lettered with these by [`lettered_markers`].
pub fn usage_markers(turns: &[TurnRecord]) -> Vec<UsageMarker> {
    let mut markers = Vec::new();
    // The most recent nonzero cache write before the current turn, which sets
    // the cache lifetime the pause is measured against.
    let mut last_write: Option<CacheWrite> = None;
    for (index, turn) in turns.iter().enumerate() {
        let current = turn.known_usage();
        let previous = match index {
            0 => None,
            _ => Some(turns[index - 1].known_usage()),
        };
        if let Some(current) = current {
            let context = current.context();
            match previous {
                Some(Some(previous)) => {
                    let previous_context = previous.context();
                    let gap_floor = if last_write.is_some_and(CacheWrite::has_one_hour_write) {
                        CACHE_GAP_ONE_HOUR_SECS
                    } else {
                        CACHE_GAP_SECS
                    };
                    if let Some(gap_secs) = turn.gap_secs
                        && gap_secs >= gap_floor
                        && CACHE_REWRITE_READ_RATIO.exceeds(
                            u64::from(current.cache_read),
                            u64::from(previous.cache_read),
                        )
                        && CACHE_REWRITE_WRITE_RATIO
                            .reached_by(current.cache_write.total(), previous_context)
                    {
                        markers.push(UsageMarker {
                            turn_ordinal: turn.ordinal,
                            kind: MarkerKind::CacheWrittenAgain,
                            detail: MarkerDetail::CacheWrittenAgain {
                                gap_secs,
                                cache_write: current.cache_write.total(),
                            },
                        });
                    }
                    if previous_context >= SHRINK_FLOOR_TOKENS
                        && SHRINK_RATIO.exceeds(context, previous_context)
                    {
                        markers.push(UsageMarker {
                            turn_ordinal: turn.ordinal,
                            kind: MarkerKind::ContextShrank,
                            detail: MarkerDetail::ContextShrank {
                                from: previous_context,
                                to: context,
                            },
                        });
                    }
                    if previous_context < LONG_CONTEXT_TOKENS && context >= LONG_CONTEXT_TOKENS {
                        markers.push(crossed(turn.ordinal, context));
                    }
                }
                // The first turn has nothing below it to cross from.
                None if context >= LONG_CONTEXT_TOKENS => {
                    markers.push(crossed(turn.ordinal, context));
                }
                // An unknown previous turn: no comparison can be made.
                _ => {}
            }
            if current.cache_write.total() > 0 {
                last_write = Some(current.cache_write);
            }
        }
    }
    markers
}

fn crossed(turn_ordinal: u32, context: u64) -> UsageMarker {
    UsageMarker {
        turn_ordinal,
        kind: MarkerKind::CrossedLongContext,
        detail: MarkerDetail::CrossedLongContext { context },
    }
}

/// Turn ordinals at which context shrank.
pub fn shrink_turns(markers: &[UsageMarker]) -> Vec<u32> {
    markers
        .iter()
        .filter(|marker| marker.kind == MarkerKind::ContextShrank)
        .map(|marker| marker.turn_ordinal)
        .collect()
}

/// Usage markers and re-reads together, ordered by turn, then kind, then
/// file position, and lettered A, B, ... in that order.
pub fn lettered_markers(usage: &[UsageMarker], rereads: &[RereadEvent]) -> Vec<LetteredMarker> {
    let mut all: Vec<UsageMarker> = usage.to_vec();
    all.extend(rereads.iter().map(|event| UsageMarker {
        turn_ordinal: event.turn_ordinal,
        kind: MarkerKind::ReRead,
        detail: MarkerDetail::ReRead {
            file_index: event.file_index,
        },
    }));
    all.sort_by_key(|marker| {
        let file_index = match marker.detail {
            MarkerDetail::ReRead { file_index } => file_index,
            _ => 0,
        };
        (marker.turn_ordinal, marker.kind, file_index)
    });
    all.into_iter()
        .enumerate()
        .map(|(index, marker)| LetteredMarker {
            letter: letter_label(index),
            marker,
        })
        .collect()
}

/// A, B, ..., Z, AA, AB, ...: a position label that carries no name.
pub fn letter_label(index: usize) -> String {
    let mut label = Vec::new();
    let mut remaining = index + 1;
    while remaining > 0 {
        remaining -= 1;
        label.push(b'A' + (remaining % 26) as u8);
        remaining /= 26;
    }
    label.reverse();
    String::from_utf8(label).expect("ASCII letters")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use trace_commons_protocol::insights_usage_series::{CacheWrite, TurnRecord};

    /// A dated-gap turn with the given counters.
    pub(crate) fn t(
        ordinal: u32,
        uncached: u32,
        read: u32,
        write: CacheWrite,
        gap: Option<u32>,
    ) -> TurnRecord {
        TurnRecord {
            ordinal,
            at: None,
            uncached_input: Some(uncached),
            cache_read: Some(read),
            cache_write: Some(write),
            output: Some(100),
            model_label_ix: None,
            gap_secs: gap,
            msg_key: None,
        }
    }

    /// A turn whose context is `context`, all of it cache read.
    pub(crate) fn ctx(ordinal: u32, context: u32) -> TurnRecord {
        t(
            ordinal,
            0,
            context,
            CacheWrite::Split { m5: 0, h1: 0 },
            Some(10),
        )
    }

    fn unknown(ordinal: u32) -> TurnRecord {
        let mut turn = ctx(ordinal, 1);
        turn.output = None;
        turn
    }

    fn kinds(markers: &[UsageMarker]) -> Vec<(u32, MarkerKind)> {
        markers.iter().map(|m| (m.turn_ordinal, m.kind)).collect()
    }

    fn m5(n: u32) -> CacheWrite {
        CacheWrite::Split { m5: n, h1: 0 }
    }

    // Turn 0 reads 100K from cache; turn 1 after `gap` reads little and
    // writes most of the context again.
    fn rewrite_pair(gap: u32, first_write: CacheWrite) -> Vec<TurnRecord> {
        vec![
            t(0, 1_000, 100_000, first_write, None),
            t(1, 1_000, 10_000, m5(60_000), Some(gap)),
        ]
    }

    #[test]
    fn cache_rewrite_fires_at_the_300_second_gap() {
        let at = usage_markers(&rewrite_pair(CACHE_GAP_SECS, m5(5)));
        assert_eq!(kinds(&at), vec![(1, MarkerKind::CacheWrittenAgain)]);
        assert_eq!(
            at[0].detail,
            MarkerDetail::CacheWrittenAgain {
                gap_secs: 300,
                cache_write: 60_000
            }
        );
        let below = usage_markers(&rewrite_pair(CACHE_GAP_SECS - 1, m5(5)));
        assert!(below.is_empty());
    }

    #[test]
    fn a_one_hour_write_needs_the_3600_second_gap() {
        let one_hour = CacheWrite::Split { m5: 0, h1: 5 };
        assert!(usage_markers(&rewrite_pair(CACHE_GAP_ONE_HOUR_SECS - 1, one_hour)).is_empty());
        assert_eq!(
            usage_markers(&rewrite_pair(CACHE_GAP_ONE_HOUR_SECS, one_hour)).len(),
            1
        );
    }

    #[test]
    fn a_total_only_write_has_no_duration_so_300_seconds_applies() {
        let at = usage_markers(&rewrite_pair(CACHE_GAP_SECS, CacheWrite::TotalOnly(5)));
        assert_eq!(kinds(&at), vec![(1, MarkerKind::CacheWrittenAgain)]);
    }

    #[test]
    fn the_last_nonzero_write_sets_the_lifetime() {
        // An hour write, then a turn with no write: the hour rule still holds.
        let turns = vec![
            t(0, 1_000, 100_000, CacheWrite::Split { m5: 0, h1: 5 }, None),
            t(1, 1_000, 100_000, m5(0), Some(10)),
            t(2, 1_000, 10_000, m5(60_000), Some(CACHE_GAP_SECS)),
        ];
        assert!(usage_markers(&turns).is_empty());
    }

    #[test]
    fn an_unknown_gap_never_fires_a_rewrite() {
        let mut turns = rewrite_pair(CACHE_GAP_SECS, m5(5));
        turns[1].gap_secs = None;
        assert!(usage_markers(&turns).is_empty());
    }

    #[test]
    fn rewrite_needs_the_read_to_halve() {
        // cache_read 50_000 is exactly half of 100_000: not "below half".
        let turns = vec![
            t(0, 1_000, 100_000, m5(5), None),
            t(1, 1_000, 50_000, m5(60_000), Some(CACHE_GAP_SECS)),
        ];
        assert!(usage_markers(&turns).is_empty());
        let turns = vec![
            t(0, 1_000, 100_000, m5(5), None),
            t(1, 1_000, 49_999, m5(60_000), Some(CACHE_GAP_SECS)),
        ];
        assert_eq!(usage_markers(&turns).len(), 1);
    }

    #[test]
    fn rewrite_needs_the_write_to_reach_half_the_previous_context() {
        // Previous context 101_005; half is 50_502.5.
        let turns = vec![
            t(0, 1_000, 100_000, m5(5), None),
            t(1, 1_000, 10_000, m5(50_502), Some(CACHE_GAP_SECS)),
        ];
        assert!(usage_markers(&turns).is_empty());
        let turns = vec![
            t(0, 1_000, 100_000, m5(5), None),
            t(1, 1_000, 10_000, m5(50_503), Some(CACHE_GAP_SECS)),
        ];
        assert_eq!(usage_markers(&turns).len(), 1);
    }

    #[test]
    fn rewrites_fire_per_occurrence() {
        let turns = vec![
            t(0, 1_000, 100_000, m5(5), None),
            t(1, 1_000, 10_000, m5(95_000), Some(400)),
            t(2, 1_000, 100_000, m5(0), Some(5)),
            t(3, 1_000, 10_000, m5(95_000), Some(900)),
        ];
        let markers = lettered_markers(&usage_markers(&turns), &[]);
        let letters: Vec<_> = markers.iter().map(|m| m.letter.as_str()).collect();
        assert_eq!(letters, vec!["A", "B"]);
        assert_eq!(markers[0].marker.turn_ordinal, 1);
        assert_eq!(markers[1].marker.turn_ordinal, 3);
    }

    #[test]
    fn crossing_long_context_fires_on_each_upward_crossing() {
        let limit = LONG_CONTEXT_TOKENS as u32;
        let turns = vec![
            ctx(0, limit - 1),
            ctx(1, limit),
            ctx(2, limit + 5),
            ctx(3, 10_000),
            ctx(4, limit + 1),
        ];
        let markers = usage_markers(&turns);
        let crossed: Vec<_> = kinds(&markers)
            .into_iter()
            .filter(|(_, kind)| *kind == MarkerKind::CrossedLongContext)
            .collect();
        assert_eq!(
            crossed,
            vec![
                (1, MarkerKind::CrossedLongContext),
                (4, MarkerKind::CrossedLongContext)
            ]
        );
    }

    #[test]
    fn a_first_turn_already_long_counts_as_crossing() {
        let markers = usage_markers(&[ctx(0, LONG_CONTEXT_TOKENS as u32)]);
        assert_eq!(kinds(&markers), vec![(0, MarkerKind::CrossedLongContext)]);
    }

    #[test]
    fn crossing_after_an_unknown_turn_cannot_be_told() {
        let turns = vec![unknown(0), ctx(1, LONG_CONTEXT_TOKENS as u32)];
        assert!(usage_markers(&turns).is_empty());
    }

    #[test]
    fn context_shrank_needs_half_and_the_floor() {
        let floor = SHRINK_FLOOR_TOKENS as u32;
        let fires = usage_markers(&[ctx(0, floor), ctx(1, floor / 2 - 1)]);
        assert_eq!(
            kinds(&fires),
            vec![(1, MarkerKind::ContextShrank)],
            "{fires:?}"
        );
        // Exactly half is not below half.
        assert!(usage_markers(&[ctx(0, floor), ctx(1, floor / 2)]).is_empty());
        // Below the floor nothing shrinks.
        assert!(usage_markers(&[ctx(0, floor - 1), ctx(1, 1)]).is_empty());
    }

    #[test]
    fn shrink_turns_lists_only_shrinks() {
        let turns = vec![
            ctx(0, 100_000),
            ctx(1, 10_000),
            ctx(2, 300_000),
            ctx(3, 20_000),
        ];
        assert_eq!(shrink_turns(&usage_markers(&turns)), vec![1, 3]);
    }

    #[test]
    fn rereads_are_lettered_with_usage_markers_in_turn_order() {
        let turns = vec![ctx(0, 100_000), ctx(1, 100_000), ctx(2, 10_000)];
        let usage = usage_markers(&turns);
        let rereads = [
            RereadEvent {
                turn_ordinal: 2,
                file_index: 1,
            },
            RereadEvent {
                turn_ordinal: 1,
                file_index: 0,
            },
            RereadEvent {
                turn_ordinal: 2,
                file_index: 0,
            },
        ];
        let lettered = lettered_markers(&usage, &rereads);
        let summary: Vec<_> = lettered
            .iter()
            .map(|m| (m.letter.as_str(), m.marker.turn_ordinal, m.marker.detail))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("A", 1, MarkerDetail::ReRead { file_index: 0 }),
                (
                    "B",
                    2,
                    MarkerDetail::ContextShrank {
                        from: 100_000,
                        to: 10_000
                    }
                ),
                ("C", 2, MarkerDetail::ReRead { file_index: 0 }),
                ("D", 2, MarkerDetail::ReRead { file_index: 1 }),
            ]
        );
    }

    #[test]
    fn letter_labels_continue_past_z() {
        assert_eq!(letter_label(0), "A");
        assert_eq!(letter_label(25), "Z");
        assert_eq!(letter_label(26), "AA");
        assert_eq!(letter_label(27), "AB");
        assert_eq!(letter_label(26 + 26 * 26), "AAA");
    }
}
