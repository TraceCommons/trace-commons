//! Per-session facts the counter pass stores on a row: counts, fixed labels
//! and indexes into the row's declared model labels only. No path, file
//! name, command text or prompt text is held here (spec section 1).
//!
//! Every count that could not be read is `None`, never zero, and stays out
//! of every figure it would change.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use trace_commons_protocol::insights_usage_series::{
    DigestKey, KeyedDigest, MAX_SERIES_TOOL_CALLS, MAX_SERIES_TURNS, ToolKind, args_key,
    edit_text_key, path_key,
};

use super::analytics_constants::CLASSIFIER_VERSION;
use super::models::MAX_DECLARED_MODELS;
use super::task_classifier::{PathClass, classify, path_class};
use super::usage_evidence::safe_model;
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
    /// Edit calls a later edit restored, in the unit of `edits`: a call
    /// counts once however many of its sub-edits were restored.
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

/// The text Claude Code writes into a declined call's `tool_result`.
pub const CLAUDE_DENIAL_TEXT: &str = "The user doesn't want to proceed with this tool use";

/// The user-line texts Claude Code writes when the contributor interrupts a
/// turn. The second starts with the first, so the longer is checked first.
pub const CLAUDE_INTERRUPT_MARKERS: [&str; 2] = [
    "[Request interrupted by user]",
    "[Request interrupted by user for tool use]",
];

/// The permission mode in which Claude Code puts each edit to the
/// contributor.
pub const CLAUDE_PROMPTING_MODE: &str = "default";

/// Permission modes in which edits are not put to the contributor. Any other
/// value, and no value read yet, is an unknown mode.
pub const CLAUDE_NOT_PROMPTING_MODES: [&str; 4] =
    ["auto", "acceptEdits", "bypassPermissions", "dontAsk"];

const ASK_USER_QUESTION: &str = "AskUserQuestion";

fn invalid() -> anyhow::Error {
    anyhow!("insights_session_facts_invalid")
}

/// The class of the permission mode an edit call was made under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeClass {
    Prompting,
    NotPrompting,
    Unknown,
}

impl ModeClass {
    fn of(mode: &str) -> Self {
        if mode == CLAUDE_PROMPTING_MODE {
            Self::Prompting
        } else if CLAUDE_NOT_PROMPTING_MODES.contains(&mode) {
            Self::NotPrompting
        } else {
            Self::Unknown
        }
    }
}

/// One model's counts while the records are read.
#[derive(Default)]
struct ModelCounts {
    edits: u32,
    /// An edit call of this model was made outside the prompting mode.
    edit_outside_prompting: bool,
    denied: u32,
    undone: u32,
    interrupted: u32,
}

/// A tool call held until its result is read.
struct PendingCall {
    model: Option<usize>,
    edit: bool,
    ask: bool,
    at: Option<DateTime<Utc>>,
    paired: bool,
    /// An edit call's keyed path and its extension class.
    path: Option<(KeyedDigest, PathClass)>,
    /// An edit call's lines removed and added, `None` when the transcript
    /// does not show them. Counted only once the edit is applied.
    lines: Option<(u32, u32)>,
    /// Keyed digests of each `Edit` or `MultiEdit` sub-edit's old and new
    /// text, dropped once the result is read.
    texts: Vec<(KeyedDigest, KeyedDigest)>,
    /// A `Bash` call's `args_key`, and whether it was made before the
    /// session's first applied edit.
    command: Option<(KeyedDigest, bool)>,
}

/// An applied edit, held by path to find a later restore.
struct HeldEdit {
    old: KeyedDigest,
    new: KeyedDigest,
    model: Option<usize>,
    barrier: u64,
    /// The applied edit call it is a sub-edit of.
    call: u32,
    /// A restore of another sub-edit of the call already counted it undone.
    counted: bool,
}

/// One contributor prompt's span while it is open.
struct Span {
    start: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    waits_ms: i64,
    broken: bool,
}

/// What a [`ClaudeFactsScan`] holds.
#[cfg(test)]
#[derive(Debug)]
struct Held {
    messages: usize,
    calls: usize,
    edit_texts: usize,
    files: usize,
    commands: usize,
}

/// Reads a Claude Code transcript's session facts one parsed record at a
/// time, from the same records [`ClaudeTurnSeriesScan`] reads (spec section
/// 1). Paths, edit texts and commands are held only as keyed digests and
/// only during the scan; [`Self::finish`] returns counts.
///
/// Memory is bounded by the series caps: at most [`MAX_SERIES_TURNS`]
/// message IDs and [`MAX_SERIES_TOOL_CALLS`] calls, held edits, edited files
/// and commands. Past a cap the facts are `capped`.
///
/// [`ClaudeTurnSeriesScan`]: super::turn_series::ClaudeTurnSeriesScan
pub struct ClaudeFactsScan<'k> {
    key: &'k DigestKey,
    mode: ModeClass,
    /// Declared model labels, in first-seen order, at most
    /// `MAX_DECLARED_MODELS`.
    labels: Vec<String>,
    counts: Vec<ModelCounts>,
    model_by_message: BTreeMap<String, Option<usize>>,
    calls: BTreeMap<String, PendingCall>,
    edits_by_path: BTreeMap<KeyedDigest, Vec<HeldEdit>>,
    held_edits: usize,
    files: BTreeMap<KeyedDigest, PathClass>,
    failed_before_edit: BTreeSet<KeyedDigest>,
    passed_after_edit: BTreeSet<KeyedDigest>,
    renamed: u32,
    lines_added: u32,
    lines_removed: u32,
    lines_unknown: bool,
    edit_calls: u32,
    /// Edits whose result was neither a denial nor an error.
    applied_edits: u32,
    modes_seen: [bool; 3],
    /// Advanced by each contributor prompt, denial and interrupt.
    barrier: u64,
    /// A denial, undo or interrupt came after the last contributor prompt.
    rejected_since_prompt: bool,
    /// The previous user record held a denial.
    denial_pending: bool,
    last_model: Option<usize>,
    /// Whether the last assistant record had text, and its tool-use IDs
    /// (`None` for a call without an ID, which can never be paired).
    last_assistant: Option<(bool, Vec<Option<String>>)>,
    user_turns: u32,
    span: Option<Span>,
    active_ms: i64,
    active_unknown: bool,
    capped: bool,
}

