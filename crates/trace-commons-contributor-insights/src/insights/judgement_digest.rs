//! The judgement digest: what a session sends to the Insights judgement
//! service (private-judgements addendum, "What is sent"; delivery step 1).
//!
//! # Pure, and the caller scrubs
//!
//! [`build_judgement_digest`] is pure: no I/O, no clock, no randomness, no
//! key. The same input gives the same bytes and the same SHA-256.
//!
//! **The caller must run the privacy filter first.** Contributor prompts and
//! the final message line arrive as [`Scrubbed`] text, and the builder trusts
//! them as already scrubbed: it selects and truncates them and never inspects
//! or rewrites their content. Raw transcript text must never reach this
//! module.
//!
//! Everything else in the digest is a count, a fixed label, or a position.
//! Inputs that could carry text are reduced on the way in:
//!
//! - an edited file is reduced to its [`ExtensionClass`] by
//!   [`extension_class_of`], so a path passed where an extension is expected
//!   leaves only its class;
//! - the usage series contributes per-tool-kind counts and per-turn outcome
//!   counts only. Its keyed digests, file extensions and timestamps are never
//!   copied;
//! - per-model facts are summed, so no model label index is sent.
//!
//! No file contents, diffs, paths, repository names or identifiers appear in
//! the digest.
//!
//! # Rules (owner-approved values)
//!
//! - At most [`MAX_PROMPT_EXCERPTS`] prompts, each cut to
//!   [`PROMPT_EXCERPT_CHARS`] characters. With more, the first
//!   [`EDGE_PROMPTS`], the last [`EDGE_PROMPTS`], then for each marked event
//!   in session order the [`EVENT_WINDOW_PROMPTS`] prompts up to and
//!   including the prompt whose turn carried the event, nearest first, until
//!   the cap (events at one prompt are taken by kind, so input order never
//!   changes the bytes). Any slots still free are filled from the middle
//!   prompts not yet chosen, at evenly spaced positions, so a long session
//!   always sends 12. Deduplicated, in session order.
//! - The final message's first line, cut to [`FINAL_LINE_CHARS`] characters.
//! - Truncation counts Unicode scalar values and never splits a codepoint.
//! - The serialized digest is at most [`DIGEST_BYTE_CAP`] bytes. Over it,
//!   whole excerpts are dropped from the middle outward and the number
//!   dropped is recorded. No field is cut mid-string. If the digest is still
//!   over the cap with no excerpts left, it is refused whole
//!   ([`DigestError::OverCap`]); the per-turn and event caps below keep that
//!   case out of reach.
//!
//! The digest is not wired into the daemon here; that is a later delivery
//! step.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::insights_usage_series::{ToolKind, UsageSeries};

use super::session_facts::{EditMode, InferredTask, ModelFacts, SessionFacts};
use super::task_classifier::{PathClass, path_class};

/// The digest's wire version.
pub const JUDGEMENT_DIGEST_VERSION: u32 = 1;
/// Most prompt excerpts a digest carries.
pub const MAX_PROMPT_EXCERPTS: usize = 12;
/// Characters each prompt excerpt is cut to.
pub const PROMPT_EXCERPT_CHARS: usize = 240;
/// Prompts taken from each end of a long session.
pub const EDGE_PROMPTS: usize = 4;
/// Prompts taken before each marked event.
pub const EVENT_WINDOW_PROMPTS: usize = 4;
/// Characters the final message line is cut to.
pub const FINAL_LINE_CHARS: usize = 160;
/// Hard cap on the serialized digest.
pub const DIGEST_BYTE_CAP: usize = 8 * 1024;
/// Most per-turn outcomes a digest carries; turns past it set
/// `turn_outcomes_truncated`.
pub const MAX_TURN_OUTCOMES: usize = 64;
/// Most marked events a digest carries; events past it set
/// `events_truncated`.
pub const MAX_DIGEST_EVENTS: usize = 64;

/// Text the caller has already run through the privacy filter. The builder
/// trusts it and never inspects it; constructing one is the caller's claim
/// that it is scrubbed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrubbed(pub String);

/// The class of an edited file. The only file information a digest carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionClass {
    Test,
    Doc,
    Source,
    Config,
}

