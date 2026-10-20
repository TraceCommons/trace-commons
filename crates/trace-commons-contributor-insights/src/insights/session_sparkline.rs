//! A Sessions row's sparkline: one session's tokens per turn, folded into a
//! short fixed-length series a shell can draw in a row without a second
//! read.
//!
//! Integers only, from the stored counters: no text, path or digest. A turn's
//! tokens are every token it reported (uncached input, cache read, cache
//! write and output), as the drill's header total counts them.
//!
//! - At most [`SPARKLINE_POINTS`] points. A session with that many turns or
//!   fewer has one point per turn; a longer one is cut into that many
//!   consecutive runs of turns, as even as integer division makes them, and
//!   each point is the largest turn in its run, so a spike is never averaged
//!   away.
//! - A turn whose counters are unknown counts as 0 in its point (a gap on a
//!   line this small reads as a dip, not a missing bar). A session with no
//!   known turn has no sparkline at all: the row says it has no counts.
//! - The peak is read from the turns themselves, never from the points: its
//!   tokens and the turn's own ordinal, the first such turn on a tie, so "peak
//!   N tokens at turn M" is literally true.

use trace_commons_protocol::insights_usage_series::TurnRecord;

/// The most points one sparkline carries.
pub const SPARKLINE_POINTS: usize = 32;

/// One session's tokens per turn, folded for a row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionSparkline {
    /// Tokens per point, oldest first; at most [`SPARKLINE_POINTS`].
    pub points: Vec<u64>,
    /// The largest one turn's tokens.
    pub peak_tokens: u64,
    /// That turn's ordinal, as the drill numbers it.
    pub peak_turn: u32,
}

/// The known turns' tokens together; `None` when no turn is known, never 0.
pub fn known_tokens(turns: &[TurnRecord]) -> Option<u64> {
    let mut any = false;
    let mut total = 0u64;
    for usage in turns.iter().filter_map(TurnRecord::known_usage) {
        any = true;
        total = total.saturating_add(usage.total());
    }
    any.then_some(total)
}

/// The sparkline for `turns`; `None` when no turn's counters are known.
pub fn session_sparkline(turns: &[TurnRecord]) -> Option<SessionSparkline> {
    let tokens: Vec<Option<u64>> = turns
        .iter()
        .map(|turn| turn.known_usage().map(|usage| usage.total()))
        .collect();
    let mut peak: Option<(u64, u32)> = None;
    for (turn, value) in turns.iter().zip(&tokens) {
        if let Some(value) = *value {
            if peak.is_none_or(|(best, _)| value > best) {
                peak = Some((value, turn.ordinal));
            }
        }
    }
    let (peak_tokens, peak_turn) = peak?;
    let count = tokens.len();
    let buckets = count.min(SPARKLINE_POINTS);
    let points = (0..buckets)
        .map(|i| {
            let start = i * count / buckets;
            let end = (i + 1) * count / buckets;
            tokens[start..end]
                .iter()
                .map(|value| value.unwrap_or(0))
                .max()
                .unwrap_or(0)
        })
        .collect();
    Some(SessionSparkline {
        points,
        peak_tokens,
        peak_turn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_protocol::insights_usage_series::CacheWrite;

    fn turn(ordinal: u32, tokens: Option<u32>) -> TurnRecord {
        TurnRecord {
            ordinal,
            at: None,
            uncached_input: tokens.map(|_| 1),
            cache_read: tokens.map(|_| 0),
            cache_write: tokens.map(|_| CacheWrite::Split { m5: 0, h1: 0 }),
            output: tokens.map(|t| t - 1),
            model_label_ix: None,
            gap_secs: None,
            msg_key: None,
        }
    }

    fn turns(tokens: &[Option<u32>]) -> Vec<TurnRecord> {
        tokens
            .iter()
            .enumerate()
            .map(|(i, t)| turn(i as u32 + 1, *t))
            .collect()
    }

    #[test]
    fn no_turns_or_no_known_turn_has_no_sparkline() {
        assert_eq!(session_sparkline(&[]), None);
        assert_eq!(known_tokens(&[]), None);
        let unknown = turns(&[None, None, None]);
        assert_eq!(session_sparkline(&unknown), None);
        assert_eq!(known_tokens(&unknown), None, "unknown is never 0");
    }

    #[test]
    fn fewer_turns_than_points_is_one_point_per_turn() {
        let series = turns(&[Some(10), Some(300), Some(20)]);
        assert_eq!(
            session_sparkline(&series),
            Some(SessionSparkline {
                points: vec![10, 300, 20],
                peak_tokens: 300,
                peak_turn: 2,
            })
        );
        assert_eq!(known_tokens(&series), Some(330));
    }

    #[test]
    fn an_unknown_turn_is_a_zero_point_and_never_the_peak() {
        let series = turns(&[Some(10), None, Some(5)]);
        let spark = session_sparkline(&series).unwrap();
        assert_eq!(spark.points, vec![10, 0, 5]);
        assert_eq!((spark.peak_tokens, spark.peak_turn), (10, 1));
        assert_eq!(known_tokens(&series), Some(15));
    }

    #[test]
    fn exactly_the_point_count_is_still_one_point_per_turn() {
        let tokens: Vec<Option<u32>> = (1..=SPARKLINE_POINTS as u32).map(Some).collect();
        let spark = session_sparkline(&turns(&tokens)).unwrap();
        assert_eq!(
            spark.points,
            (1..=SPARKLINE_POINTS as u64).collect::<Vec<_>>()
        );
    }

    #[test]
    fn many_turns_fold_into_the_point_count_by_each_runs_largest_turn() {
        // 100 turns: tokens rise with the turn, with one spike at turn 51.
        let mut tokens: Vec<Option<u32>> = (1..=100).map(Some).collect();
        tokens[50] = Some(9_000);
        let spark = session_sparkline(&turns(&tokens)).unwrap();
        assert_eq!(spark.points.len(), SPARKLINE_POINTS);
        // Runs are [i*100/32, (i+1)*100/32): the first is turns 1..=3, the
        // last turns 97..=100.
        assert_eq!(spark.points[0], 3);
        assert_eq!(spark.points[SPARKLINE_POINTS - 1], 100);
        assert!(
            spark.points.contains(&9_000),
            "a spike is never averaged away"
        );
        assert_eq!((spark.peak_tokens, spark.peak_turn), (9_000, 51));
        // Every turn lands in exactly one run, in order.
        assert!(spark.points.windows(2).filter(|w| w[0] > w[1]).count() <= 1);
    }

    #[test]
    fn the_peak_is_the_first_largest_turn_by_its_own_ordinal() {
        let series = vec![turn(4, Some(50)), turn(9, Some(70)), turn(12, Some(70))];
        let spark = session_sparkline(&series).unwrap();
        assert_eq!((spark.peak_tokens, spark.peak_turn), (70, 9));
    }
}
