//! Feed T comparisons for the Insights window: the lever of the week and the
//! weekly summary card. The threshold goals marked here left Insights
//! (product owner, 2026-10-29); Goals are now the `/goal` goals the daemon's
//! `insights_week` answer carries.
//!
//! The figures are the daemon's (`insights_week`'s `history`), passed through
//! by the shell: the lever's feedback lives in the in-process store,
//! which the daemon never reads, and the counter rows live in the daemon,
//! which this process never reads. Nothing here takes an assumption from the
//! caller; the figures are measured counts and every rule is a core constant.
//!
//! Under feed S nothing is compared: there is no lever and no summary card
//! (owner decision D4, open). The lever is an observation only (owner
//! decision D2, open), and the summary card is in the window only, with no
//! notification (owner decision D1, open).

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use super::cache_share::best_week_permille;
use super::goal_store::GoalState;
use super::goals::{ThresholdCount, WeekFigures};
use super::lever::{kinds_off, lever};
use super::patterns::PatternKind;
use super::week_rollup::{AnalyticsSource, Feed, Unavailable, change_permille};

/// Weeks a request may pass. A bound on the request, not an analytics
/// constant: the daemon sends at most its kept weeks.
pub const MAX_COUNTER_WEEKS: usize = 64;
/// Items the summary card lists, one per rule.
pub const MAX_RECAP_ITEMS: usize = 3;

/// The lever's kind and the figures its line names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverView {
    pub kind: PatternKind,
    /// The kind's token figure this week.
    pub tokens: u64,
    /// Occurrences: reads, calls, cycles or long-context turns.
    pub count: Option<u32>,
    /// Repeated reads only: files read again.
    pub files: Option<u32>,
    /// This week against the median of its recent comparable weeks.
    pub ratio_permille: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeverState {
    pub pick: Option<LeverView>,
    /// Kinds off after repeated "Not useful", until turned back on.
    pub disabled: Vec<PatternKind>,
    pub unavailable: Option<Unavailable>,
    /// The Settings switch for the lever's card (`analytics_setting_lever`):
    /// false while it is switched off, or while any kind is off after
    /// repeated "Not useful", whatever the weekly history. Turning it on is
    /// the `lever_card` operation. Switched off, no card is drawn and
    /// `disabled` is empty.
    pub card_enabled: bool,
}

/// One harness's line on the summary card. Never summed with another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecapSource {
    pub source: AnalyticsSource,
    pub tokens: u64,
    /// Against the week before; `None` when that week is not comparable.
    pub change_permille: Option<i64>,
}

/// One item on the summary card, by its fixed rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecapItem {
    /// "Best cache week yet: {p}%. Previous best {q}%."
    BestCacheWeek {
        source: AnalyticsSource,
        permille: u64,
        previous_best_permille: u64,
    },
    /// "{pattern} went up {p}% against your usual week."
    PatternUp {
        pattern: PatternKind,
        /// The rise over the usual week, per mille.
        up_permille: u64,
    },
}

/// The weekly summary card for the last closed week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recap {
    pub week_start: NaiveDate,
    pub week_end: NaiveDate,
    pub sessions: u32,
    pub sources: Vec<RecapSource>,
    /// At most [`MAX_RECAP_ITEMS`], in rule order.
    pub items: Vec<RecapItem>,
    /// Only when the user has set a context threshold.
    pub past_threshold: Option<ThresholdCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekComparisons {
    pub feed: Feed,
    pub week_start: NaiveDate,
    pub lever: LeverState,
    /// Present only on the first opens after a week closes, while the card
    /// is switched on.
    pub recap: Option<Recap>,
}

/// What the request carries besides the stored state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparisonsRequest {
    /// The Monday of the week on screen.
    pub week_start: NaiveDate,
    /// The Monday of the local week now in progress.
    pub current_week: NaiveDate,
    /// The daemon's `insights_recap_card_enabled`.
    pub recap_card_enabled: bool,
}

fn figures_until(weeks: &[WeekFigures], week_start: NaiveDate) -> &[WeekFigures] {
    let end = weeks.partition_point(|week| week.week_start <= week_start);
    &weeks[..end]
}