impl<'k> ClaudeFactsScan<'k> {
    pub fn new(key: &'k DigestKey) -> Self {
        Self {
            key,
            mode: ModeClass::Unknown,
            labels: Vec::new(),
            counts: Vec::new(),
            model_by_message: BTreeMap::new(),
            calls: BTreeMap::new(),
            edits_by_path: BTreeMap::new(),
            held_edits: 0,
            files: BTreeMap::new(),
            failed_before_edit: BTreeSet::new(),
            passed_after_edit: BTreeSet::new(),
            renamed: 0,
            lines_added: 0,
            lines_removed: 0,
            lines_unknown: false,
            edit_calls: 0,
            applied_edits: 0,
            modes_seen: [false; 3],
            barrier: 0,
            rejected_since_prompt: false,
            denial_pending: false,
            last_model: None,
            last_assistant: None,
            user_turns: 0,
            span: None,
            active_ms: 0,
            active_unknown: false,
            capped: false,
        }
    }

    #[cfg(test)]
    fn held(&self) -> Held {
        Held {
            messages: self.model_by_message.len(),
            calls: self.calls.len(),
            edit_texts: self.held_edits,
            files: self.files.len(),
            commands: self.failed_before_edit.len() + self.passed_after_edit.len(),
        }
    }

    /// One record, under the contract of `ClaudeTurnSeriesScan::observe`:
    /// fails with a fixed label on a record that is not an object with a
    /// string `type`, and ignores record types it does not read.
    pub fn observe(&mut self, row: &Value) -> Result<()> {
        let kind = row
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        // Read from every line that carries it, including Claude Code's
        // `permission-mode` records.
        if let Some(mode) = row.get("permissionMode").and_then(Value::as_str) {
            self.mode = ModeClass::of(mode);
        }
        let at = row
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        match kind {
            "assistant" => self.assistant(row, at),
            "user" => self.user(row, at),
            _ => {}
        }
        Ok(())
    }

    /// The index of a declared model, interning it. `None` past the label
    /// bound, which caps the facts.
    fn model_ix(&mut self, model: &str) -> Option<usize> {
        if let Some(ix) = self.labels.iter().position(|label| label == model) {
            return Some(ix);
        }
        if self.labels.len() >= MAX_DECLARED_MODELS {
            self.capped = true;
            return None;
        }
        self.labels.push(model.to_owned());
        self.counts.push(ModelCounts::default());
        Some(self.labels.len() - 1)
    }

