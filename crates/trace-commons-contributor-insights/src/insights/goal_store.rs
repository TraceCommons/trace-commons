//! The user's goals, the lever's feedback and the week whose summary card was
//! last opened, in one file beside the in-process Insights store's index.
//!
//! Only rule IDs (pattern kinds), ISO weeks (their Mondays), goal kinds,
//! source labels and the user's own thresholds are kept: no figure, no path,
//! no session. Goals and the lever are evaluated against feed T, whose rows
//! live in the daemon; this store never holds a figure from them.
//!
//! Every write takes the store lock and replaces the file atomically. A file
//! that cannot be read is an error, never an empty list, and a write refuses
//! rather than replace it.

use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use super::LocalInsightStore;
use super::goals::Goal;
use super::lever::{LeverDismissal, LeverReenable};
use super::patterns::PatternKind;

/// The file, in the Insights store directory.
pub const GOALS_FILE: &str = "analytics-goals.json";
pub const GOAL_STORE_SCHEMA: &str = "trace_commons.insights_goals.v1";
/// Goals one store holds. A bound on the store, not an analytics constant.
pub const MAX_GOALS: usize = 8;
/// "Not useful" and re-enable entries kept, newest weeks first. A bound on
/// the store, not an analytics constant.
pub const MAX_LEVER_FEEDBACK: usize = 64;
/// The largest per mille a cache-share goal may name.
const MAX_PERMILLE: u64 = 1_000;

/// Fixed labels for a goal store refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStoreError {
    /// The file cannot be read. Nothing is shown in its place.
    Unreadable,
    /// A threshold outside its range.
    GoalInvalid,
    /// [`MAX_GOALS`] are already set.
    Full,
    /// No goal has that ID.
    NotFound,
    /// A week that is not a Monday.
    WeekInvalid,
}

impl std::fmt::Display for GoalStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "insights_goals_unreadable",
            Self::GoalInvalid => "insights_goal_invalid",
            Self::Full => "insights_goal_limit",
            Self::NotFound => "insights_goal_not_found",
            Self::WeekInvalid => "insights_week_invalid",
        })
    }
}

impl std::error::Error for GoalStoreError {}

/// One goal and the ID it is deleted by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredGoal {
    pub id: String,
    pub goal: Goal,
}

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
    pub goals: Vec<StoredGoal>,
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
    next_id: u64,
    #[serde(deserialize_with = "goals_without_retired_kinds")]
    goals: Vec<StoredGoal>,
    lever_dismissals: Vec<LeverDismissal>,
    lever_reenables: Vec<LeverReenable>,
    #[serde(default)]
    lever_card_off: bool,
    recap_opened_week: Option<NaiveDate>,
}

/// Goal kinds that left Insights (product owner, 2026-10-29). A file written
/// before that still holds them; they are dropped on read, and the file is
/// written without them on the next change, so they never fail the store.
const RETIRED_GOAL_KINDS: [&str; 2] = ["repeated_reads_under", "long_context_under"];

fn goals_without_retired_kinds<'de, D>(deserializer: D) -> Result<Vec<StoredGoal>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut goals = Vec::with_capacity(entries.len());
    for entry in entries {
        let retired = entry
            .get("goal")
            .and_then(|goal| goal.get("kind"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| RETIRED_GOAL_KINDS.contains(&kind));
        if !retired {
            goals.push(serde_json::from_value(entry).map_err(serde::de::Error::custom)?);
        }
    }
    Ok(goals)
}

impl GoalFile {
    fn state(&self) -> GoalState {
        GoalState {
            goals: self.goals.clone(),
            lever_dismissals: self.lever_dismissals.clone(),
            lever_reenables: self.lever_reenables.clone(),
            lever_card_off: self.lever_card_off,
            recap_opened_week: self.recap_opened_week,
        }
    }
}

