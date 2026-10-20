//! Per-session facts the counter pass stores on a row: counts, fixed labels
//! and indexes into the row's declared model labels only. No path, file
//! name, command text or prompt text is held here (spec section 1).
//!
//! Every count that could not be read is `None`, never zero, and stays out
//! of every figure it would change.

use serde::{Deserialize, Serialize};

use crate::insights::TaskCategory;

/// One model's facts in a session, keyed by its index into the row's
/// `model_labels`. Each count is `None` when it could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFacts {
    pub label_ix: u16,
    /// Edit tool calls the model made.
    pub edits: Option<u32>,
    /// Edit tool calls the contributor declined; known only where the
    /// contributor could have been asked.
    pub denied: Option<u32>,
    /// Edits a later edit restored.
    pub undone: Option<u32>,
    /// Interrupts that ended one of this model's turns.
    pub interrupted: Option<u32>,
}

/// Whether the session's edits were put to the contributor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditMode {
    /// Every edit call was made where the contributor is asked.
    Prompting,
    /// No edit call was made where the contributor is asked.
    NotPrompting,
    /// Mixed, unread, or a mode the reader does not know.
    Unknown,
}

/// Distinct edited files per extension class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionClassCounts {
    pub test: u32,
    pub doc: u32,
    pub source: u32,
    pub config: u32,
}

impl ExtensionClassCounts {
    /// Distinct edited files across every class.
    pub fn total(&self) -> u32 {
        self.test
            .saturating_add(self.doc)
            .saturating_add(self.source)
            .saturating_add(self.config)
    }
}

/// The counts the task classifier reads (spec section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionShape {
    pub edited_files: ExtensionClassCounts,
    /// Files renamed or moved by `git mv` or `mv` commands.
    pub renamed_files: u32,
    /// Lines added by edit calls; `None` when any edit's line counts are
    /// unknown (every `Write` and `NotebookEdit` call).
    pub lines_added: Option<u32>,
    /// Lines removed by edit calls; `None` under the same rule.
    pub lines_removed: Option<u32>,
    /// Commands that failed before the session's first edit and passed
    /// after it.
    pub failed_before_first_edit_then_passed: u32,
}

/// The task type inferred from a session's shape. Not [`TaskCategory`],
/// which is a user label; [`InferredTask::task_category`] maps to it where a
/// shared label is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredTask {
    BugFix,
    Refactor,
    Tests,
    Docs,
    Other,
    NoEdits,
}

impl InferredTask {
    /// The shared label: `BugFix` is `Debugging`, `NoEdits` is `Other`, the
    /// rest by name. There is no mixed label.
    pub fn task_category(self) -> TaskCategory {
        match self {
            InferredTask::BugFix => TaskCategory::Debugging,
            InferredTask::Refactor => TaskCategory::Refactor,
            InferredTask::Tests => TaskCategory::Tests,
            InferredTask::Docs => TaskCategory::Docs,
            InferredTask::Other | InferredTask::NoEdits => TaskCategory::Other,
        }
    }
}

/// Everything the counter pass stores about one session beyond its tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFacts {
    pub models: Vec<ModelFacts>,
    pub shape: SessionShape,
    pub task: InferredTask,
    pub classifier_version: u32,
    pub active_secs: Option<u32>,
    pub finished: Option<bool>,
    pub user_turns: Option<u32>,
    pub edit_mode: EditMode,
    /// The scan or series hit a cap.
    pub capped: bool,
    /// Set by the daemon when the session delegated to members it did not
    /// read (`group_member_count > 0`).
    pub delegated_unread: bool,
    /// Every turn declared the Codex automatic reviewer's model.
    pub reviewer: bool,
}