    fn assistant(&mut self, row: &Value, at: Option<DateTime<Utc>>) {
        self.model_event(at);
        let declared = row
            .pointer("/message/model")
            .and_then(Value::as_str)
            .filter(|model| safe_model(model));
        let record_model = declared.and_then(|model| self.model_ix(model));
        // A message's model is its first record's, as the series keys it.
        let id = row
            .pointer("/message/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256);
        let model = match id {
            Some(id) => match self.model_by_message.get(id) {
                Some(&model) => model,
                None => {
                    if self.model_by_message.len() < MAX_SERIES_TURNS {
                        self.model_by_message.insert(id.to_owned(), record_model);
                    } else {
                        self.capped = true;
                    }
                    record_model
                }
            },
            None => record_model,
        };
        if model.is_some() {
            self.last_model = model;
        }
        let blocks = match row.pointer("/message/content") {
            Some(Value::Array(blocks)) => blocks.as_slice(),
            _ => &[],
        };
        let has_text = blocks.iter().any(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        });
        let mut ids = Vec::new();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let id = block.get("id").and_then(Value::as_str);
            ids.push(id.map(str::to_owned));
            if id.is_some_and(|id| self.calls.contains_key(id)) {
                continue;
            }
            let call = self.tool_use(block, model, at);
            match id {
                Some(id) if self.calls.len() < MAX_SERIES_TOOL_CALLS => {
                    self.calls.insert(id.to_owned(), call);
                }
                _ => self.capped |= id.is_some(),
            }
        }
        self.last_assistant = Some((has_text, ids));
    }

    /// Counts one `tool_use` block at the time it is made, and returns what
    /// its result needs. An edit call counts toward its model's edits here;
    /// the session shape (edited files and lines) counts only edits that
    /// were applied, when the result arrives.
    fn tool_use(
        &mut self,
        block: &Value,
        model: Option<usize>,
        at: Option<DateTime<Utc>>,
    ) -> PendingCall {
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let tool = ToolKind::from_tool_name(name);
        let input = block.get("input").unwrap_or(&Value::Null);
        let mut call = PendingCall {
            model,
            edit: tool.is_edit(),
            ask: name == ASK_USER_QUESTION,
            at,
            paired: false,
            path: None,
            lines: Some((0, 0)),
            texts: Vec::new(),
            command: None,
        };
        if tool == ToolKind::Bash {
            if let Some(command) = input.get("command").and_then(Value::as_str) {
                self.renamed = self.renamed.saturating_add(renamed_operands(command));
            }
            call.command = args_key(self.key, tool, input)
                .ok()
                .map(|args| (args, self.applied_edits == 0));
            return call;
        }
        if !call.edit {
            return call;
        }
        self.edit_calls = self.edit_calls.saturating_add(1);
        let mode = self.mode;
        self.modes_seen[mode as usize] = true;
        if let Some(model) = model {
            let counts = &mut self.counts[model];
            counts.edits = counts.edits.saturating_add(1);
            counts.edit_outside_prompting |= mode != ModeClass::Prompting;
        }
        if let Some(path) = input
            .get("file_path")
            .or_else(|| input.get("notebook_path"))
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
        {
            call.path = Some((path_key(self.key, path), path_class(path)));
        }
        let mut pairs: Vec<(Option<&str>, Option<&str>)> = Vec::new();
        match tool {
            ToolKind::Edit => pairs.push((
                input.get("old_string").and_then(Value::as_str),
                input.get("new_string").and_then(Value::as_str),
            )),
            ToolKind::MultiEdit => match input.get("edits").and_then(Value::as_array) {
                Some(edits) => {
                    for edit in edits {
                        pairs.push((
                            edit.get("old_string").and_then(Value::as_str),
                            edit.get("new_string").and_then(Value::as_str),
                        ));
                    }
                }
                None => call.lines = None,
            },
            // An overwrite's removed lines are not in the transcript.
            _ => call.lines = None,
        }
        for pair in pairs {
            let (Some(old), Some(new)) = pair else {
                call.lines = None;
                continue;
            };
            if let Some((removed, added)) = &mut call.lines {
                *removed = removed.saturating_add(line_count(old));
                *added = added.saturating_add(line_count(new));
            }
            call.texts
                .push((edit_text_key(self.key, old), edit_text_key(self.key, new)));
        }
        call
    }

    fn user(&mut self, row: &Value, at: Option<DateTime<Utc>>) {
        // A compact summary is written by Claude Code, not the contributor.
        let meta = ["isMeta", "isCompactSummary"]
            .iter()
            .any(|flag| row.get(flag).and_then(Value::as_bool) == Some(true));
        let (texts, results): (Vec<&str>, Vec<&Value>) = match row.pointer("/message/content") {
            Some(Value::String(text)) => (vec![text.as_str()], Vec::new()),
            Some(Value::Array(blocks)) => {
                let mut texts = Vec::new();
                let mut results = Vec::new();
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                texts.push(text);
                            }
                        }
                        Some("tool_result") => results.push(block),
                        _ => {}
                    }
                }
                (texts, results)
            }
            _ => (Vec::new(), Vec::new()),
        };
        if !results.is_empty() {
            self.model_event(at);
        }
        let mut denial_here = false;
        for block in &results {
            denial_here |= self.tool_result(block, at);
        }
        let mut prompt_text = false;
        for text in texts {
            if text.starts_with(CLAUDE_INTERRUPT_MARKERS[1]) {
                // Part of the denial it follows, not a second rejection.
                if self.denial_pending || denial_here {
                    continue;
                }
                self.interrupt();
            } else if text.starts_with(CLAUDE_INTERRUPT_MARKERS[0]) {
                self.interrupt();
            } else {
                prompt_text |= contributor_words(text);
            }
        }
        if prompt_text && !meta && results.is_empty() {
            self.prompt(at);
        }
        self.denial_pending = denial_here;
    }

    fn interrupt(&mut self) {
        if let Some(model) = self.last_model {
            let counts = &mut self.counts[model];
            counts.interrupted = counts.interrupted.saturating_add(1);
        }
        self.barrier += 1;
        self.rejected_since_prompt = true;
    }

    fn prompt(&mut self, at: Option<DateTime<Utc>>) {
        self.user_turns = self.user_turns.saturating_add(1);
        self.barrier += 1;
        self.rejected_since_prompt = false;
        // The answer to an earlier prompt does not answer this one.
        self.last_assistant = None;
        self.close_span();
        self.span = Some(Span {
            start: at,
            last: None,
            waits_ms: 0,
            broken: at.is_none(),
        });
    }

    /// One `tool_result` block. Returns whether it is a denial.
    fn tool_result(&mut self, block: &Value, at: Option<DateTime<Utc>>) -> bool {
        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
            return false;
        };
        let Some(call) = self.calls.get_mut(id) else {
            return false;
        };
        if call.paired {
            return false;
        }
        call.paired = true;
        let texts = std::mem::take(&mut call.texts);
        let (model, edit, ask, used_at, path, lines, command) = (
            call.model,
            call.edit,
            call.ask,
            call.at,
            call.path,
            call.lines,
            call.command,
        );
        let denied = result_text_contains(block.get("content"), CLAUDE_DENIAL_TEXT);
        if denied || ask {
            self.subtract_wait(used_at, at);
        }
        if denied {
            self.barrier += 1;
            self.rejected_since_prompt = true;
            if let (true, Some(model)) = (edit, model) {
                let counts = &mut self.counts[model];
                counts.denied = counts.denied.saturating_add(1);
            }
            return true;
        }
        let is_error = block.get("is_error").and_then(Value::as_bool);
        if let Some((args, before_first_edit)) = command {
            match is_error {
                Some(true) if before_first_edit => {
                    if self.failed_before_edit.len() + self.passed_after_edit.len()
                        < MAX_SERIES_TOOL_CALLS
                    {
                        self.failed_before_edit.insert(args);
                    } else {
                        self.capped = true;
                    }
                }
                Some(false) if !before_first_edit && self.failed_before_edit.contains(&args) => {
                    self.passed_after_edit.insert(args);
                }
                _ => {}
            }
        }
        if edit && is_error != Some(true) {
            self.applied_edits = self.applied_edits.saturating_add(1);
            match lines {
                Some((removed, added)) => {
                    self.lines_removed = self.lines_removed.saturating_add(removed);
                    self.lines_added = self.lines_added.saturating_add(added);
                }
                None => self.lines_unknown = true,
            }
            if let Some((path, class)) = path {
                if !self.files.contains_key(&path) {
                    if self.files.len() < MAX_SERIES_TOOL_CALLS {
                        self.files.insert(path, class);
                    } else {
                        self.capped = true;
                    }
                }
                let call = self.applied_edits;
                for (old, new) in texts {
                    self.applied_edit(path, old, new, model, call);
                }
            }
        }
        false
    }

    /// An applied edit or sub-edit of edit call `call`: credits the earlier
    /// call it restores, if a prompt, denial or interrupt came between, then
    /// holds it. A call is undone once, as it is one edit, however many of
    /// its sub-edits are restored; they share its path.
    fn applied_edit(
        &mut self,
        path: KeyedDigest,
        old: KeyedDigest,
        new: KeyedDigest,
        model: Option<usize>,
        call: u32,
    ) {
        let barrier = self.barrier;
        let held = self.edits_by_path.entry(path).or_default();
        if let Some(ix) = held
            .iter()
            .rposition(|e| e.old == new && e.new == old && e.barrier < barrier)
        {
            let restored = held.remove(ix);
            self.held_edits -= 1;
            if !restored.counted {
                for sibling in held.iter_mut().filter(|e| e.call == restored.call) {
                    sibling.counted = true;
                }
                if let Some(model) = restored.model {
                    let counts = &mut self.counts[model];
                    counts.undone = counts.undone.saturating_add(1);
                }
            }
            self.rejected_since_prompt = true;
        }
        if self.held_edits < MAX_SERIES_TOOL_CALLS {
            held.push(HeldEdit {
                old,
                new,
                model,
                barrier,
                call,
                counted: false,
            });
            self.held_edits += 1;
        } else {
            self.capped = true;
        }
    }

    /// A model event (an assistant record or a tool result) inside the open
    /// span.
    fn model_event(&mut self, at: Option<DateTime<Utc>>) {
        let Some(span) = &mut self.span else {
            return;
        };
        match at {
            Some(at) => span.last = Some(span.last.map_or(at, |last| last.max(at))),
            None => span.broken = true,
        }
    }

    /// Time a call waited on the contributor, from its use to its result.
    fn subtract_wait(&mut self, used: Option<DateTime<Utc>>, result: Option<DateTime<Utc>>) {
        let Some(span) = &mut self.span else {
            return;
        };
        match (used, result) {
            (Some(used), Some(result)) if result >= used => {
                span.waits_ms = span
                    .waits_ms
                    .saturating_add((result - used).num_milliseconds());
            }
            _ => span.broken = true,
        }
    }

    fn close_span(&mut self) {
        let Some(span) = self.span.take() else {
            return;
        };
        let (Some(start), false) = (span.start, span.broken) else {
            self.active_unknown = true;
            return;
        };
        let end = span.last.unwrap_or(start);
        if end < start {
            self.active_unknown = true;
            return;
        }
        let ms = (end - start).num_milliseconds() - span.waits_ms;
        self.active_ms = self.active_ms.saturating_add(ms.max(0));
    }

    /// The facts, with each model keyed by its index into `model_labels`
    /// (the series evidence's labels). A model not among them is dropped
    /// and the facts are `capped`.
    pub fn finish(mut self, model_labels: &[String]) -> SessionFacts {
        self.close_span();
        let mut models = Vec::new();
        for (label, counts) in self.labels.iter().zip(&self.counts) {
            let Some(label_ix) = model_labels
                .iter()
                .position(|known| known == label)
                .and_then(|ix| u16::try_from(ix).ok())
            else {
                self.capped = true;
                continue;
            };
            models.push(ModelFacts {
                label_ix,
                edits: Some(counts.edits),
                denied: (!counts.edit_outside_prompting).then_some(counts.denied),
                undone: Some(counts.undone),
                interrupted: Some(counts.interrupted),
            });
        }
        models.sort_by_key(|model| model.label_ix);
        let edit_mode = match self.modes_seen {
            _ if self.edit_calls == 0 => EditMode::Unknown,
            [true, false, false] => EditMode::Prompting,
            [false, true, false] => EditMode::NotPrompting,
            _ => EditMode::Unknown,
        };
        let mut edited_files = ExtensionClassCounts::default();
        for class in self.files.values() {
            let count = match class {
                PathClass::Test => &mut edited_files.test,
                PathClass::Doc => &mut edited_files.doc,
                PathClass::Source => &mut edited_files.source,
                PathClass::Config => &mut edited_files.config,
            };
            *count = count.saturating_add(1);
        }
        let shape = SessionShape {
            edited_files,
            renamed_files: self.renamed,
            lines_added: (!self.lines_unknown).then_some(self.lines_added),
            lines_removed: (!self.lines_unknown).then_some(self.lines_removed),
            failed_before_first_edit_then_passed: u32::try_from(self.passed_after_edit.len())
                .unwrap_or(u32::MAX),
        };
        let ended_in_text = self.last_assistant.as_ref().is_some_and(|(text, ids)| {
            *text
                && ids.iter().all(|id| {
                    id.as_ref()
                        .and_then(|id| self.calls.get(id))
                        .is_some_and(|call| call.paired)
                })
        });
        SessionFacts {
            models,
            task: classify(&shape),
            shape,
            classifier_version: CLASSIFIER_VERSION,
            // No prompt opened a span: the time was never measured.
            active_secs: (!self.active_unknown && self.user_turns > 0)
                .then(|| u32::try_from(self.active_ms / 1000).unwrap_or(u32::MAX)),
            finished: Some(self.edit_calls > 0 && ended_in_text && !self.rejected_since_prompt),
            user_turns: Some(self.user_turns),
            edit_mode,
            capped: self.capped,
            delegated_unread: false,
            reviewer: false,
        }
    }
}

