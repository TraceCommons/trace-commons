//! The person's own labels on a watched session (feed T): what the task
//! was, and what became of the work (Insights PM request, 2026-10-21).
//!
//! Local only: the daemon keeps them in a file of their own beside its
//! counter store, by the row they name, and they never leave the device. Two closed sets, each set
//! with one click on a Sessions feed row:
//!
//! - [`LabelledTask`]: the task as the person names it. Pre-filled from the
//!   inferred task ([`InferredTask`]), so confirming it is one click; once
//!   set it overrides the inferred task wherever the Models answers read
//!   one. There is no label for "no edits": that is a fact, not a task.
//! - [`SessionOutcome`]: whether the work was kept. Unset until the person
//!   says; never inferred. The Models answers read it as an acceptance
//!   signal beside their rejection figures (`model_answers::KeptFigure`).
//!
//! Not [`TaskCategory`](super::TaskCategory) and
//! [`TaskOutcome`](super::TaskOutcome), which label a saved snapshot in the
//! Analyze flow.

use serde::{Deserialize, Serialize};

use super::session_facts::InferredTask;

/// The task as the person names it: the inferred task's labels less
/// `no_edits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelledTask {
    BugFix,
    Refactor,
    Tests,
    Docs,
    Other,
}

impl LabelledTask {
    /// Every label, in the order a shell lists them.
    pub const ALL: [LabelledTask; 5] = [
        LabelledTask::BugFix,
        LabelledTask::Refactor,
        LabelledTask::Tests,
        LabelledTask::Docs,
        LabelledTask::Other,
    ];

    /// The task type the Models answers file the session under.
    pub fn inferred(self) -> InferredTask {
        match self {
            LabelledTask::BugFix => InferredTask::BugFix,
            LabelledTask::Refactor => InferredTask::Refactor,
            LabelledTask::Tests => InferredTask::Tests,
            LabelledTask::Docs => InferredTask::Docs,
            LabelledTask::Other => InferredTask::Other,
        }
    }

    /// The label for its wire word; `None` for any other string, `no_edits`
    /// included.
    pub fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|task| task.wire() == wire)
    }

    /// The snake_case word it crosses as.
    pub fn wire(self) -> &'static str {
        match self {
            LabelledTask::BugFix => "bug_fix",
            LabelledTask::Refactor => "refactor",
            LabelledTask::Tests => "tests",
            LabelledTask::Docs => "docs",
            LabelledTask::Other => "other",
        }
    }
}

/// What became of the session's work, as the person says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    Kept,
    PartlyKept,
    ThrownAway,
}

impl SessionOutcome {
    /// Every outcome, in the order a shell lists them.
    pub const ALL: [SessionOutcome; 3] = [
        SessionOutcome::Kept,
        SessionOutcome::PartlyKept,
        SessionOutcome::ThrownAway,
    ];

    /// The outcome for its wire word; `None` for any other string.
    pub fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.wire() == wire)
    }

    /// The snake_case word it crosses as.
    pub fn wire(self) -> &'static str {
        match self {
            SessionOutcome::Kept => "kept",
            SessionOutcome::PartlyKept => "partly_kept",
            SessionOutcome::ThrownAway => "thrown_away",
        }
    }
}

/// One session's labels. Either may be unset; a label with neither is not
/// stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionLabel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<LabelledTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<SessionOutcome>,
}

impl SessionLabel {
    pub fn is_empty(&self) -> bool {
        self.task.is_none() && self.outcome.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_label_crosses_as_its_serde_word() {
        for task in LabelledTask::ALL {
            let wire = serde_json::to_value(task).unwrap();
            assert_eq!(wire, task.wire());
            assert_eq!(LabelledTask::from_wire(task.wire()), Some(task));
            // The same word the inferred task it stands for crosses as.
            assert_eq!(serde_json::to_value(task.inferred()).unwrap(), wire);
        }
        for outcome in SessionOutcome::ALL {
            assert_eq!(serde_json::to_value(outcome).unwrap(), outcome.wire());
            assert_eq!(SessionOutcome::from_wire(outcome.wire()), Some(outcome));
        }
    }

    #[test]
    fn no_edits_and_unknown_words_are_no_label() {
        for wire in ["no_edits", "", "BugFix", "bug fix", "kept ", "accepted"] {
            assert_eq!(LabelledTask::from_wire(wire), None, "{wire}");
            assert_eq!(SessionOutcome::from_wire(wire), None, "{wire}");
        }
    }

    #[test]
    fn an_unset_label_is_left_out_of_the_stored_form() {
        assert_eq!(
            serde_json::to_value(SessionLabel::default()).unwrap(),
            serde_json::json!({})
        );
        let both = SessionLabel {
            task: Some(LabelledTask::BugFix),
            outcome: Some(SessionOutcome::PartlyKept),
        };
        let stored = serde_json::to_value(both).unwrap();
        assert_eq!(
            stored,
            serde_json::json!({"task": "bug_fix", "outcome": "partly_kept"})
        );
        assert_eq!(
            serde_json::from_value::<SessionLabel>(stored).unwrap(),
            both
        );
        assert!(
            serde_json::from_value::<SessionLabel>(serde_json::json!({"task": "no_edits"}))
                .is_err()
        );
    }
}