impl From<PathClass> for ExtensionClass {
    fn from(class: PathClass) -> Self {
        match class {
            PathClass::Test => Self::Test,
            PathClass::Doc => Self::Doc,
            PathClass::Source => Self::Source,
            PathClass::Config => Self::Config,
        }
    }
}

/// Reduces an extension (`md`, `.md`) or a whole path to its class. A path
/// passed here leaves nothing but its class.
pub fn extension_class_of(input: &str) -> ExtensionClass {
    let looks_like_path = input.contains('/') || input.contains('\\');
    let class = if looks_like_path {
        path_class(input)
    } else {
        // A bare `md` has no dot, and `path_class` would read it as a file
        // name; give it a stem. A dotted name (`.md`, `notes.md`) keeps its
        // last extension.
        path_class(&format!("x.{}", input.trim_start_matches('.')))
    };
    class.into()
}

/// What kind of contributor pushback a marked event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Denial,
    Interrupt,
    Undo,
}

/// A denial, interrupt or undo, placed by the zero-based index of the
/// contributor prompt whose turn carried it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventMarker {
    pub prompt_ix: u32,
    pub kind: EventKind,
}

/// Everything the builder reads.
#[derive(Debug, Clone, Copy)]
pub struct DigestInput<'a> {
    pub facts: &'a SessionFacts,
    pub series: Option<&'a UsageSeries>,
    /// Every contributor prompt in session order, already scrubbed.
    pub prompts: &'a [Scrubbed],
    /// The first line of the model's final message, already scrubbed.
    pub final_line: Option<&'a Scrubbed>,
    /// The edited files' extensions (or paths, which are reduced to class).
    pub edited_extensions: &'a [&'a str],
    pub events: &'a [EventMarker],
}

/// Session counts, from the stored facts and the usage series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestCounts {
    pub task: InferredTask,
    pub classifier_version: u32,
    pub user_turns: Option<u32>,
    pub active_secs: Option<u32>,
    pub finished: Option<bool>,
    pub edit_mode: EditMode,
    /// Summed over the declared models. Calls whose model was never read
    /// are in no model's facts and are not counted. `None` when any model's
    /// count is unknown, or when no model was declared.
    pub edits: Option<u32>,
    pub denied: Option<u32>,
    pub undone: Option<u32>,
    pub interrupted: Option<u32>,
    /// Distinct edited files per class, as the counter pass stored them.
    pub stored_edited_files: FileClassCounts,
    pub renamed_files: u32,
    pub lines_added: Option<u32>,
    pub lines_removed: Option<u32>,
    pub failed_before_first_edit_then_passed: u32,
    pub capped: bool,
    pub delegated_unread: bool,
    /// Tool calls per kind; empty when no series was read.
    pub tool_kinds: BTreeMap<ToolKind, u32>,
    pub commands_passed: u32,
    pub commands_failed: u32,
    pub commands_unknown: u32,
    pub series_truncated: bool,
}

/// One turn's edit and command outcomes, serialized as
/// `[ordinal, edits, edits_failed, commands_passed, commands_failed]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOutcome(pub u32, pub u32, pub u32, pub u32, pub u32);

/// Distinct edited files per class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileClassCounts {
    pub test: u32,
    pub doc: u32,
    pub source: u32,
    pub config: u32,
}

/// One selected prompt, with its zero-based position in the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptExcerpt {
    pub ix: u32,
    pub text: String,
}

/// The digest sent to the judgement service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementDigest {
    pub schema_version: u32,
    pub counts: DigestCounts,
    pub turn_outcomes: Vec<TurnOutcome>,
    pub turn_outcomes_truncated: bool,
    pub edited_file_classes: FileClassCounts,
    pub events: Vec<EventMarker>,
    pub events_truncated: bool,
    pub prompts_total: u32,
    pub excerpts: Vec<PromptExcerpt>,
    /// Excerpts dropped to fit [`DIGEST_BYTE_CAP`].
    pub excerpts_dropped: u32,
    pub final_line: Option<String>,
}

/// A built digest, the exact bytes to send, and their SHA-256. The caller
/// sends `bytes` and logs only the hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltDigest {
    pub digest: JudgementDigest,
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
}