/// The wrappers Claude Code writes around a slash command and its local
/// output. What they hold is the interface's, not the contributor's words.
const CLAUDE_COMMAND_WRAPPERS: [&str; 8] = [
    "command-name",
    "command-message",
    "local-command-stdout",
    "local-command-stderr",
    "local-command-caveat",
    // Bash mode (`! command`): the contributor ran a shell command, not a
    // prompt to the model.
    "bash-input",
    "bash-stdout",
    "bash-stderr",
];

/// Whether a user text holds the contributor's own words, read as the
/// preview's task prompt reads it: the slash command wrappers are skipped,
/// a slash command's non-empty arguments are words (`/review fix the login
/// race`), and an unclosed wrapper or nothing left is none.
fn contributor_words(mut text: &str) -> bool {
    loop {
        text = text.trim();
        if let Some(args) = text.strip_prefix("<command-args>") {
            let Some((args, rest)) = args.split_once("</command-args>") else {
                return false;
            };
            if !args.trim().is_empty() {
                return true;
            }
            text = rest;
            continue;
        }
        let Some(tag) = CLAUDE_COMMAND_WRAPPERS
            .iter()
            .find(|tag| text.starts_with(&format!("<{tag}>")))
        else {
            return !text.is_empty();
        };
        let Some((_, rest)) = text.split_once(&format!("</{tag}>")) else {
            return false;
        };
        text = rest;
    }
}

