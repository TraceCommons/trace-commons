//! The lever's feedback and the week whose summary card was last opened, in
//! one file beside the in-process Insights store's index. The file keeps its
//! name and schema from when it also held the user's threshold goals.
//!
//! Only rule IDs (pattern kinds) and ISO weeks (their Mondays) are kept: no
//! figure, no path, no session. The lever is evaluated against feed T, whose
//! rows live in the daemon; this store never holds a figure from them.
//!
//! The threshold goals left Insights (product owner, 2026-10-29): Goals are
//! now the `/goal` goals set in Claude and Codex (`slash_goals`). A file
//! written before still holds them under `goals`; they are ignored on read,
//! whatever their shape, and never fail the store, and the next write leaves
//! the list empty.
//!
//! Every write takes the store lock and replaces the file atomically. A file
//! that cannot be read is an error, never an empty list, and a write refuses
//! rather than replace it.

use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use super::LocalInsightStore;
use super::lever::{LeverDismissal, LeverReenable};
use super::patterns::PatternKind;

/// The file, in the Insights store directory.
pub const GOALS_FILE: &str = "analytics-goals.json";
pub const GOAL_STORE_SCHEMA: &str = "trace_commons.insights_goals.v1";
/// "Not useful" and re-enable entries kept, newest weeks first. A bound on
/// the store, not an analytics constant.
pub const MAX_LEVER_FEEDBACK: usize = 64;

/// Fixed labels for a goal store refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStoreError {
    /// The file cannot be read. Nothing is shown in its place.
    Unreadable,
    /// A week that is not a Monday.
    WeekInvalid,
}

impl std::fmt::Display for GoalStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "insights_goals_unreadable",
            Self::WeekInvalid => "insights_week_invalid",
        })
    }
}

impl std::error::Error for GoalStoreError {}

/// What "Not useful" did, or what turned a kind back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeverFeedbackAction {
    NotUseful,
    Reenable,
}

/// The store's contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalState {
    pub lever_dismissals: Vec<LeverDismissal>,
    pub lever_reenables: Vec<LeverReenable>,
    /// The lever's card was switched off in Settings
    /// (`analytics_setting_lever`). Absent from a file written before the
    /// switch, which reads as on.
    #[serde(default)]
    pub lever_card_off: bool,
    /// The latest week whose summary card was opened.
    pub recap_opened_week: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalFile {
    schema: String,
    /// Kept as read, for an older build that still reads the file.
    next_id: u64,
    /// The retired threshold goals: read and ignored, written empty.
    #[serde(default)]
    goals: RetiredGoals,
    lever_dismissals: Vec<LeverDismissal>,
    lever_reenables: Vec<LeverReenable>,
    #[serde(default)]
    lever_card_off: bool,
    recap_opened_week: Option<NaiveDate>,
}

/// Whatever a file holds under `goals`, dropped on read; always written as
/// an empty list, so a build from before still reads the file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RetiredGoals;

impl Serialize for RetiredGoals {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(std::iter::empty::<()>())
    }
}

impl<'de> Deserialize<'de> for RetiredGoals {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(RetiredGoals)
    }
}

impl GoalFile {
    fn state(&self) -> GoalState {
        GoalState {
            lever_dismissals: self.lever_dismissals.clone(),
            lever_reenables: self.lever_reenables.clone(),
            lever_card_off: self.lever_card_off,
            recap_opened_week: self.recap_opened_week,
        }
    }
}

fn monday(week_start: NaiveDate) -> Result<NaiveDate, GoalStoreError> {
    (week_start.weekday() == Weekday::Mon)
        .then_some(week_start)
        .ok_or(GoalStoreError::WeekInvalid)
}

/// Keep the newest [`MAX_LEVER_FEEDBACK`] entries by week.
fn bound<T>(entries: &mut Vec<T>, week: impl Fn(&T) -> NaiveDate) {
    if entries.len() > MAX_LEVER_FEEDBACK {
        entries.sort_by_key(|entry| std::cmp::Reverse(week(entry)));
        entries.truncate(MAX_LEVER_FEEDBACK);
        entries.sort_by_key(&week);
    }
}

