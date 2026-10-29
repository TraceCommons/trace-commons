//! Goals the person set with the `/goal` slash command in Claude and Codex
//! (product owner, 2026-10-29). Each goal is shown with the tool's own
//! status: met or not met, and the tokens, time and checks it took.
//!
//! Privacy rule: a goal's text never leaves the parser. Claude's
//! `condition` and `reason`, the `/goal` command's arguments and Codex's
//! `objective` are never kept, stored, logged, sent over IPC or shown.
//! Only status, counts, tokens, time and timestamps are, keyed to the
//! session they belong to.
//!
//! Claude writes each check of a goal as an `attachment` record of type
//! `goal_status` in the session transcript. The record written when the
//! goal is set carries `sentinel: true`; later ones are checks, with `met`
//! and, when the tool reports them, `iterations`, `durationMs` and
//! `tokens`. A goal's state is its latest record's.
//!
//! Codex keeps one goal per thread in `goals_1.sqlite` under its home
//! directory (`thread_goals`). The daemon reads its status and counters
//! there; this module only names the statuses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::week_rollup::AnalyticsSource;

/// Goals one Claude session keeps, newest last. A bound on the store, not
/// an analytics constant: a session setting more keeps its latest ones.
pub const MAX_CLAUDE_GOALS_PER_SESSION: usize = 16;

/// The attachment type Claude writes for a goal being set or checked.
pub const CLAUDE_GOAL_STATUS: &str = "goal_status";

/// Where a Claude goal stands, from its latest `goal_status` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeGoalStatus {
    /// Set, and not checked yet: the latest record is the sentinel.
    Open,
    Met,
    NotMet,
}

/// One Claude goal as the counter pass stores it on a session's row. No
/// text: the condition and the reason are never held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeGoal {
    pub status: ClaudeGoalStatus,
    /// The latest record's `timestamp`: when the goal was set, or last
    /// checked. `None` when that record carries no valid time.
    pub updated_at: Option<DateTime<Utc>>,
    /// From the latest check, when Claude reported them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
}

/// Reads a Claude transcript's goals, record by record. Never fails: a
/// record it does not understand is passed over.
///
/// Goals are told apart by the sentinel record: each one starts a new
/// goal. A check whose condition differs from the current goal's also
/// starts one (its sentinel was not read). The condition is compared by a
/// SHA-256 held only while the transcript is read, never stored.
#[derive(Debug, Default)]
pub struct ClaudeGoalScan {
    goals: Vec<ClaudeGoal>,
    /// The current goal's condition digest; `None` with no current goal or
    /// one whose condition was not a string.
    current: Option<[u8; 32]>,
}

impl ClaudeGoalScan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, record: &Value) {
        if record.get("type").and_then(Value::as_str) != Some("attachment") {
            return;
        }
        let Some(attachment) = record.get("attachment") else {
            return;
        };
        if attachment.get("type").and_then(Value::as_str) != Some(CLAUDE_GOAL_STATUS) {
            return;
        }
        let sentinel = attachment.get("sentinel").and_then(Value::as_bool) == Some(true);
        // Hashed in place and dropped: the text is never copied out.
        let condition = attachment
            .get("condition")
            .and_then(Value::as_str)
            .map(|text| <[u8; 32]>::from(Sha256::digest(text.as_bytes())));
        let continues =
            !sentinel && !self.goals.is_empty() && condition.is_some() && condition == self.current;
        if !continues {
            if self.goals.len() >= MAX_CLAUDE_GOALS_PER_SESSION {
                self.goals.remove(0);
            }
            self.goals.push(ClaudeGoal {
                status: ClaudeGoalStatus::Open,
                updated_at: None,
                iterations: None,
                duration_ms: None,
                tokens: None,
            });
            self.current = condition;
        }
        let goal = self.goals.last_mut().expect("a goal was just ensured");
        goal.updated_at = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|at| at.with_timezone(&Utc));
        if sentinel {
            goal.status = ClaudeGoalStatus::Open;
            return;
        }
        goal.status = if attachment.get("met").and_then(Value::as_bool) == Some(true) {
            ClaudeGoalStatus::Met
        } else {
            ClaudeGoalStatus::NotMet
        };
        goal.iterations = attachment
            .get("iterations")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        goal.duration_ms = attachment.get("durationMs").and_then(Value::as_u64);
        goal.tokens = attachment.get("tokens").and_then(Value::as_u64);
    }

    pub fn finish(self) -> Vec<ClaudeGoal> {
        self.goals
    }
}