impl BuiltDigest {
    pub fn sha256_hex(&self) -> String {
        hex::encode(self.sha256)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DigestError {
    #[error("insights_judgement_digest_over_cap")]
    OverCap,
    #[error("insights_judgement_digest_unserializable")]
    Serialize,
}

/// The prompt indices to send, in session order (see the module rules).
pub fn select_prompts(prompt_count: usize, events: &[EventMarker]) -> Vec<usize> {
    if prompt_count <= MAX_PROMPT_EXCERPTS {
        return (0..prompt_count).collect();
    }
    let mut picked: BTreeSet<usize> = (0..EDGE_PROMPTS)
        .chain(prompt_count - EDGE_PROMPTS..prompt_count)
        .collect();
    'events: for event in sorted_events(events) {
        let at = event.prompt_ix as usize;
        if at >= prompt_count {
            continue;
        }
        let first = (at + 1).saturating_sub(EVENT_WINDOW_PROMPTS);
        for ix in (first..=at).rev() {
            if picked.len() >= MAX_PROMPT_EXCERPTS {
                break 'events;
            }
            picked.insert(ix);
        }
    }
    // Slots the events left go to unchosen middle prompts, evenly spaced:
    // the j-th of `k` picks is the candidate at the centre of the j-th of
    // `k` equal spans. The middle holds more than `k` candidates (it has at
    // least five prompts and every pick already in it took a slot), so the
    // picks are distinct.
    let slots = MAX_PROMPT_EXCERPTS.saturating_sub(picked.len());
    if slots > 0 {
        let middle: Vec<usize> = (EDGE_PROMPTS..prompt_count - EDGE_PROMPTS)
            .filter(|ix| !picked.contains(ix))
            .collect();
        for j in 0..slots.min(middle.len()) {
            picked.insert(middle[(2 * j + 1) * middle.len() / (2 * slots)]);
        }
    }
    picked.into_iter().collect()
}

/// Events in session order, then by kind, whatever order the caller used.
fn sorted_events(events: &[EventMarker]) -> Vec<EventMarker> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|event| (event.prompt_ix, event.kind));
    sorted
}

/// Cuts `text` to at most `max` characters, on a codepoint boundary.
pub fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

fn first_line(text: &str) -> &str {
    let line = text.split('\n').next().unwrap_or("");
    line.strip_suffix('\r').unwrap_or(line)
}

/// Sums one per-model count; `None` when any model's is unknown or there
/// is no model, never a zero nobody measured.
fn sum_models(facts: &SessionFacts, count: impl Fn(&ModelFacts) -> Option<u32>) -> Option<u32> {
    if facts.models.is_empty() {
        return None;
    }
    facts.models.iter().try_fold(0u32, |total, model| {
        Some(total.saturating_add(count(model)?))
    })
}

fn counts(facts: &SessionFacts, series: Option<&UsageSeries>) -> DigestCounts {
    let mut tool_kinds = BTreeMap::new();
    let (mut passed, mut failed, mut unknown) = (0u32, 0u32, 0u32);
    if let Some(series) = series {
        for call in &series.tool_calls {
            let n: &mut u32 = tool_kinds.entry(call.tool).or_default();
            *n = n.saturating_add(1);
            if call.tool == ToolKind::Bash {
                let slot = match call.success {
                    Some(true) => &mut passed,
                    Some(false) => &mut failed,
                    None => &mut unknown,
                };
                *slot = slot.saturating_add(1);
            }
        }
    }
    DigestCounts {
        task: facts.task,
        classifier_version: facts.classifier_version,
        user_turns: facts.user_turns,
        active_secs: facts.active_secs,
        finished: facts.finished,
        edit_mode: facts.edit_mode,
        edits: sum_models(facts, |m| m.edits),
        denied: sum_models(facts, |m| m.denied),
        undone: sum_models(facts, |m| m.undone),
        interrupted: sum_models(facts, |m| m.interrupted),
        stored_edited_files: FileClassCounts {
            test: facts.shape.edited_files.test,
            doc: facts.shape.edited_files.doc,
            source: facts.shape.edited_files.source,
            config: facts.shape.edited_files.config,
        },
        renamed_files: facts.shape.renamed_files,
        lines_added: facts.shape.lines_added,
        lines_removed: facts.shape.lines_removed,
        failed_before_first_edit_then_passed: facts.shape.failed_before_first_edit_then_passed,
        capped: facts.capped,
        delegated_unread: facts.delegated_unread,
        tool_kinds,
        commands_passed: passed,
        commands_failed: failed,
        commands_unknown: unknown,
        series_truncated: series.is_some_and(|s| s.truncated),
    }
}