impl LocalInsightStore {
    fn goals_path(&self) -> std::path::PathBuf {
        self.dir.join(GOALS_FILE)
    }

    fn read_goal_file(&self) -> Result<GoalFile> {
        let bytes = match std::fs::read(self.goals_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GoalFile {
                    schema: GOAL_STORE_SCHEMA.to_string(),
                    next_id: 0,
                    goals: RetiredGoals,
                    lever_dismissals: Vec::new(),
                    lever_reenables: Vec::new(),
                    lever_card_off: false,
                    recap_opened_week: None,
                });
            }
            Err(_) => bail!(GoalStoreError::Unreadable),
        };
        let file: GoalFile =
            serde_json::from_slice(&bytes).map_err(|_| anyhow!(GoalStoreError::Unreadable))?;
        if file.schema != GOAL_STORE_SCHEMA {
            bail!(GoalStoreError::Unreadable);
        }
        Ok(file)
    }

    /// The lever's feedback and the last opened summary week.
    pub fn goal_state(&self) -> Result<GoalState> {
        Ok(self.read_goal_file()?.state())
    }

    fn update_goals(&self, change: impl FnOnce(&mut GoalFile) -> Result<()>) -> Result<GoalState> {
        let (_lock, _) = self.locked()?;
        let mut file = self.read_goal_file()?;
        change(&mut file)?;
        let bytes = serde_json::to_vec(&file)?;
        trace_commons_contributor_platform::local_files::write_atomic_0600(
            &self.dir,
            &self.goals_path(),
            &bytes,
        )
        .map_err(|_| anyhow!(super::InsightsStoreError::Unavailable))?;
        Ok(file.state())
    }

    /// "Not useful" on the lever's kind in `week_start`, or a kind turned
    /// back on. Stored as the kind and the week only.
    pub fn lever_feedback(
        &self,
        kind: PatternKind,
        week_start: NaiveDate,
        action: LeverFeedbackAction,
    ) -> Result<GoalState> {
        let week_start = monday(week_start)?;
        self.update_goals(|file| {
            match action {
                LeverFeedbackAction::NotUseful => {
                    let entry = LeverDismissal { kind, week_start };
                    if !file.lever_dismissals.contains(&entry) {
                        file.lever_dismissals.push(entry);
                    }
                    bound(&mut file.lever_dismissals, |entry| entry.week_start);
                }
                LeverFeedbackAction::Reenable => {
                    let entry = LeverReenable { kind, week_start };
                    if !file.lever_reenables.contains(&entry) {
                        file.lever_reenables.push(entry);
                    }
                    bound(&mut file.lever_reenables, |entry| entry.week_start);
                }
            }
            Ok(())
        })
    }

    /// The Settings switch for the lever's card (`analytics_setting_lever`).
    /// Off, no card is drawn, and the weekly summary card leaves out its
    /// pattern line, the same observation. On, the card is back, and so is
    /// every kind that three "Not useful" turned off
    /// ([`super::analytics_constants::LEVER_DISMISSALS_TO_DISABLE`]): that
    /// kind's "Not useful" are forgotten, so one given later, even in the
    /// same week, counts again from there.
    /// Other kinds' feedback is kept.
    pub fn lever_card(&self, enabled: bool) -> Result<GoalState> {
        self.update_goals(|file| {
            file.lever_card_off = !enabled;
            if enabled {
                let off = super::lever::kinds_off(&file.lever_dismissals, &file.lever_reenables);
                file.lever_dismissals
                    .retain(|entry| !off.contains(&entry.kind));
            }
            Ok(())
        })
    }

    /// The summary card for `week_start` was opened: no card for it, or for
    /// any earlier week, appears again.
    pub fn recap_opened(&self, week_start: NaiveDate) -> Result<GoalState> {
        let week_start = monday(week_start)?;
        self.update_goals(|file| {
            file.recap_opened_week = file.recap_opened_week.max(Some(week_start));
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    fn store() -> (tempfile::TempDir, LocalInsightStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("insights");
        let store = LocalInsightStore::open_with_digest_keys(
            &path,
            Box::new(InMemoryDigestKeyStore::with_seed([9; 32])),
        )
        .unwrap();
        (dir, store)
    }

    fn monday_at(weeks_ago: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap() - chrono::Duration::weeks(weeks_ago)
    }

    fn label(error: anyhow::Error) -> GoalStoreError {
        *error
            .downcast_ref::<GoalStoreError>()
            .expect("a goal store label")
    }

    #[test]
    fn an_absent_store_reads_empty_and_is_not_created() {
        let (_dir, store) = store();
        assert_eq!(store.goal_state().unwrap(), GoalState::default());
        assert!(!store.dir.join(GOALS_FILE).exists());
    }

    #[test]
    fn lever_feedback_keeps_the_kind_and_the_week_only() {
        let (_dir, store) = store();
        let state = store
            .lever_feedback(
                PatternKind::RepeatedReads,
                monday_at(0),
                LeverFeedbackAction::NotUseful,
            )
            .unwrap();
        assert_eq!(
            state.lever_dismissals,
            vec![LeverDismissal {
                kind: PatternKind::RepeatedReads,
                week_start: monday_at(0)
            }]
        );
        // The same feedback twice is one entry.
        let state = store
            .lever_feedback(
                PatternKind::RepeatedReads,
                monday_at(0),
                LeverFeedbackAction::NotUseful,
            )
            .unwrap();
        assert_eq!(state.lever_dismissals.len(), 1);
        let state = store
            .lever_feedback(
                PatternKind::RepeatedReads,
                monday_at(0),
                LeverFeedbackAction::Reenable,
            )
            .unwrap();
        assert_eq!(state.lever_reenables.len(), 1);
        assert_eq!(
            label(
                store
                    .lever_feedback(
                        PatternKind::LongContext,
                        monday_at(0) + chrono::Duration::days(1),
                        LeverFeedbackAction::NotUseful,
                    )
                    .unwrap_err()
            ),
            GoalStoreError::WeekInvalid
        );
        let text = std::fs::read_to_string(store.dir.join(GOALS_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "goals",
                "lever_card_off",
                "lever_dismissals",
                "lever_reenables",
                "next_id",
                "recap_opened_week",
                "schema"
            ]
        );
        assert_eq!(
            value["lever_dismissals"][0],
            serde_json::json!({"kind": "repeated_reads", "week_start": "2026-10-05"})
        );
    }

    /// The Settings switch for the lever's card (`analytics_setting_lever`):
    /// off, it stays off; on, every kind that three "Not useful" turned off
    /// is back, and a later "Not useful" counts again from there.
    #[test]
    fn the_lever_card_switch_turns_it_off_and_back_on() {
        let (_dir, store) = store();
        let state = store.goal_state().unwrap();
        assert!(!state.lever_card_off, "on by default");
        for weeks_ago in [3, 2, 1] {
            store
                .lever_feedback(
                    PatternKind::RepeatedReads,
                    monday_at(0) - chrono::Duration::weeks(weeks_ago),
                    LeverFeedbackAction::NotUseful,
                )
                .unwrap();
        }
        store
            .lever_feedback(
                PatternKind::LongContext,
                monday_at(0),
                LeverFeedbackAction::NotUseful,
            )
            .unwrap();
        let state = store.goal_state().unwrap();
        assert_eq!(
            super::super::lever::kinds_off(&state.lever_dismissals, &state.lever_reenables),
            vec![PatternKind::RepeatedReads]
        );

        let off = store.lever_card(false).unwrap();
        assert!(off.lever_card_off);
        assert_eq!(
            off.lever_dismissals.len(),
            4,
            "switching off forgets nothing"
        );

        let on = store.lever_card(true).unwrap();
        assert!(!on.lever_card_off);
        assert!(
            super::super::lever::kinds_off(&on.lever_dismissals, &on.lever_reenables).is_empty()
        );
        assert_eq!(
            on.lever_dismissals,
            vec![LeverDismissal {
                kind: PatternKind::LongContext,
                week_start: monday_at(0)
            }],
            "only the kinds that were off are forgotten"
        );
        // Persisted, and read back.
        assert_eq!(store.goal_state().unwrap(), on);
        // A "Not useful" in the week it was turned back on counts.
        let again = store
            .lever_feedback(
                PatternKind::RepeatedReads,
                monday_at(0),
                LeverFeedbackAction::NotUseful,
            )
            .unwrap();
        assert_eq!(again.lever_dismissals.len(), 2);
    }

    #[test]
    fn lever_feedback_is_bounded_to_the_newest_weeks() {
        let (_dir, store) = store();
        for ago in 0..(MAX_LEVER_FEEDBACK as i64 + 3) {
            store
                .lever_feedback(
                    PatternKind::RetriedCalls,
                    monday_at(ago),
                    LeverFeedbackAction::NotUseful,
                )
                .unwrap();
        }
        let state = store.goal_state().unwrap();
        assert_eq!(state.lever_dismissals.len(), MAX_LEVER_FEEDBACK);
        assert_eq!(
            state.lever_dismissals.last().unwrap().week_start,
            monday_at(0)
        );
        assert_eq!(
            state.lever_dismissals.first().unwrap().week_start,
            monday_at(MAX_LEVER_FEEDBACK as i64 - 1)
        );
    }

    #[test]
    fn the_opened_summary_week_only_moves_forward() {
        let (_dir, store) = store();
        assert_eq!(
            store.recap_opened(monday_at(1)).unwrap().recap_opened_week,
            Some(monday_at(1))
        );
        assert_eq!(
            store.recap_opened(monday_at(3)).unwrap().recap_opened_week,
            Some(monday_at(1))
        );
        assert_eq!(
            label(
                store
                    .recap_opened(monday_at(0) - chrono::Duration::days(2))
                    .unwrap_err()
            ),
            GoalStoreError::WeekInvalid
        );
    }

    /// A file written before the lever card's switch reads as switched on.
    #[test]
    fn a_file_without_the_lever_switch_reads_as_on() {
        let (_dir, store) = store();
        std::fs::write(
            store.dir.join(GOALS_FILE),
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":0,"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
        )
        .unwrap();
        assert!(!store.goal_state().unwrap().lever_card_off);
    }

    /// The threshold goals left Insights: a file that still holds them,
    /// of any kind or shape, loads without them and is not unreadable, and
    /// the next write leaves the list empty, keeping the file readable by a
    /// build from before.
    #[test]
    fn stored_threshold_goals_are_ignored_and_never_fail_the_store() {
        let (_dir, store) = store();
        std::fs::write(
            store.dir.join(GOALS_FILE),
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":4,"goals":[{"id":"goal-1","goal":{"kind":"repeated_reads_under","tokens":50000}},{"id":"goal-2","goal":{"kind":"weekly_tokens_under","source":"codex","tokens":9000}},{"id":"goal-3","goal":{"kind":"cache_share_at_least","source":"claude_code","permille":0}},{"id":"goal-4","goal":{"kind":"something_else"},"extra":true}],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
        )
        .unwrap();
        assert_eq!(store.goal_state().unwrap(), GoalState::default());
        store.recap_opened(monday_at(0)).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.dir.join(GOALS_FILE)).unwrap()).unwrap();
        assert_eq!(written["goals"], serde_json::json!([]));
        assert_eq!(written["next_id"], 4);
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_is_never_replaced() {
        let (_dir, store) = store();
        for bytes in [
            &b"not json"[..],
            br#"{"schema":"other","next_id":0,"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":0,"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null,"path":"/x"}"#,
        ] {
            std::fs::write(store.dir.join(GOALS_FILE), bytes).unwrap();
            assert_eq!(
                label(store.goal_state().unwrap_err()),
                GoalStoreError::Unreadable
            );
            assert_eq!(
                label(store.recap_opened(monday_at(0)).unwrap_err()),
                GoalStoreError::Unreadable
            );
            assert_eq!(std::fs::read(store.dir.join(GOALS_FILE)).unwrap(), bytes);
        }
    }
}
