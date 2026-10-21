//! Codex per-turn reader (spec section 2): one [`TurnRecord`] per response
//! and one [`ToolCallRecord`] per tool call, with the session's facts, from
//! the records of one Codex transcript.
//!
//! What is kept: counts, sizes, dates, declared model labels and keyed
//! digests. Paths, patch text, command text and call IDs are read into local
//! maps and dropped before [`CodexTurnSeriesScan::finish`] returns.
//!
//! Turns. Codex writes a `token_count` event after each response, and often
//! repeats it unchanged. A turn is a `token_count` whose running total
//! (`info.total_token_usage.total_tokens`) is greater than the previous one;
//! an equal total is a repeat and is skipped, and a smaller one makes that
//! turn's usage unknown and becomes the new baseline. An event with no
//! running total is a turn of unknown usage and leaves the baseline as it
//! was. Counters come from
//! `info.last_token_usage` by the pricing mapping (`session_pricing`'s module
//! doc): `uncached_input = input - cached - cache_write`, `cache_read =
//! cached`, `cache_write = TotalOnly(cache_write)`, `output = output`. The
//! write is never also inside `uncached_input`, so a turn's `total()` is
//! `input + output`, once.
//!
//! Each turn carries the model of the `turn_context` before it. A tool call
//! belongs to the turn whose `token_count` follows it.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, DigestKey, KeyedDigest, MAX_SERIES_TOOL_CALLS, MAX_SERIES_TURNS, ToolCallRecord,
    ToolKind, TurnRecord, UsageSeries, args_key, edit_text_key, gap_between, path_ext, path_key,
};

use super::analytics_constants::{
    CLASSIFIER_VERSION, CODEX_DENIAL_CONFIRMED, CODEX_REVIEWER_MODEL,
};
use super::models::MAX_DECLARED_MODELS;
use super::session_facts::{
    EditMode, ExtensionClassCounts, ModelFacts, SessionFacts, SessionShape, renamed_operands,
};
use super::task_classifier::{PathClass, classify, path_class};
use super::usage_evidence::safe_model;

// Codex `denied` is `None` in every mode below. Confirming a Codex denial
// marker must revisit this reader, not only flip the constant.
const _: () = assert!(!CODEX_DENIAL_CONFIRMED);

/// The `turn_aborted` reason Codex writes when the contributor interrupts.
pub const CODEX_INTERRUPTED_REASON: &str = "interrupted";

/// How a tool output starts when the contributor stopped the command.
pub const CODEX_EXEC_ABORT_PREFIX: &str = "aborted by user";

/// The approval policy under which Codex puts no call to the contributor.
const NEVER_ASK_POLICY: &str = "never";

/// The reviewer value under which Codex's automatic reviewer, not the
/// contributor, approves calls.
const AUTO_REVIEWER: &str = "auto_review";

/// Tools that wait on the contributor; their call-to-output time is not
/// active time.
const WAIT_TOOLS: [&str; 2] = ["request_user_input", "request_user_input_async"];

/// The saved per-turn rows of one Codex session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexTurnSeriesEvidence {
    /// Declared model labels, in first-seen order. `model_label_ix` indexes
    /// here.
    pub model_labels: Vec<String>,
    pub series: UsageSeries,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights_codex_series_invalid")
}