/// Per-turn outcomes for turns with an edit or a command, in ordinal order,
/// at most [`MAX_TURN_OUTCOMES`]; the flag is set when more were dropped.
fn turn_outcomes(series: Option<&UsageSeries>) -> (Vec<TurnOutcome>, bool) {
    let Some(series) = series else {
        return (Vec::new(), false);
    };
    let mut by_turn: BTreeMap<u32, TurnOutcome> = BTreeMap::new();
    for call in &series.tool_calls {
        let edit = call.tool.is_edit();
        let command = call.tool == ToolKind::Bash;
        if !edit && !command {
            continue;
        }
        let row =
            by_turn
                .entry(call.turn_ordinal)
                .or_insert(TurnOutcome(call.turn_ordinal, 0, 0, 0, 0));
        if edit {
            row.1 = row.1.saturating_add(1);
            if call.success == Some(false) {
                row.2 = row.2.saturating_add(1);
            }
        } else {
            match call.success {
                Some(true) => row.3 = row.3.saturating_add(1),
                Some(false) => row.4 = row.4.saturating_add(1),
                None => {}
            }
        }
    }
    let truncated = by_turn.len() > MAX_TURN_OUTCOMES;
    let rows = by_turn.into_values().take(MAX_TURN_OUTCOMES).collect();
    (rows, truncated)
}

fn file_classes(extensions: &[&str]) -> FileClassCounts {
    let mut counts = FileClassCounts::default();
    for ext in extensions {
        let slot = match extension_class_of(ext) {
            ExtensionClass::Test => &mut counts.test,
            ExtensionClass::Doc => &mut counts.doc,
            ExtensionClass::Source => &mut counts.source,
            ExtensionClass::Config => &mut counts.config,
        };
        *slot = slot.saturating_add(1);
    }
    counts
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Builds the digest, its bytes and its hash. Pure.
pub fn build_judgement_digest(input: &DigestInput<'_>) -> Result<BuiltDigest, DigestError> {
    let events = sorted_events(input.events);
    let events_truncated = events.len() > MAX_DIGEST_EVENTS;
    let excerpts = select_prompts(input.prompts.len(), &events)
        .into_iter()
        .map(|ix| PromptExcerpt {
            ix: to_u32(ix),
            text: truncate_chars(&input.prompts[ix].0, PROMPT_EXCERPT_CHARS),
        })
        .collect();
    let (turn_outcomes, turn_outcomes_truncated) = turn_outcomes(input.series);
    let mut digest = JudgementDigest {
        schema_version: JUDGEMENT_DIGEST_VERSION,
        counts: counts(input.facts, input.series),
        turn_outcomes,
        turn_outcomes_truncated,
        edited_file_classes: file_classes(input.edited_extensions),
        events: events.into_iter().take(MAX_DIGEST_EVENTS).collect(),
        events_truncated,
        prompts_total: to_u32(input.prompts.len()),
        excerpts,
        excerpts_dropped: 0,
        final_line: input
            .final_line
            .map(|line| truncate_chars(first_line(&line.0), FINAL_LINE_CHARS)),
    };
    loop {
        let bytes = serde_json::to_vec(&digest).map_err(|_| DigestError::Serialize)?;
        if bytes.len() <= DIGEST_BYTE_CAP {
            let sha256 = Sha256::digest(&bytes).into();
            return Ok(BuiltDigest {
                digest,
                bytes,
                sha256,
            });
        }
        if digest.excerpts.is_empty() {
            return Err(DigestError::OverCap);
        }
        // Repeatedly removing the middle one drops from the middle outward,
        // alternating sides.
        digest.excerpts.remove(digest.excerpts.len() / 2);
        digest.excerpts_dropped += 1;
    }
}

#[cfg(test)]
#[path = "judgement_digest_tests.rs"]
mod tests;