fn lever_state(
    feed: Feed,
    weeks: &[WeekFigures],
    week_start: NaiveDate,
    state: &GoalState,
) -> LeverState {
    if state.lever_card_off {
        return LeverState {
            pick: None,
            disabled: Vec::new(),
            unavailable: None,
            card_enabled: false,
        };
    }
    let card_enabled = kinds_off(&state.lever_dismissals, &state.lever_reenables).is_empty();
    let shown = figures_until(weeks, week_start);
    let outcome = match shown.split_last() {
        _ if feed != Feed::CounterPass => Err(Unavailable::NeedsCounterPass),
        Some((this, earlier)) if this.week_start == week_start => lever(
            feed,
            this,
            earlier,
            &state.lever_dismissals,
            &state.lever_reenables,
        )
        .map(|outcome| (this, outcome)),
        _ => Err(Unavailable::InsufficientHistory),
    };
    match outcome {
        Ok((this, outcome)) => LeverState {
            pick: outcome.pick.map(|pick| LeverView {
                kind: pick.kind,
                tokens: pick.figure,
                count: this.pattern_counts.get(&pick.kind).copied(),
                files: (pick.kind == PatternKind::RepeatedReads)
                    .then_some(this.reread_files)
                    .flatten(),
                ratio_permille: pick.ratio_permille,
            }),
            disabled: outcome.disabled,
            unavailable: None,
            card_enabled,
        },
        Err(reason) => LeverState {
            pick: None,
            disabled: Vec::new(),
            unavailable: Some(reason),
            card_enabled,
        },
    }
}

fn recap(weeks: &[WeekFigures], request: ComparisonsRequest, state: &GoalState) -> Option<Recap> {
    let closed = request.current_week - Duration::weeks(1);
    if !request.recap_card_enabled || state.recap_opened_week >= Some(closed) {
        return None;
    }
    let shown = figures_until(weeks, closed);
    let (this, earlier) = shown.split_last()?;
    if this.week_start != closed || !this.comparable {
        return None;
    }
    let last = earlier
        .last()
        .filter(|week| week.week_start == closed - Duration::weeks(1) && week.comparable);
    let sources = this
        .tokens
        .iter()
        .map(|(source, tokens)| RecapSource {
            source: *source,
            tokens: *tokens,
            change_permille: last
                .and_then(|last| last.tokens.get(source))
                .and_then(|before| change_permille(*tokens, *before)),
        })
        .collect();

    let mut items = Vec::new();
    // Rule 1: a new best cache week, first source in fixed order.
    for (source, permille) in &this.cache_share_permille {
        let history: Vec<Option<u64>> = earlier
            .iter()
            .map(|week| {
                week.comparable
                    .then(|| week.cache_share_permille.get(source).copied())
                    .flatten()
            })
            .collect();
        if let Ok(best) = best_week_permille(*permille, &history)
            && best.is_new_best
        {
            items.push(RecapItem::BestCacheWeek {
                source: *source,
                permille: *permille,
                previous_best_permille: best.previous_best_permille,
            });
            break;
        }
    }
    // Rule 2: the pattern furthest above its usual week, by the lever's rule,
    // and not while the lever's card is switched off in Settings: the same
    // observation, as a kind's "Not useful" already leaves it out here.
    if !state.lever_card_off
        && let Ok(outcome) = lever(
            Feed::CounterPass,
            this,
            earlier,
            &state.lever_dismissals,
            &state.lever_reenables,
        )
        && let Some(pick) = outcome.pick
    {
        items.push(RecapItem::PatternUp {
            pattern: pick.kind,
            up_permille: pick.ratio_permille.saturating_sub(1_000),
        });
    }
    items.truncate(MAX_RECAP_ITEMS);
    Some(Recap {
        week_start: closed,
        week_end: closed + Duration::days(6),
        sessions: this.sessions,
        sources,
        items,
        past_threshold: this.past_threshold,
    })
}

/// The lever and the summary card. `counter_weeks` are the daemon's
/// weekly figures, oldest first; `None` under feed S.
pub fn week_comparisons(
    counter_weeks: Option<&[WeekFigures]>,
    request: ComparisonsRequest,
    state: &GoalState,
) -> WeekComparisons {
    let (feed, weeks) = match counter_weeks {
        Some(weeks) => (Feed::CounterPass, weeks),
        None => (Feed::Saved, &[][..]),
    };
    WeekComparisons {
        feed,
        week_start: request.week_start,
        lever: lever_state(feed, weeks, request.week_start, state),
        recap: counter_weeks.and_then(|weeks| recap(weeks, request, state)),
    }
}