impl CodexTurnSeriesEvidence {
    pub fn validate(&self) -> Result<()> {
        let mut labels = BTreeSet::new();
        if self.model_labels.len() > MAX_DECLARED_MODELS
            || !self
                .model_labels
                .iter()
                .all(|label| safe_model(label) && labels.insert(label))
        {
            return Err(invalid());
        }
        self.series.validate().map_err(|_| invalid())?;
        for turn in &self.series.turns {
            if turn
                .model_label_ix
                .is_some_and(|ix| usize::from(ix) >= self.model_labels.len())
                || turn.msg_key.is_some()
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// One model's counts while the records are read.
#[derive(Default)]
struct ModelCounts {
    /// `apply_patch` file sections.
    edits: u32,
    patched: bool,
    shell: bool,
    undone: u32,
    interrupted: u32,
}

/// One file section of an `apply_patch` call, reduced to digests and counts.
struct PatchFile {
    /// The file after the patch: the move target when there is one.
    path: KeyedDigest,
    class: PathClass,
    moved: bool,
    /// Lines removed and added; `None` for a deleted file, whose removed
    /// lines are not in the transcript.
    lines: Option<(u32, u32)>,
    /// Each update hunk's removed and added text, as keyed digests.
    hunks: Vec<(KeyedDigest, KeyedDigest)>,
}

/// A tool call held until its output is read.
struct PendingCall {
    series_ix: Option<usize>,
    model: Option<usize>,
    at: Option<DateTime<Utc>>,
    wait: bool,
    paired: bool,
    patch: Option<Vec<PatchFile>>,
    /// A shell call's `args_key`, and whether it was made before the
    /// session's first applied patch.
    command: Option<(KeyedDigest, bool)>,
}

/// An applied update hunk, held by path to find a later restore.
struct HeldEdit {
    old: KeyedDigest,
    new: KeyedDigest,
    model: Option<usize>,
    barrier: u64,
}

/// One contributor task's span while it is open.
struct Span {
    start: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    waits_ms: i64,
    broken: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskEnd {
    Complete,
    Aborted,
}

/// Reads a Codex transcript one parsed record at a time. The caller skips
/// blank lines and fails the scan on a line that is not UTF-8 JSON.
///
/// Memory is bounded by the series caps: at most [`MAX_SERIES_TURNS`] turns
/// and [`MAX_SERIES_TOOL_CALLS`] calls, held edits, edited files and
/// commands. Past a cap the series is `truncated` and the facts `capped`.
pub struct CodexTurnSeriesScan<'k> {
    key: &'k DigestKey,
    labels: Vec<String>,
    counts: Vec<ModelCounts>,
    /// The model of the latest `turn_context`.
    model: Option<usize>,
    /// The latest `turn_context` puts no call to the contributor.
    not_asking: bool,
    labelled_contexts: u32,
    reviewer_contexts: u32,
    // Series.
    turns: Vec<TurnRecord>,
    baseline: u64,
    previous_at: Option<DateTime<Utc>>,
    series_calls: Vec<ToolCallRecord>,
    /// Calls from here on wait for the turn whose `token_count` follows.
    unassigned_from: usize,
    truncated: bool,
    // Facts.
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
    patch_calls: u32,
    applied_patches: u32,
    /// A patch call was made under a turn context that may ask.
    patch_where_asked: bool,
    /// Advanced by each task start, interrupt and undo.
    barrier: u64,
    /// An undo or interrupt came after the last task start.
    rejected_since_prompt: bool,
    /// An exec abort not yet matched by a `turn_aborted`, with its model.
    abort_pending: Option<Option<usize>>,
    last_end: Option<TaskEnd>,
    user_turns: u32,
    span: Option<Span>,
    active_ms: i64,
    active_unknown: bool,
    capped: bool,
}

impl<'k> CodexTurnSeriesScan<'k> {
    pub fn new(key: &'k DigestKey) -> Self {
        Self {
            key,
            labels: Vec::new(),
            counts: Vec::new(),
            model: None,
            not_asking: false,
            labelled_contexts: 0,
            reviewer_contexts: 0,
            turns: Vec::new(),
            baseline: 0,
            previous_at: None,
            series_calls: Vec::new(),
            unassigned_from: 0,
            truncated: false,
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
            patch_calls: 0,
            applied_patches: 0,
            patch_where_asked: false,
            barrier: 0,
            rejected_since_prompt: false,
            abort_pending: None,
            last_end: None,
            user_turns: 0,
            span: None,
            active_ms: 0,
            active_unknown: false,
            capped: false,
        }
    }

    /// One record. Fails with a fixed label on a record that is not an
    /// object with a string `type`, and ignores record types it does not
    /// read.
    pub fn observe(&mut self, row: &Value) -> Result<()> {
        let kind = row
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let at = row
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let payload = row.get("payload").unwrap_or(&Value::Null);
        let payload_type = payload.get("type").and_then(Value::as_str);
        match (kind, payload_type) {
            ("turn_context", _) => self.turn_context(payload),
            ("event_msg", Some("task_started")) => self.task_started(at),
            ("event_msg", Some("token_count")) => self.token_count(payload, at),
            ("event_msg", Some("task_complete")) => {
                self.model_event(at);
                self.last_end = Some(TaskEnd::Complete);
            }
            ("event_msg", Some("turn_aborted")) => {
                self.model_event(at);
                self.last_end = Some(TaskEnd::Aborted);
                if payload.get("reason").and_then(Value::as_str) == Some(CODEX_INTERRUPTED_REASON) {
                    // The exec abort that led here is the same interrupt.
                    let model = self.abort_pending.take().unwrap_or(self.model);
                    self.interrupt(model);
                }
            }
            ("response_item", Some("message")) => {
                if payload.get("role").and_then(Value::as_str) == Some("assistant") {
                    self.model_event(at);
                }
            }
            ("response_item", Some(kind @ ("custom_tool_call" | "function_call"))) => {
                self.model_event(at);
                self.tool_call(payload, kind == "custom_tool_call", at);
            }
            ("response_item", Some("custom_tool_call_output" | "function_call_output")) => {
                self.model_event(at);
                self.tool_output(payload, at);
            }
            ("response_item", Some(_)) => self.model_event(at),
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

    fn turn_context(&mut self, payload: &Value) {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| safe_model(model));
        self.model = model.and_then(|model| self.model_ix(model));
        if let Some(model) = model {
            self.labelled_contexts = self.labelled_contexts.saturating_add(1);
            if model == CODEX_REVIEWER_MODEL {
                self.reviewer_contexts = self.reviewer_contexts.saturating_add(1);
            }
        }
        let policy = payload.get("approval_policy").and_then(Value::as_str);
        let reviewer = payload.get("approvals_reviewer").and_then(Value::as_str);
        self.not_asking = policy == Some(NEVER_ASK_POLICY) || reviewer == Some(AUTO_REVIEWER);
    }

    fn task_started(&mut self, at: Option<DateTime<Utc>>) {
        self.flush_abort();
        self.user_turns = self.user_turns.saturating_add(1);
        self.barrier += 1;
        self.rejected_since_prompt = false;
        self.last_end = None;
        self.close_span();
        self.span = Some(Span {
            start: at,
            last: None,
            waits_ms: 0,
            broken: at.is_none(),
        });
    }

    fn token_count(&mut self, payload: &Value, at: Option<DateTime<Utc>>) {
        let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
            // A rate-limit update: no response was made.
            return;
        };
        self.model_event(at);
        let total = info
            .pointer("/total_token_usage/total_tokens")
            .and_then(Value::as_u64);
        let known = match total {
            Some(total) if total > self.baseline => {
                self.baseline = total;
                true
            }
            Some(total) if total == self.baseline => return,
            Some(total) => {
                self.baseline = total;
                false
            }
            None => false,
        };
        let counters = if known {
            info.get("last_token_usage").and_then(codex_counters)
        } else {
            None
        };
        if self.turns.len() >= MAX_SERIES_TURNS {
            self.truncated = true;
            return;
        }
        let ordinal = self.turns.len() as u32;
        self.turns.push(TurnRecord {
            ordinal,
            at,
            uncached_input: counters.map(|c| c[0]),
            cache_read: counters.map(|c| c[1]),
            cache_write: counters.map(|c| CacheWrite::TotalOnly(c[2])),
            output: counters.map(|c| c[3]),
            model_label_ix: self.model.and_then(|ix| u16::try_from(ix).ok()),
            gap_secs: if ordinal == 0 {
                None
            } else {
                gap_between(self.previous_at, at)
            },
            msg_key: None,
        });
        self.previous_at = at;
        for call in &mut self.series_calls[self.unassigned_from..] {
            call.turn_ordinal = ordinal;
        }
        self.unassigned_from = self.series_calls.len();
    }

    fn tool_call(&mut self, payload: &Value, custom: bool, at: Option<DateTime<Utc>>) {
        let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
        let tool = ToolKind::from_codex_tool_name(name);
        // A custom tool's input is free text; a function's arguments are a
        // JSON string.
        let raw = payload
            .get(if custom { "input" } else { "arguments" })
            .unwrap_or(&Value::Null);
        let input = match (custom, raw) {
            (false, Value::String(text)) => {
                serde_json::from_str(text).unwrap_or_else(|_| raw.clone())
            }
            _ => raw.clone(),
        };
        let model = self.model;
        let mut call = PendingCall {
            series_ix: None,
            model,
            at,
            wait: WAIT_TOOLS.contains(&name),
            paired: false,
            patch: None,
            command: None,
        };
        let mut first_path = None;
        match tool {
            ToolKind::ApplyPatch => {
                self.patch_calls = self.patch_calls.saturating_add(1);
                self.patch_where_asked |= !self.not_asking;
                let text = input
                    .as_str()
                    .or_else(|| input.get("input").and_then(Value::as_str))
                    .unwrap_or("");
                let parsed = parse_patch(text);
                first_path = parsed.first().map(|file| file.target());
                if let Some(model) = model {
                    let counts = &mut self.counts[model];
                    counts.patched = true;
                    counts.edits = counts
                        .edits
                        .saturating_add(u32::try_from(parsed.len()).unwrap_or(u32::MAX));
                }
                call.patch = Some(parsed.iter().map(|file| file.digest(self.key)).collect());
            }
            ToolKind::Bash => {
                if let Some(model) = model {
                    self.counts[model].shell = true;
                }
                // `exec` takes a script, not a command line, and is not
                // read for renames.
                if let Some(command) = shell_command_text(&input) {
                    self.renamed = self.renamed.saturating_add(renamed_operands(&command));
                }
                call.command = args_key(self.key, tool, &input)
                    .ok()
                    .map(|args| (args, self.applied_patches == 0));
            }
            _ => {}
        }
        if self.turns.len() >= MAX_SERIES_TURNS || self.series_calls.len() >= MAX_SERIES_TOOL_CALLS
        {
            self.truncated = true;
        } else if let Ok(args) = args_key(self.key, tool, &input) {
            call.series_ix = Some(self.series_calls.len());
            self.series_calls.push(ToolCallRecord {
                turn_ordinal: 0,
                tool,
                args_key: args,
                path_key: first_path.map(|path| path_key(self.key, path)),
                path_ext: first_path.and_then(path_ext),
                result_bytes: None,
                success: None,
                paired: false,
            });
        }
        match payload.get("call_id").and_then(Value::as_str) {
            Some(id) if self.calls.contains_key(id) => {}
            Some(id) if self.calls.len() < MAX_SERIES_TOOL_CALLS => {
                self.calls.insert(id.to_owned(), call);
            }
            Some(_) => self.capped = true,
            None => {}
        }
    }

    fn tool_output(&mut self, payload: &Value, at: Option<DateTime<Utc>>) {
        let Some(id) = payload.get("call_id").and_then(Value::as_str) else {
            return;
        };
        let Some(call) = self.calls.get_mut(id) else {
            return;
        };
        if call.paired {
            return;
        }
        call.paired = true;
        let patch = call.patch.take();
        let (series_ix, model, used_at, wait, command) =
            (call.series_ix, call.model, call.at, call.wait, call.command);
        let text = output_text(payload.get("output"));
        let success = text.as_deref().and_then(exit_code).map(|code| code == 0);
        if let Some(record) = series_ix.and_then(|ix| self.series_calls.get_mut(ix)) {
            record.paired = true;
            record.success = success;
            record.result_bytes = text.as_ref().and_then(|t| u32::try_from(t.len()).ok());
        }
        if wait {
            self.subtract_wait(used_at, at);
        }
        if text
            .as_deref()
            .is_some_and(|text| text.starts_with(CODEX_EXEC_ABORT_PREFIX))
        {
            // Counted once: by the `turn_aborted` that usually follows, or
            // at the next task start or the end of the scan.
            self.flush_abort();
            self.abort_pending = Some(model);
            self.barrier += 1;
            self.rejected_since_prompt = true;
            return;
        }
        if let Some((args, before_first_edit)) = command {
            match success {
                Some(false) if before_first_edit => {
                    if self.failed_before_edit.len() + self.passed_after_edit.len()
                        < MAX_SERIES_TOOL_CALLS
                    {
                        self.failed_before_edit.insert(args);
                    } else {
                        self.capped = true;
                    }
                }
                Some(true) if !before_first_edit && self.failed_before_edit.contains(&args) => {
                    self.passed_after_edit.insert(args);
                }
                _ => {}
            }
        }
        let Some(files) = patch else {
            return;
        };
        if success == Some(false) {
            return;
        }
        self.applied_patches = self.applied_patches.saturating_add(1);
        for file in files {
            match file.lines {
                Some((removed, added)) => {
                    self.lines_removed = self.lines_removed.saturating_add(removed);
                    self.lines_added = self.lines_added.saturating_add(added);
                }
                None => self.lines_unknown = true,
            }
            if file.moved {
                self.renamed = self.renamed.saturating_add(1);
            }
            if !self.files.contains_key(&file.path) {
                if self.files.len() < MAX_SERIES_TOOL_CALLS {
                    self.files.insert(file.path, file.class);
                } else {
                    self.capped = true;
                }
            }
            for (old, new) in file.hunks {
                self.applied_edit(file.path, old, new, model);
            }
        }
    }

    /// An applied hunk: credits the earlier hunk it restores, if a task
    /// start or interrupt came between, then holds it (Task 4's rule).
    fn applied_edit(
        &mut self,
        path: KeyedDigest,
        old: KeyedDigest,
        new: KeyedDigest,
        model: Option<usize>,
    ) {
        let barrier = self.barrier;
        let held = self.edits_by_path.entry(path).or_default();
        if let Some(ix) = held
            .iter()
            .rposition(|e| e.old == new && e.new == old && e.barrier < barrier)
        {
            let restored = held.remove(ix);
            self.held_edits -= 1;
            if let Some(model) = restored.model {
                let counts = &mut self.counts[model];
                counts.undone = counts.undone.saturating_add(1);
            }
            self.rejected_since_prompt = true;
        }
        if self.held_edits < MAX_SERIES_TOOL_CALLS {
            held.push(HeldEdit {
                old,
                new,
                model,
                barrier,
            });
            self.held_edits += 1;
        } else {
            self.capped = true;
        }
    }

    fn interrupt(&mut self, model: Option<usize>) {
        if let Some(model) = model {
            let counts = &mut self.counts[model];
            counts.interrupted = counts.interrupted.saturating_add(1);
        }
        self.barrier += 1;
        self.rejected_since_prompt = true;
    }

    fn flush_abort(&mut self) {
        if let Some(model) = self.abort_pending.take() {
            self.interrupt(model);
        }
    }

    /// A model event inside the open span.
    fn model_event(&mut self, at: Option<DateTime<Utc>>) {
        let Some(span) = &mut self.span else {
            return;
        };
        match at {
            Some(at) => span.last = Some(span.last.map_or(at, |last| last.max(at))),
            None => span.broken = true,
        }
    }

    /// Time a call waited on the contributor, from the call to its output.
    fn subtract_wait(&mut self, used: Option<DateTime<Utc>>, output: Option<DateTime<Utc>>) {
        let Some(span) = &mut self.span else {
            return;
        };
        match (used, output) {
            (Some(used), Some(output)) if output >= used => {
                span.waits_ms = span
                    .waits_ms
                    .saturating_add((output - used).num_milliseconds());
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

    /// The series and the facts, each model keyed by its index into the
    /// series' `model_labels`.
    pub fn finish(mut self) -> Result<(CodexTurnSeriesEvidence, SessionFacts)> {
        self.flush_abort();
        self.close_span();
        // Calls after the last `token_count` belong to the last turn; with
        // no turn there is nothing to hold them.
        match self.turns.last() {
            Some(last) => {
                let ordinal = last.ordinal;
                for call in &mut self.series_calls[self.unassigned_from..] {
                    call.turn_ordinal = ordinal;
                }
            }
            None if !self.series_calls.is_empty() => {
                self.series_calls.clear();
                self.truncated = true;
            }
            None => {}
        }
        let models = self
            .counts
            .iter()
            .enumerate()
            .filter_map(|(ix, counts)| {
                Some(ModelFacts {
                    label_ix: u16::try_from(ix).ok()?,
                    edits: if counts.patched {
                        Some(counts.edits)
                    } else if counts.shell {
                        // Edits made through a shell are not seen.
                        None
                    } else {
                        Some(0)
                    },
                    // While `CODEX_DENIAL_CONFIRMED` is false.
                    denied: None,
                    undone: Some(counts.undone),
                    interrupted: Some(counts.interrupted),
                })
            })
            .collect();
        // No Codex turn is `Prompting` while no denial marker is confirmed.
        let edit_mode = if self.patch_calls > 0 && !self.patch_where_asked {
            EditMode::NotPrompting
        } else {
            EditMode::Unknown
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
        let mut series = UsageSeries::default();
        for turn in self.turns {
            series.push_turn(turn);
        }
        for call in self.series_calls {
            series.push_tool_call(call);
        }
        series.truncated |= self.truncated;
        let facts = SessionFacts {
            models,
            task: classify(&shape),
            shape,
            classifier_version: CLASSIFIER_VERSION,
            // No prompt opened a span: the time was never measured.
            active_secs: (!self.active_unknown && self.user_turns > 0)
                .then(|| u32::try_from(self.active_ms / 1000).unwrap_or(u32::MAX)),
            finished: Some(
                self.patch_calls > 0
                    && self.last_end == Some(TaskEnd::Complete)
                    && !self.rejected_since_prompt,
            ),
            user_turns: Some(self.user_turns),
            edit_mode,
            capped: self.capped || series.truncated,
            delegated_unread: false,
            reviewer: self.labelled_contexts > 0
                && self.reviewer_contexts == self.labelled_contexts,
        };
        let evidence = CodexTurnSeriesEvidence {
            model_labels: self.labels,
            series,
        };
        evidence.validate()?;
        Ok((evidence, facts))
    }
}

/// `[uncached, cache_read, cache_write, output]` from a `last_token_usage`
/// object, or `None` when any field is missing, does not fit, or the cached
/// and written tokens exceed the input.
fn codex_counters(usage: &Value) -> Option<[u32; 4]> {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64);
    let input = field("input_tokens")?;
    let cached = field("cached_input_tokens")?;
    let write = field("cache_write_input_tokens")?;
    let output = field("output_tokens")?;
    let uncached = input.checked_sub(cached)?.checked_sub(write)?;
    Some([
        u32::try_from(uncached).ok()?,
        u32::try_from(cached).ok()?,
        u32::try_from(write).ok()?,
        u32::try_from(output).ok()?,
    ])
}

/// The command line of an `exec_command` (`cmd`) or `shell` (`command`, a
/// string or an argument list) call.
fn shell_command_text(input: &Value) -> Option<String> {
    if let Some(cmd) = input.get("cmd").and_then(Value::as_str) {
        return Some(cmd.to_owned());
    }
    match input.get("command")? {
        Value::String(command) => Some(command.clone()),
        Value::Array(words) => {
            let words: Option<Vec<&str>> = words.iter().map(Value::as_str).collect();
            Some(words?.join(" "))
        }
        _ => None,
    }
}

/// A tool output's text: a string, or the text parts of a list.
fn output_text(output: Option<&Value>) -> Option<String> {
    match output? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                text.push_str(part.get("text")?.as_str()?);
            }
            Some(text)
        }
        _ => None,
    }
}

/// The exit code a shell or patch output states: "Process exited with code
/// N" (`exec_command`) or a leading "Exit code: N" (`apply_patch`).
fn exit_code(text: &str) -> Option<i64> {
    let digits = |rest: &str| -> Option<i64> {
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    const PROCESS_EXITED: &str = "Process exited with code ";
    if let Some(at) = text.find(PROCESS_EXITED) {
        return digits(&text[at + PROCESS_EXITED.len()..]);
    }
    text.strip_prefix("Exit code: ").and_then(digits)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchOp {
    Add,
    Delete,
    Update,
}

/// One file section of a patch, borrowed from its text.
struct ParsedFile<'a> {
    op: PatchOp,
    path: &'a str,
    move_to: Option<&'a str>,
    added: u32,
    removed: u32,
    /// Each update hunk's removed and added lines.
    hunks: Vec<(Vec<&'a str>, Vec<&'a str>)>,
}

impl<'a> ParsedFile<'a> {
    fn target(&self) -> &'a str {
        self.move_to.unwrap_or(self.path)
    }

    fn digest(&self, key: &DigestKey) -> PatchFile {
        let target = self.target();
        PatchFile {
            path: path_key(key, target),
            class: path_class(target),
            moved: self.move_to.is_some(),
            lines: (self.op != PatchOp::Delete).then_some((self.removed, self.added)),
            hunks: self
                .hunks
                .iter()
                .filter(|(old, new)| !(old.is_empty() && new.is_empty()))
                .map(|(old, new)| {
                    (
                        edit_text_key(key, &old.join("\n")),
                        edit_text_key(key, &new.join("\n")),
                    )
                })
                .collect(),
        }
    }
}

/// The file sections of an `apply_patch` input: `*** Add File:`, `*** Delete
/// File:` and `*** Update File:` headers, an optional `*** Move to:` line,
/// `@@` hunk starts, and `+`/`-` lines.
fn parse_patch(text: &str) -> Vec<ParsedFile<'_>> {
    let mut files: Vec<ParsedFile<'_>> = Vec::new();
    for line in text.lines() {
        let header = [
            ("*** Add File: ", PatchOp::Add),
            ("*** Delete File: ", PatchOp::Delete),
            ("*** Update File: ", PatchOp::Update),
        ]
        .into_iter()
        .find_map(|(prefix, op)| line.strip_prefix(prefix).map(|path| (op, path.trim())));
        if let Some((op, path)) = header {
            files.push(ParsedFile {
                op,
                path,
                move_to: None,
                added: 0,
                removed: 0,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(target) = line.strip_prefix("*** Move to: ") {
            file.move_to = Some(target.trim());
            continue;
        }
        if line.starts_with("***") {
            continue;
        }
        if line.starts_with("@@") {
            if file.op == PatchOp::Update {
                file.hunks.push((Vec::new(), Vec::new()));
            }
            continue;
        }
        let update = file.op == PatchOp::Update;
        if update && file.hunks.is_empty() {
            file.hunks.push((Vec::new(), Vec::new()));
        }
        if let Some(added) = line.strip_prefix('+') {
            file.added = file.added.saturating_add(1);
            if let (true, Some(hunk)) = (update, file.hunks.last_mut()) {
                hunk.1.push(added);
            }
        } else if let Some(removed) = line.strip_prefix('-') {
            file.removed = file.removed.saturating_add(1);
            if let (true, Some(hunk)) = (update, file.hunks.last_mut()) {
                hunk.0.push(removed);
            }
        }
    }
    files
}

#[cfg(test)]
#[path = "codex_series_tests.rs"]
mod tests;