/// Lines in an edit's text: zero for an empty string.
pub(crate) fn line_count(text: &str) -> u32 {
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

/// Path operands of each `mv` or `git mv` command in a command line, minus
/// one per command: the files those commands rename or move. A control
/// operator (`&&`, `||`, `;`, `|`, `&`) starts a new command, and a
/// redirection (`>`, `<`, `2>`, `&>`) ends a command's operands, so neither
/// is counted as a path. Quotes group words. Zero for any other command.
/// Only the count leaves this function.
pub(crate) fn renamed_operands(command: &str) -> u32 {
    let mut total: u32 = 0;
    for words in shell_commands(command) {
        let mut words = words.iter().map(String::as_str);
        let rest = match words.next() {
            Some("mv") => words,
            Some("git") if words.next() == Some("mv") => words,
            _ => continue,
        };
        let operands = rest.filter(|word| !word.starts_with('-')).count();
        let renamed = u32::try_from(operands.saturating_sub(1)).unwrap_or(u32::MAX);
        total = total.saturating_add(renamed);
    }
    total
}

/// The words of each simple command in `line`, split at control operators,
/// with each command's words cut at its first redirection. A small reading
/// of shell syntax: quotes group, and nothing is expanded.
fn shell_commands(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut redirected = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                in_word = true;
            }
            '<' | '>' => {
                // A numeric file descriptor before it belongs to the
                // redirection, not to the operands.
                if !(in_word && !word.is_empty() && word.chars().all(|d| d.is_ascii_digit())) {
                    end_word(&mut words, &mut word, &mut in_word, redirected);
                }
                word.clear();
                in_word = false;
                redirected = true;
            }
            '&' if chars.peek() == Some(&'>') => {
                end_word(&mut words, &mut word, &mut in_word, redirected);
                redirected = true;
            }
            '&' if redirected && word.is_empty() && !in_word => {
                // `>&1`: part of the redirection.
            }
            ';' | '|' | '&' => {
                end_word(&mut words, &mut word, &mut in_word, redirected);
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
                redirected = false;
            }
            c if c.is_whitespace() => end_word(&mut words, &mut word, &mut in_word, redirected),
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    end_word(&mut words, &mut word, &mut in_word, redirected);
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

fn end_word(words: &mut Vec<String>, word: &mut String, in_word: &mut bool, redirected: bool) {
    if *in_word && !redirected {
        words.push(std::mem::take(word));
    }
    word.clear();
    *in_word = false;
}

/// Whether a `tool_result`'s content (a string, or any text part) contains
/// `needle`.
fn result_text_contains(content: Option<&Value>, needle: &str) -> bool {
    match content {
        Some(Value::String(text)) => text.contains(needle),
        Some(Value::Array(blocks)) => blocks.iter().any(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains(needle))
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use trace_commons_protocol::insights_usage_series::{
        DigestKey, DigestKeyStore, InMemoryDigestKeyStore, MAX_SERIES_TOOL_CALLS, MAX_SERIES_TURNS,
    };

    use super::*;
    use crate::insights::analytics_constants::CLASSIFIER_VERSION;
    use crate::insights::turn_series::extract_claude_turn_series;

    const A: &str = "claude-model-a";
    const B: &str = "claude-model-b";

    fn key() -> DigestKey {
        InMemoryDigestKeyStore::with_seed([7; 32])
            .load_or_create()
            .unwrap()
    }

    fn fixture_text(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../trace-commons-contributor/fixtures/insights/claude-session-facts/{name}.jsonl"
            )),
        )
        .unwrap()
    }

    /// The facts of `text`, with the model labels the series scan reads from
    /// the same bytes, as the daemon pairs them.
    fn facts_of(text: &str) -> SessionFacts {
        let key = key();
        let labels = extract_claude_turn_series(text.as_bytes(), &key)
            .unwrap()
            .model_labels;
        let mut scan = ClaudeFactsScan::new(&key);
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            scan.observe(&serde_json::from_str::<Value>(line).unwrap())
                .unwrap();
        }
        scan.finish(&labels)
    }

    fn facts(name: &str) -> SessionFacts {
        facts_of(&fixture_text(name))
    }

    /// The facts of the model whose label is `label`.
    fn model<'f>(facts: &'f SessionFacts, name: &str, label: &str) -> &'f ModelFacts {
        let labels = extract_claude_turn_series(fixture_text(name).as_bytes(), &key())
            .unwrap()
            .model_labels;
        let ix = labels.iter().position(|l| l == label).unwrap() as u16;
        facts.models.iter().find(|m| m.label_ix == ix).unwrap()
    }

    fn one(name: &str) -> (SessionFacts, ModelFacts) {
        let f = facts(name);
        let m = model(&f, name, A).clone();
        (f, m)
    }

    #[test]
    fn a_denial_counts_once_though_its_text_appears_twice() {
        let (f, m) = one("denial-twice");
        assert_eq!(m.edits, Some(1));
        assert_eq!(m.denied, Some(1));
        assert_eq!(f.edit_mode, EditMode::Prompting);
    }

    #[test]
    fn a_declined_bash_or_ask_user_question_is_not_a_denied_edit() {
        let (_, m) = one("declined-bash-and-question");
        assert_eq!(m.edits, Some(1));
        assert_eq!(m.denied, Some(0));
    }

    #[test]
    fn the_interrupt_after_a_denial_is_not_counted_again() {
        let (_, m) = one("interrupt-after-denial");
        assert_eq!(m.denied, Some(1));
        assert_eq!(m.interrupted, Some(0));
    }

    #[test]
    fn both_interrupt_spellings_count_with_or_without_a_newline() {
        let (f, m) = one("interrupts");
        assert_eq!(m.interrupted, Some(4));
        assert_eq!(f.user_turns, Some(4), "interrupt markers are not prompts");
    }

    #[test]
    fn denied_is_unknown_under_each_non_prompting_mode() {
        let template = fixture_text("mode-template");
        for mode in [
            "auto",
            "acceptEdits",
            "bypassPermissions",
            "dontAsk",
            "someNewMode",
        ] {
            let f = facts_of(&template.replace("MODE", mode));
            let m = &f.models[0];
            assert_eq!(m.edits, Some(1), "{mode}");
            assert_eq!(m.denied, None, "{mode}");
            let expected = if mode == "someNewMode" {
                EditMode::Unknown
            } else {
                EditMode::NotPrompting
            };
            assert_eq!(f.edit_mode, expected, "{mode}");
        }
        let f = facts_of(&template.replace("MODE", "default"));
        assert_eq!(f.models[0].denied, Some(0));
        assert_eq!(f.edit_mode, EditMode::Prompting);
    }

    #[test]
    fn an_edit_before_any_mode_is_read_has_denied_unknown() {
        let template = fixture_text("mode-template");
        let without: String = template
            .lines()
            .filter(|line| !line.contains("permissionMode"))
            .map(|line| format!("{line}\n"))
            .collect();
        let f = facts_of(&without);
        assert_eq!(f.models[0].denied, None);
        assert_eq!(f.edit_mode, EditMode::Unknown);
    }

    #[test]
    fn a_mode_change_mid_session_makes_denied_and_edit_mode_unknown() {
        let name = "mode-change";
        let f = facts(name);
        assert_eq!(model(&f, name, A).denied, None);
        assert_eq!(model(&f, name, A).edits, Some(2));
        // Every one of model-b's edits was made under `default`.
        assert_eq!(model(&f, name, B).denied, Some(0));
        assert_eq!(f.edit_mode, EditMode::Unknown);
    }

    #[test]
    fn a_restore_after_a_contributor_prompt_is_undone() {
        let (f, m) = one("restore-after-prompt");
        assert_eq!(m.undone, Some(1));
        // Lines from each Edit's old and new text: 2 + 3 removed, 3 + 2 added.
        assert_eq!(f.shape.lines_removed, Some(5));
        assert_eq!(f.shape.lines_added, Some(5));
        assert_eq!(f.shape.edited_files.source, 1);
    }

    #[test]
    fn the_same_restore_with_nothing_between_is_not_undone() {
        let (_, m) = one("restore-without-barrier");
        assert_eq!(m.undone, Some(0));
    }

    #[test]
    fn a_multiedit_sub_edit_restore_counts_and_a_write_overwrite_does_not() {
        let (f, m) = one("multiedit-and-write");
        assert_eq!(m.edits, Some(5));
        assert_eq!(m.undone, Some(1));
        // A Write's removed lines are not in the transcript.
        assert_eq!(f.shape.lines_added, None);
        assert_eq!(f.shape.lines_removed, None);
        assert_eq!(f.shape.edited_files.source, 2);
    }

    /// Edits and undone count one unit, the edit call: a MultiEdit whose
    /// three sub-edits a later MultiEdit restores is one edit undone.
    #[test]
    fn a_multiedit_restored_whole_is_one_edit_undone() {
        let (_, m) = one("multiedit-restore");
        assert_eq!(m.edits, Some(2));
        assert_eq!(m.undone, Some(1));
        assert!(m.undone <= m.edits);
    }

    #[test]
    fn undone_goes_to_the_model_that_made_the_edit() {
        let name = "undone-other-model";
        let f = facts(name);
        assert_eq!(model(&f, name, A).undone, Some(1));
        assert_eq!(model(&f, name, B).undone, Some(0));
    }

    #[test]
    fn a_long_build_inside_a_span_counts_and_the_idle_tail_does_not() {
        let (f, _) = one("active-time");
        assert_eq!(f.active_secs, Some(320));
        assert_eq!(f.user_turns, Some(2));
    }

    #[test]
    fn ask_user_question_and_denied_call_waits_are_subtracted() {
        let (f, m) = one("waits");
        assert_eq!(f.active_secs, Some(20));
        assert_eq!(m.denied, Some(1));
    }

    #[test]
    fn a_missing_timestamp_in_a_span_makes_active_time_unknown() {
        let (f, _) = one("missing-timestamp");
        assert_eq!(f.active_secs, None);
    }

    /// With no contributor prompt there is no span: the time was never
    /// measured, so it is unknown, never zero seconds.
    #[test]
    fn a_session_with_no_contributor_prompt_has_unknown_active_time() {
        let text = fixture_text("finished");
        let without_prompt: String = text
            .lines()
            .skip(1)
            .map(|line| format!("{line}\n"))
            .collect();
        let f = facts_of(&without_prompt);
        assert_eq!(f.user_turns, Some(0));
        assert_eq!(f.active_secs, None);
        // The fixture with its prompt still measures its span.
        assert!(facts("finished").active_secs.is_some());
    }

    #[test]
    fn finished_needs_an_edit_a_text_ending_and_no_rejection_after_the_last_prompt() {
        assert_eq!(facts("finished").finished, Some(true));
        assert_eq!(facts("finished-no-edit").finished, Some(false));
        assert_eq!(facts("finished-unpaired-call").finished, Some(false));
        assert_eq!(
            facts("finished-interrupt-after-prompt").finished,
            Some(false)
        );
        // A denial after the last prompt: the denied call is followed by text.
        assert_eq!(facts("denial-twice").finished, Some(false));
    }

    /// The answer the earlier prompt got does not finish a session whose
    /// last prompt went unanswered.
    #[test]
    fn a_trailing_unanswered_prompt_is_not_finished() {
        let f = facts("finished-trailing-prompt");
        assert_eq!(f.user_turns, Some(2));
        assert_eq!(f.finished, Some(false));
    }

    /// A bash-mode command after the model's answer (`! git push`) is not a
    /// prompt: the session stays finished and gains no user turn.
    #[test]
    fn a_bash_mode_command_after_the_answer_is_not_a_prompt() {
        let f = facts("finished-bash-mode-after-answer");
        assert_eq!(f.user_turns, facts("finished").user_turns);
        assert_eq!(f.finished, Some(true));
    }

    /// `/exit`, its caveat and its output are not prompts, so the
    /// interrupt before them is still the last word.
    #[test]
    fn an_exit_after_an_interrupt_is_not_a_prompt_and_not_finished() {
        let (f, m) = one("finished-exit-after-interrupt");
        assert_eq!(f.user_turns, Some(1));
        assert_eq!(m.interrupted, Some(1));
        assert_eq!(f.finished, Some(false));
    }

    /// A slash command's name and local output, and a compact summary, are
    /// neither user turns nor undo barriers; a slash command's arguments
    /// are the contributor's words.
    #[test]
    fn slash_command_lines_and_compact_summaries_are_not_prompts_or_barriers() {
        let (f, m) = one("slash-commands");
        assert_eq!(f.user_turns, Some(1));
        // The restore after `/cost` has no prompt before it: the model's
        // own retry, not an undo.
        assert_eq!(m.undone, Some(0));
        assert_eq!(f.finished, Some(true));
    }

    #[test]
    fn meta_lines_and_tool_results_are_not_contributor_prompts() {
        let (f, m) = one("meta-and-results");
        assert_eq!(f.user_turns, Some(2));
        assert_eq!(m.interrupted, Some(1));
    }

    #[test]
    fn renames_count_operands_and_keep_no_command_text() {
        let (f, _) = one("renames");
        // `mv a b` is 1, `git mv -f c d e dir` is 3, `mv -n x` and an `mv`
        // that is not the first word are 0. Then the compound commands:
        // `mv h i && build` is 1, `mv j k; mv l m` is 2, `mv n o 2>/dev/null
        // || fallback` is 1, and `git mv p q | tee log > out` is 1.
        assert_eq!(f.shape.renamed_files, 9);
        let bytes = serde_json::to_vec(&f).unwrap();
        assert!(!contains(&bytes, "PRIVATE_CMD"));
    }

    #[test]
    fn a_shell_operator_or_redirection_ends_the_mv_operands() {
        assert_eq!(renamed_operands("mv a b"), 1);
        assert_eq!(renamed_operands("mv a b && cargo build"), 1);
        assert_eq!(renamed_operands("mv a b&&cargo build x y"), 1);
        assert_eq!(renamed_operands("mv a b || echo failed here"), 1);
        assert_eq!(renamed_operands("mv a b; mv c d"), 2);
        assert_eq!(renamed_operands("mv a b;mv c d e"), 3);
        assert_eq!(renamed_operands("mv a b | tee log"), 1);
        assert_eq!(renamed_operands("mv a b > out.log"), 1);
        assert_eq!(renamed_operands("mv a b >out.log"), 1);
        assert_eq!(renamed_operands("mv a b 2>/dev/null"), 1);
        assert_eq!(renamed_operands("mv a b 2> /dev/null"), 1);
        assert_eq!(renamed_operands("mv a b < in"), 1);
        assert_eq!(renamed_operands("mv a b &>/dev/null"), 1);
        assert_eq!(renamed_operands("mv a b 2>&1 | head"), 1);
        assert_eq!(renamed_operands("mv a b & mv c d"), 2);
        assert_eq!(renamed_operands("cargo build && mv a b"), 1);
        assert_eq!(renamed_operands("git mv a b c dir && git commit -m x"), 3);
        assert_eq!(renamed_operands("mv 'a b' \"c d\""), 1);
        assert_eq!(renamed_operands("mv 'a;b' c"), 1);
        assert_eq!(renamed_operands("echo mv a b"), 0);
        assert_eq!(renamed_operands("mv -n x"), 0);
        assert_eq!(renamed_operands(""), 0);
    }

    #[test]
    fn a_denied_or_failed_edit_adds_no_file_lines_or_first_edit() {
        let (f, m) = one("unapplied-edits");
        // Every call is an edit made, and the denied one is counted.
        assert_eq!(m.edits, Some(4));
        assert_eq!(m.denied, Some(1));
        // Only the applied Edit reaches the shape: one source file, and its
        // lines. The failed Write does not make the line counts unknown.
        assert_eq!(f.shape.edited_files.source, 1);
        assert_eq!(f.shape.edited_files.doc, 0);
        assert_eq!(f.shape.lines_removed, Some(1));
        assert_eq!(f.shape.lines_added, Some(2));
        // The test failed after a denied edit but before the first applied
        // one, then passed after it.
        assert_eq!(f.shape.failed_before_first_edit_then_passed, 1);
    }

    #[test]
    fn the_tdd_fixture_has_no_failure_before_the_first_edit() {
        let (f, _) = one("tdd");
        assert_eq!(f.shape.failed_before_first_edit_then_passed, 0);
        assert_eq!(f.shape.edited_files.test, 1);
        assert_ne!(f.task, InferredTask::BugFix);
        let (f, _) = one("bug-fix");
        assert_eq!(f.shape.failed_before_first_edit_then_passed, 1);
        assert_eq!(f.task, InferredTask::BugFix);
        assert_eq!(f.classifier_version, CLASSIFIER_VERSION);
    }

    fn contains(bytes: &[u8], needle: &str) -> bool {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    #[test]
    fn serialized_facts_hold_no_path_text_or_command() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../trace-commons-contributor/fixtures/insights/claude-session-facts");
        let mut read = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let Some(name) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let text = fixture_text(name).replace("MODE", "default");
            let facts = facts_of(&text);
            let bytes = serde_json::to_vec(&facts).unwrap();
            for secret in ["PRIVATE_PATH", "PRIVATE_TEXT", "PRIVATE_CMD", "src/", ".rs"] {
                assert!(!contains(&bytes, secret), "{name}: {secret}");
            }
            read += 1;
        }
        assert!(read >= 20, "{read}");
    }

    #[test]
    fn a_model_with_turns_but_no_edits_has_zero_edits_not_unknown() {
        let name = "model-without-edits";
        let f = facts(name);
        let b = model(&f, name, B);
        assert_eq!(b.edits, Some(0));
        // Nothing of b's could have been declined: known zero, not unknown.
        assert_eq!(b.denied, Some(0));
        assert_eq!(b.undone, Some(0));
        assert_eq!(b.interrupted, Some(0));
        assert!(!f.capped);
    }

    #[test]
    fn a_model_missing_from_the_labels_is_dropped_and_capped() {
        let key = key();
        let mut scan = ClaudeFactsScan::new(&key);
        for line in fixture_text("model-without-edits").lines() {
            scan.observe(&serde_json::from_str::<Value>(line).unwrap())
                .unwrap();
        }
        let f = scan.finish(&[A.to_owned()]);
        assert_eq!(f.models.len(), 1);
        assert_eq!(f.models[0].label_ix, 0);
        assert!(f.capped);
    }

    #[test]
    fn a_record_without_a_string_type_fails_as_the_series_scan_does() {
        let key = key();
        let mut scan = ClaudeFactsScan::new(&key);
        assert!(scan.observe(&json!({"message": {}})).is_err());
        assert!(scan.observe(&json!({"type": 3})).is_err());
        assert!(scan.observe(&json!({"type": "summary"})).is_ok());
    }

    /// More messages, calls and edits than the series keeps.
    fn past_the_caps() -> Vec<Value> {
        let mut rows = vec![json!({"type": "permission-mode", "permissionMode": "default"})];
        let at = "2026-01-05T10:00:00.000Z";
        for turn in 0..MAX_SERIES_TURNS + 10 {
            let calls: Vec<Value> = (0..5)
                .map(|n| {
                    json!({"type": "tool_use", "id": format!("t{turn}-{n}"), "name": "Edit",
                        "input": {"file_path": format!("f{n}.rs"),
                                  "old_string": format!("o{turn}"), "new_string": format!("n{turn}")}})
                })
                .collect();
            rows.push(json!({"type": "assistant", "timestamp": at,
                "message": {"id": format!("m{turn}"), "model": A, "content": calls}}));
            let results: Vec<Value> = (0..5)
                .map(|n| {
                    json!({"type": "tool_result", "tool_use_id": format!("t{turn}-{n}"),
                        "content": "ok", "is_error": false})
                })
                .collect();
            rows.push(json!({"type": "user", "timestamp": at,
                "message": {"role": "user", "content": results}}));
        }
        rows
    }

    #[test]
    fn the_scan_holds_no_more_than_the_caps() {
        let key = key();
        let mut scan = ClaudeFactsScan::new(&key);
        for row in past_the_caps() {
            scan.observe(&row).unwrap();
        }
        let held = scan.held();
        assert!(held.messages <= MAX_SERIES_TURNS, "{held:?}");
        assert!(held.calls <= MAX_SERIES_TOOL_CALLS, "{held:?}");
        assert!(held.edit_texts <= MAX_SERIES_TOOL_CALLS, "{held:?}");
        assert!(held.files <= MAX_SERIES_TOOL_CALLS, "{held:?}");
        assert!(held.commands <= MAX_SERIES_TOOL_CALLS, "{held:?}");
        let f = scan.finish(&[A.to_owned()]);
        assert!(f.capped);
    }
}