/// The daemon's weeks are Mondays, strictly increasing, and bounded.
pub fn counter_weeks_valid(weeks: &[WeekFigures]) -> bool {
    use chrono::{Datelike, Weekday};
    weeks.len() <= MAX_COUNTER_WEEKS
        && weeks
            .iter()
            .all(|week| week.week_start.weekday() == Weekday::Mon)
        && weeks
            .windows(2)
            .all(|pair| pair[0].week_start < pair[1].week_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::lever::LeverDismissal;

    const READS: PatternKind = PatternKind::RepeatedReads;
    const CLAUDE: AnalyticsSource = AnalyticsSource::ClaudeCode;

    fn monday(weeks_ago: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() - Duration::weeks(weeks_ago)
    }

    /// A comparable week with `tokens` of Claude, the given cache share and
    /// repeated-reads figure.
    fn week(weeks_ago: i64, tokens: u64, share: u64, reads: u64) -> WeekFigures {
        WeekFigures {
            week_start: monday(weeks_ago),
            comparable: true,
            tokens: [(CLAUDE, tokens)].into(),
            cache_share_permille: [(CLAUDE, share)].into(),
            patterns: [(READS, reads)].into(),
            sessions: 6,
            pattern_counts: [(READS, 9)].into(),
            reread_files: Some(4),
            past_threshold: None,
        }
    }

    /// Weeks 6..=1 ago with a steady 100K of re-reads and a 300 share, and
    /// this week (0) as given.
    fn history(this: WeekFigures) -> Vec<WeekFigures> {
        let mut weeks: Vec<_> = (1..=6)
            .rev()
            .map(|ago| week(ago, 1_000_000, 300, 100_000))
            .collect();
        weeks.push(this);
        weeks
    }

    fn request(week_start: NaiveDate) -> ComparisonsRequest {
        ComparisonsRequest {
            week_start,
            current_week: monday(0),
            recap_card_enabled: true,
        }
    }

    #[test]
    fn under_feed_s_nothing_is_compared() {
        let found = week_comparisons(None, request(monday(0)), &GoalState::default());
        assert_eq!(found.feed, Feed::Saved);
        assert_eq!(found.lever.unavailable, Some(Unavailable::NeedsCounterPass));
        assert_eq!(found.recap, None);
    }

    #[test]
    fn comparisons_carry_no_goals() {
        let weeks = history(week(0, 800_000, 300, 100_000));
        let found = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        let value = serde_json::to_value(&found).unwrap();
        assert!(value.get("goals").is_none(), "{value}");
    }

    #[test]
    fn the_lever_names_its_kind_and_the_figures_its_line_fills() {
        let weeks = history(week(0, 1_000_000, 300, 300_000));
        let found = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        assert_eq!(
            found.lever.pick,
            Some(LeverView {
                kind: READS,
                tokens: 300_000,
                count: Some(9),
                files: Some(4),
                ratio_permille: 3_000
            })
        );
        assert_eq!(found.lever.unavailable, None);
    }

    #[test]
    fn not_useful_suppresses_the_lever() {
        let weeks = history(week(0, 1_000_000, 300, 300_000));
        let dismissed = GoalState {
            lever_dismissals: vec![LeverDismissal {
                kind: READS,
                week_start: monday(0),
            }],
            ..GoalState::default()
        };
        let found = week_comparisons(Some(&weeks), request(monday(0)), &dismissed);
        assert_eq!(found.lever.pick, None);
        assert_eq!(found.lever.unavailable, None);
    }

    /// `card_enabled` is the Settings switch for the lever's card: off after
    /// three "Not useful" turned a kind off, or when switched off, and then
    /// no card is drawn. Switched off, the "turned off by Not useful" list
    /// is empty, so the shell draws no line for it either.
    #[test]
    fn the_lever_card_switch_reads_off_after_three_not_useful_or_when_switched_off() {
        let weeks = history(week(0, 1_000_000, 300, 300_000));
        let on = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        assert!(on.lever.card_enabled);

        let three = GoalState {
            lever_dismissals: [20, 14, 8]
                .iter()
                .map(|ago| LeverDismissal {
                    kind: READS,
                    week_start: monday(*ago),
                })
                .collect(),
            ..GoalState::default()
        };
        let off = week_comparisons(Some(&weeks), request(monday(0)), &three);
        assert!(!off.lever.card_enabled);
        assert_eq!(off.lever.disabled, vec![READS]);
        // Under feed S there is no lever, but the switch still reads off.
        assert!(
            !week_comparisons(None, request(monday(0)), &three)
                .lever
                .card_enabled
        );

        let switched_off = GoalState {
            lever_card_off: true,
            ..GoalState::default()
        };
        let found = week_comparisons(Some(&weeks), request(monday(0)), &switched_off);
        assert!(!found.lever.card_enabled);
        assert_eq!(found.lever.pick, None);
        assert!(found.lever.disabled.is_empty());
        assert_eq!(found.lever.unavailable, None);
    }

    /// The closed week (1 ago) with a new best share, more re-reads than
    /// usual; the weeks before it steady.
    fn closed_week_history() -> Vec<WeekFigures> {
        let mut weeks: Vec<_> = (2..=7)
            .rev()
            .map(|ago| week(ago, 1_000_000, 300, 100_000))
            .collect();
        weeks.push(week(1, 800_000, 450, 250_000));
        weeks.push(week(0, 10, 10, 10));
        weeks
    }

    #[test]
    fn the_summary_card_lists_one_item_per_rule_in_order() {
        let weeks = closed_week_history();
        let found = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        let recap = found.recap.unwrap();
        assert_eq!(recap.week_start, monday(1));
        assert_eq!(recap.week_end, monday(1) + Duration::days(6));
        assert_eq!(recap.sessions, 6);
        assert_eq!(
            recap.sources,
            vec![RecapSource {
                source: CLAUDE,
                tokens: 800_000,
                change_permille: Some(-200)
            }]
        );
        assert_eq!(
            recap.items,
            vec![
                RecapItem::BestCacheWeek {
                    source: CLAUDE,
                    permille: 450,
                    previous_best_permille: 300
                },
                RecapItem::PatternUp {
                    pattern: READS,
                    up_permille: 1_500
                },
            ]
        );
        assert_eq!(recap.past_threshold, None);
    }

    /// The lever's card switched off in Settings leaves the summary card's
    /// pattern line out too, as a kind turned off by "Not useful" already
    /// is: the same observation, by the lever's rule. The other items stay.
    #[test]
    fn the_lever_card_switched_off_leaves_the_pattern_line_out_of_the_summary() {
        let weeks = closed_week_history();
        let switched_off = GoalState {
            lever_card_off: true,
            ..GoalState::default()
        };
        let recap = week_comparisons(Some(&weeks), request(monday(0)), &switched_off)
            .recap
            .unwrap();
        assert_eq!(
            recap.items,
            vec![RecapItem::BestCacheWeek {
                source: CLAUDE,
                permille: 450,
                previous_best_permille: 300
            },]
        );
    }

    #[test]
    fn an_item_whose_rule_has_no_figure_is_left_out_never_zero() {
        let mut weeks = closed_week_history();
        // Last week not comparable: no change; too few earlier weeks for a
        // best or a lever baseline.
        weeks.drain(..4);
        weeks[1].comparable = false;
        let found = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        let recap = found.recap.unwrap();
        assert_eq!(recap.sources[0].change_permille, None);
        assert!(recap.items.is_empty());
    }

    #[test]
    fn the_threshold_line_follows_the_users_threshold() {
        let mut weeks = closed_week_history();
        let closed = weeks.len() - 2;
        weeks[closed].past_threshold = Some(ThresholdCount {
            threshold: 200_000,
            sessions: 3,
        });
        let found = week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default());
        assert_eq!(
            found.recap.unwrap().past_threshold,
            Some(ThresholdCount {
                threshold: 200_000,
                sessions: 3
            })
        );
    }

    #[test]
    fn the_summary_card_shows_until_opened_and_only_while_switched_on() {
        let weeks = closed_week_history();
        let mut opened = GoalState::default();
        let shown = |state: &GoalState, enabled: bool| {
            week_comparisons(
                Some(&weeks),
                ComparisonsRequest {
                    recap_card_enabled: enabled,
                    ..request(monday(0))
                },
                state,
            )
            .recap
            .is_some()
        };
        assert!(shown(&opened, true));
        assert!(!shown(&opened, false));
        opened.recap_opened_week = Some(monday(2));
        assert!(shown(&opened, true));
        opened.recap_opened_week = Some(monday(1));
        assert!(!shown(&opened, true));
        // Feed S has no card.
        assert_eq!(
            week_comparisons(None, request(monday(0)), &GoalState::default()).recap,
            None
        );
    }

    #[test]
    fn a_thin_or_missing_closed_week_has_no_card() {
        let mut weeks = closed_week_history();
        let closed = weeks.len() - 2;
        weeks[closed].comparable = false;
        assert_eq!(
            week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default()).recap,
            None
        );
        weeks.remove(closed);
        assert_eq!(
            week_comparisons(Some(&weeks), request(monday(0)), &GoalState::default()).recap,
            None
        );
    }

    #[test]
    fn the_daemons_weeks_must_be_mondays_in_order_and_bounded() {
        let weeks = closed_week_history();
        assert!(counter_weeks_valid(&weeks));
        let mut reversed = weeks.clone();
        reversed.reverse();
        assert!(!counter_weeks_valid(&reversed));
        let mut tuesday = weeks.clone();
        tuesday[0].week_start += Duration::days(1);
        assert!(!counter_weeks_valid(&tuesday));
        let many: Vec<_> = (0..=MAX_COUNTER_WEEKS as i64)
            .rev()
            .map(|ago| week(ago, 1, 1, 1))
            .collect();
        assert!(!counter_weeks_valid(&many));
    }
}