/// A goal's threshold is in range: a share from 1 to 1,000 per mille, or a
/// token count above zero.
pub fn validate_goal(goal: &Goal) -> Result<(), GoalStoreError> {
    let valid = match goal {
        Goal::CacheShareAtLeast { permille, .. } => (1..=MAX_PERMILLE).contains(permille),
        Goal::WeeklyTokensUnder { tokens, .. } => *tokens > 0,
    };
    valid.then_some(()).ok_or(GoalStoreError::GoalInvalid)
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
                    goals: Vec::new(),
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
        if file.schema != GOAL_STORE_SCHEMA
            || file.goals.len() > MAX_GOALS
            || file
                .goals
                .iter()
                .any(|stored| validate_goal(&stored.goal).is_err())
        {
            bail!(GoalStoreError::Unreadable);
        }
        Ok(file)
    }

    /// The goals, the lever's feedback and the last opened summary week.
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

    /// Add a goal, or replace the one with `id`.
    pub fn goal_set(&self, id: Option<&str>, goal: Goal) -> Result<GoalState> {
        validate_goal(&goal)?;
        self.update_goals(|file| {
            match id {
                Some(id) => {
                    let stored = file
                        .goals
                        .iter_mut()
                        .find(|stored| stored.id == id)
                        .ok_or(GoalStoreError::NotFound)?;
                    stored.goal = goal;
                }
                None => {
                    if file.goals.len() >= MAX_GOALS {
                        bail!(GoalStoreError::Full);
                    }
                    file.next_id += 1;
                    file.goals.push(StoredGoal {
                        id: format!("goal-{}", file.next_id),
                        goal,
                    });
                }
            }
            Ok(())
        })
    }

    pub fn goal_delete(&self, id: &str) -> Result<GoalState> {
        self.update_goals(|file| {
            let before = file.goals.len();
            file.goals.retain(|stored| stored.id != id);
            if file.goals.len() == before {
                bail!(GoalStoreError::NotFound);
            }
            Ok(())
        })
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
    use crate::insights::week_rollup::AnalyticsSource;
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

    const UNDER: Goal = Goal::WeeklyTokensUnder {
        source: AnalyticsSource::ClaudeCode,
        tokens: 50_000,
    };

    #[test]
    fn an_absent_store_reads_empty_and_is_not_created() {
        let (_dir, store) = store();
        assert_eq!(store.goal_state().unwrap(), GoalState::default());
        assert!(!store.dir.join(GOALS_FILE).exists());
    }

    #[test]
    fn goals_are_added_replaced_and_deleted_by_id() {
        let (_dir, store) = store();
        let state = store.goal_set(None, UNDER).unwrap();
        assert_eq!(state.goals.len(), 1);
        let id = state.goals[0].id.clone();
        let share = Goal::CacheShareAtLeast {
            source: AnalyticsSource::ClaudeCode,
            permille: 600,
        };
        let state = store.goal_set(Some(&id), share.clone()).unwrap();
        assert_eq!(
            state.goals,
            vec![StoredGoal {
                id: id.clone(),
                goal: share
            }]
        );
        assert_eq!(store.goal_state().unwrap(), state);
        let state = store.goal_delete(&id).unwrap();
        assert!(state.goals.is_empty());
        assert_eq!(
            label(store.goal_delete(&id).unwrap_err()),
            GoalStoreError::NotFound
        );
        assert_eq!(
            label(store.goal_set(Some("goal-99"), UNDER).unwrap_err()),
            GoalStoreError::NotFound
        );
        // IDs are never reused.
        let state = store.goal_set(None, UNDER).unwrap();
        assert_ne!(state.goals[0].id, id);
    }

    #[test]
    fn goals_are_bounded_and_their_thresholds_checked() {
        let (_dir, store) = store();
        for _ in 0..MAX_GOALS {
            store.goal_set(None, UNDER).unwrap();
        }
        assert_eq!(
            label(store.goal_set(None, UNDER).unwrap_err()),
            GoalStoreError::Full
        );
        for bad in [
            Goal::CacheShareAtLeast {
                source: AnalyticsSource::Codex,
                permille: 0,
            },
            Goal::CacheShareAtLeast {
                source: AnalyticsSource::Codex,
                permille: 1_001,
            },
            Goal::WeeklyTokensUnder {
                source: AnalyticsSource::ClaudeCode,
                tokens: 0,
            },
        ] {
            assert_eq!(
                validate_goal(&bad),
                Err(GoalStoreError::GoalInvalid),
                "{bad:?}"
            );
        }
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

    /// "Repeated reads under" and "Long context under" left Insights; a file
    /// that still holds them loads without them and is not unreadable.
    #[test]
    fn goals_of_a_retired_kind_are_dropped_and_never_fail_the_store() {
        let (_dir, store) = store();
        std::fs::write(
            store.dir.join(GOALS_FILE),
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":3,"goals":[{"id":"goal-1","goal":{"kind":"repeated_reads_under","tokens":50000}},{"id":"goal-2","goal":{"kind":"weekly_tokens_under","source":"codex","tokens":9000}},{"id":"goal-3","goal":{"kind":"long_context_under","tokens":0}}],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
        )
        .unwrap();
        let state = store.goal_state().unwrap();
        assert_eq!(
            state.goals,
            vec![StoredGoal {
                id: "goal-2".to_owned(),
                goal: Goal::WeeklyTokensUnder {
                    source: AnalyticsSource::Codex,
                    tokens: 9_000
                }
            }]
        );
        // A write succeeds and leaves the file without them.
        let state = store.goal_set(None, UNDER).unwrap();
        assert_eq!(state.goals.len(), 2);
        let written = std::fs::read_to_string(store.dir.join(GOALS_FILE)).unwrap();
        assert!(!written.contains("repeated_reads_under"));
        assert!(!written.contains("long_context_under"));
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_is_never_replaced() {
        let (_dir, store) = store();
        for bytes in [
            &b"not json"[..],
            br#"{"schema":"other","next_id":0,"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":0,"goals":[{"id":"goal-1","goal":{"kind":"weekly_tokens_under","source":"claude_code","tokens":0}}],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null}"#,
            br#"{"schema":"trace_commons.insights_goals.v1","next_id":0,"goals":[],"lever_dismissals":[],"lever_reenables":[],"recap_opened_week":null,"path":"/x"}"#,
        ] {
            std::fs::write(store.dir.join(GOALS_FILE), bytes).unwrap();
            assert_eq!(
                label(store.goal_state().unwrap_err()),
                GoalStoreError::Unreadable
            );
            assert_eq!(
                label(store.goal_set(None, UNDER).unwrap_err()),
                GoalStoreError::Unreadable
            );
            assert_eq!(std::fs::read(store.dir.join(GOALS_FILE)).unwrap(), bytes);
        }
    }
}