/// A goal's status as the week's answer names it: Codex's own status, or
/// Claude's met, not met or open. `Unknown` is a Codex status this build
/// does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
    Met,
    NotMet,
    Open,
    Unknown,
}

impl GoalStatus {
    /// Codex's `thread_goals.status`; any other value is `Unknown`.
    pub fn from_codex(status: &str) -> Self {
        match status {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "blocked" => Self::Blocked,
            "usage_limited" => Self::UsageLimited,
            "budget_limited" => Self::BudgetLimited,
            "complete" => Self::Complete,
            _ => Self::Unknown,
        }
    }

    /// Codex `complete`, or Claude's latest check met. Every other status
    /// counts as not met.
    pub fn met(self) -> bool {
        matches!(self, Self::Complete | Self::Met)
    }
}

impl From<ClaudeGoalStatus> for GoalStatus {
    fn from(status: ClaudeGoalStatus) -> Self {
        match status {
            ClaudeGoalStatus::Open => Self::Open,
            ClaudeGoalStatus::Met => Self::Met,
            ClaudeGoalStatus::NotMet => Self::NotMet,
        }
    }
}

/// One goal as the week's answer carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekGoal {
    pub tool: AnalyticsSource,
    pub status: GoalStatus,
    pub met: bool,
    pub tokens: Option<u64>,
    pub time_secs: Option<u64>,
    /// Claude's checks; Codex reports none.
    pub iterations: Option<u32>,
    /// The session's folder label, as the Sessions list shows it.
    pub folder: Option<String>,
    /// The session's listed reference, which the Sessions drill takes.
    pub session_ref: String,
    pub updated_at: DateTime<Utc>,
}

/// Met and total, for one tool or all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCount {
    pub met: u32,
    pub total: u32,
}

impl GoalCount {
    fn add(&mut self, met: bool) {
        self.total += 1;
        self.met += u32::from(met);
    }
}

/// The week's goals: each one, newest update first, and the counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekGoals {
    pub goals: Vec<WeekGoal>,
    pub met: u32,
    pub total: u32,
    /// Per tool with a goal this week.
    pub by_tool: std::collections::BTreeMap<AnalyticsSource, GoalCount>,
}

impl WeekGoals {
    pub fn new(mut goals: Vec<WeekGoal>) -> Self {
        goals.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let mut all = GoalCount::default();
        let mut by_tool = std::collections::BTreeMap::new();
        for goal in &goals {
            all.add(goal.met);
            by_tool
                .entry(goal.tool)
                .or_insert_with(GoalCount::default)
                .add(goal.met);
        }
        Self {
            goals,
            met: all.met,
            total: all.total,
            by_tool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "SECRET-GOAL-CONDITION-TEXT";

    fn status(condition: &str, at: &str, sentinel: bool, met: bool) -> Value {
        let mut attachment = json!({"type": "goal_status", "met": met, "condition": condition});
        if sentinel {
            attachment["sentinel"] = json!(true);
        } else {
            attachment["reason"] = json!("SECRET-GOAL-REASON-TEXT");
            attachment["iterations"] = json!(2);
            attachment["durationMs"] = json!(14_065);
            attachment["tokens"] = json!(926);
        }
        json!({"type": "attachment", "attachment": attachment, "timestamp": at})
    }

    fn scan(records: &[Value]) -> Vec<ClaudeGoal> {
        let mut scan = ClaudeGoalScan::new();
        for record in records {
            scan.observe(record);
        }
        scan.finish()
    }

    #[test]
    fn a_goal_set_and_not_checked_is_open() {
        let goals = scan(&[status(SECRET, "2026-10-29T14:36:55.256Z", true, false)]);
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].status, ClaudeGoalStatus::Open);
        assert_eq!(goals[0].tokens, None);
        assert_eq!(
            goals[0].updated_at.unwrap().to_rfc3339(),
            "2026-10-29T14:36:55.256+00:00"
        );
    }

    #[test]
    fn the_latest_check_decides_and_carries_its_counters() {
        let goals = scan(&[
            status(SECRET, "2026-10-29T14:36:55Z", true, false),
            status(SECRET, "2026-10-29T14:37:00Z", false, false),
            status(SECRET, "2026-10-29T14:37:09Z", false, true),
        ]);
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].status, ClaudeGoalStatus::Met);
        assert_eq!(goals[0].iterations, Some(2));
        assert_eq!(goals[0].duration_ms, Some(14_065));
        assert_eq!(goals[0].tokens, Some(926));
        assert_eq!(
            goals[0].updated_at.unwrap().to_rfc3339(),
            "2026-10-29T14:37:09+00:00"
        );
    }

    #[test]
    fn each_sentinel_starts_a_new_goal_even_with_the_same_condition() {
        let goals = scan(&[
            status(SECRET, "2026-10-29T14:00:00Z", true, false),
            status(SECRET, "2026-10-29T14:01:00Z", false, true),
            status(SECRET, "2026-10-29T15:00:00Z", true, false),
        ]);
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].status, ClaudeGoalStatus::Met);
        assert_eq!(goals[1].status, ClaudeGoalStatus::Open);
    }

    #[test]
    fn a_check_with_another_condition_is_a_new_goal() {
        let goals = scan(&[
            status("first", "2026-10-29T14:00:00Z", true, false),
            status("second", "2026-10-29T14:01:00Z", false, false),
            status("first", "2026-10-29T14:02:00Z", false, true),
        ]);
        assert_eq!(goals.len(), 3);
        assert_eq!(goals[1].status, ClaudeGoalStatus::NotMet);
        assert_eq!(goals[2].status, ClaudeGoalStatus::Met);
    }

    #[test]
    fn other_records_are_passed_over() {
        let goals = scan(&[
            json!({"type": "user", "message": {"content": "<command-name>/goal</command-name>"}}),
            json!({"type": "attachment", "attachment": {"type": "hook_success"}}),
            json!({"type": "attachment"}),
            json!("not an object"),
        ]);
        assert!(goals.is_empty());
    }

    #[test]
    fn a_session_keeps_its_latest_goals() {
        let records: Vec<Value> = (0..MAX_CLAUDE_GOALS_PER_SESSION + 3)
            .map(|n| status(&n.to_string(), "2026-10-29T14:00:00Z", true, false))
            .collect();
        assert_eq!(scan(&records).len(), MAX_CLAUDE_GOALS_PER_SESSION);
    }

    #[test]
    fn stored_goals_hold_no_text() {
        let goals = scan(&[
            status(SECRET, "2026-10-29T14:36:55Z", true, false),
            status(SECRET, "2026-10-29T14:37:09Z", false, true),
        ]);
        let bytes = serde_json::to_string(&goals).unwrap();
        assert!(!bytes.contains("SECRET"), "{bytes}");
    }

    #[test]
    fn codex_statuses_map_and_only_complete_or_met_is_met() {
        for (wire, status) in [
            ("active", GoalStatus::Active),
            ("paused", GoalStatus::Paused),
            ("blocked", GoalStatus::Blocked),
            ("usage_limited", GoalStatus::UsageLimited),
            ("budget_limited", GoalStatus::BudgetLimited),
            ("complete", GoalStatus::Complete),
            ("something_new", GoalStatus::Unknown),
        ] {
            assert_eq!(GoalStatus::from_codex(wire), status);
            assert_eq!(status.met(), wire == "complete");
        }
        assert!(GoalStatus::from(ClaudeGoalStatus::Met).met());
        assert!(!GoalStatus::from(ClaudeGoalStatus::Open).met());
        assert!(!GoalStatus::from(ClaudeGoalStatus::NotMet).met());
    }

    #[test]
    fn week_goals_count_by_tool_newest_first() {
        let goal = |tool, status: GoalStatus, at: &str| WeekGoal {
            tool,
            status,
            met: status.met(),
            tokens: None,
            time_secs: None,
            iterations: None,
            folder: None,
            session_ref: "t1.2".into(),
            updated_at: at.parse().unwrap(),
        };
        let week = WeekGoals::new(vec![
            goal(
                AnalyticsSource::Codex,
                GoalStatus::Complete,
                "2026-10-27T00:00:00Z",
            ),
            goal(
                AnalyticsSource::ClaudeCode,
                GoalStatus::NotMet,
                "2026-10-28T00:00:00Z",
            ),
            goal(
                AnalyticsSource::Codex,
                GoalStatus::Paused,
                "2026-10-26T00:00:00Z",
            ),
        ]);
        assert_eq!((week.met, week.total), (1, 3));
        assert_eq!(week.goals[0].tool, AnalyticsSource::ClaudeCode);
        assert_eq!(
            week.by_tool[&AnalyticsSource::Codex],
            GoalCount { met: 1, total: 2 }
        );
        assert_eq!(
            week.by_tool[&AnalyticsSource::ClaudeCode],
            GoalCount { met: 0, total: 1 }
        );
    }
}
