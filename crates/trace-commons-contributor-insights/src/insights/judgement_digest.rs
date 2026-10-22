//! The judgement digest: what a session sends to the Insights judgement
//! service (private-judgements addendum, "What is sent"; delivery step 1).
//!
//! # Gated on owner decision O1
//!
//! What may leave the machine is not decided (addendum, "What is sent (open
//! decision O1)"). This module only builds bytes. Nothing here sends, and no
//! caller may send a digest until O1 is decided. The shape built here is
//! option B's; under option A a caller passes no prompts and no final
//! message, and under option C it does so for every session not approved.
//!
//! # Pure, and the builder scrubs
//!
//! [`build_judgement_digest`] is pure: no I/O, no clock, no randomness, no
//! key, no environment read. The same input gives the same bytes and the same
//! SHA-256. The home and working directories arrive in [`ScrubContext`]; the
//! builder never looks them up.
//!
//! Prompts and the final message arrive **raw**, and the builder runs the
//! scrub itself (addendum, "The scrub"), so its order cannot be got wrong by
//! a caller:
//!
//! 1. The deterministic redactor, built only through
//!    `DeterministicTraceRedactor::deterministic_only` with the home and
//!    working directories as known path prefixes. It attaches no network
//!    filter, whatever the environment or the contributor's configuration
//!    says.
//! 2. On the whole text, before any cut, so a secret straddling the cut is
//!    masked whole. The final message is redacted whole and then its first
//!    line taken.
//! 3. [`mask_excerpt`]: emails, URLs, path-like tokens, the repository name
//!    and its long parts, runs of capitalised words, and long digit runs.
//! 4. The residual gate: the redactor's detector runs again over each masked
//!    text, and a text it still flags is dropped and counted in
//!    `excerpts_dropped`. If the residual risk of the texts that would be
//!    sent (the selected prompts and the final line, not every prompt) is
//!    not Low, no text is sent at all: counts only, as option A, with every
//!    selected excerpt counted as dropped.
//! 5. Then the cut.
//!
//! Words none of these rules recognise still pass: a lower-case client or
//! product name, for example. That residual is what O1 weighs.
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
//! # Hash
//!
//! [`BuiltDigest::sha256`] is SHA-256 over exactly [`BuiltDigest::bytes`],
//! the bytes sent. That is the `digest_sha256` the service signs and the
//! daemon checks against what it sent (service design, section 4).
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
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, DeterministicTraceRedactor, RedactionReport, residual_risk_basis,
};

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
    /// Every contributor prompt in session order, raw. The builder scrubs
    /// the ones it selects.
    pub prompts: &'a [String],
    /// The model's final message, raw. The builder scrubs it whole and keeps
    /// its first line.
    pub final_message: Option<&'a str>,
    /// The directories the scrub masks.
    pub scrub: ScrubContext<'a>,
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

// ---- the scrub (addendum, "The scrub") ----

/// The directories the scrub masks: the home directory and the session's
/// recorded working directory. Passed in so the builder stays pure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrubContext<'a> {
    pub home_dir: Option<&'a str>,
    pub working_dir: Option<&'a str>,
}

/// Repository name parts shorter than this are not masked on their own.
const REPO_PART_MIN_CHARS: usize = 4;
/// Digit runs this long or longer become `<number>`.
const LONG_NUMBER_DIGITS: usize = 6;

/// The redactor the scrub uses. Deterministic only: no privacy-filter
/// adapter is attached, whatever the environment says.
pub(crate) fn scrub_redactor(scrub: &ScrubContext<'_>) -> DeterministicTraceRedactor {
    let prefixes = [scrub.home_dir, scrub.working_dir]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect();
    DeterministicTraceRedactor::deterministic_only(prefixes)
}

/// The terms masked as `<repo>`: the working directory's last component and
/// its parts split on `-`, `_` and `.` that are at least
/// [`REPO_PART_MIN_CHARS`] characters long. Lower-cased, longest first.
pub fn repo_terms(working_dir: Option<&str>) -> Vec<String> {
    let Some(name) =
        working_dir.and_then(|dir| dir.split(['/', '\\']).rev().find(|part| !part.is_empty()))
    else {
        return Vec::new();
    };
    let mut terms: Vec<String> = std::iter::once(name)
        .chain(
            name.split(['-', '_', '.'])
                .filter(|part| part.chars().count() >= REPO_PART_MIN_CHARS),
        )
        .map(str::to_ascii_lowercase)
        .collect();
    terms.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    terms.dedup();
    terms
}

/// Replaces every ASCII-case-insensitive occurrence of `needle`, which is
/// already lower-cased. ASCII lower-casing keeps every byte offset, so a
/// match in the lowered copy falls on the same char boundaries in `text`.
fn replace_ascii_ci(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(needle) {
        let start = at + found;
        out.push_str(&text[at..start]);
        out.push_str(with);
        at = start + needle.len();
    }
    out.push_str(&text[at..]);
    out
}

const LEADING_WRAP: &[char] = &['(', '[', '{', '"', '\'', '`', '<'];
const TRAILING_WRAP: &[char] = &[
    ')', ']', '}', '"', '\'', '`', '>', ',', ';', ':', '!', '?', '.',
];

/// Splits a whitespace-free token into its leading wrap, its core and its
/// trailing wrap.
fn split_wrap(token: &str) -> (&str, &str, &str) {
    let core_start = token.len() - token.trim_start_matches(LEADING_WRAP).len();
    let rest = &token[core_start..];
    let core = rest.trim_end_matches(TRAILING_WRAP);
    (&token[..core_start], core, &rest[core.len()..])
}

/// Any `@` with text on both sides. Over-masks `user@host` forms too.
fn looks_like_email(core: &str) -> bool {
    core.split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
}

fn looks_like_url(core: &str) -> bool {
    let lower = core.to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("www.")
}

/// A token whose path separators have text on both sides of at least one of
/// them, or one ending in a file extension: `name.ext`, the extension 1 to 8
/// ASCII letters and digits with at least one letter.
fn looks_like_path(core: &str) -> bool {
    let parts = core.split(['/', '\\']).filter(|p| !p.is_empty()).count();
    if parts >= 2 {
        return true;
    }
    core.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    })
}

fn path_placeholder(core: &str) -> String {
    let class = match ExtensionClass::from(path_class(core)) {
        ExtensionClass::Test => "test",
        ExtensionClass::Doc => "doc",
        ExtensionClass::Source => "source",
        ExtensionClass::Config => "config",
    };
    format!("<path:{class}>")
}

fn mask_token(token: &str) -> String {
    let (pre, core, post) = split_wrap(token);
    let masked = if core.is_empty() {
        return token.to_owned();
    } else if looks_like_email(core) {
        "<email>".to_owned()
    } else if looks_like_url(core) {
        "<url>".to_owned()
    } else if looks_like_path(core) {
        path_placeholder(core)
    } else {
        return token.to_owned();
    };
    format!("{pre}{masked}{post}")
}

/// A capitalised word: an upper-case first letter, then at least one more
/// letter, apostrophe or hyphen, and nothing else.
fn is_capitalised_word(core: &str) -> bool {
    let mut chars = core.chars();
    chars.next().is_some_and(char::is_uppercase)
        && core.chars().count() >= 2
        && chars.all(|c| c.is_alphabetic() || matches!(c, '\'' | '\u{2019}' | '-'))
}

/// Splits text into alternating whitespace and token pieces, each tagged
/// with whether it is whitespace.
fn pieces(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        if let Some(prev) = in_space {
            if prev != space {
                out.push((prev, &text[start..i]));
                start = i;
            }
        }
        in_space = Some(space);
    }
    if let Some(prev) = in_space {
        out.push((prev, &text[start..]));
    }
    out
}

/// Replaces each run of two or more capitalised words with `<name>`. A run
/// is broken by a line break, by any punctuation after a word, or by a
/// leading wrap before one. A run at the start of a sentence is masked too:
/// the addendum masks runs that do not start one, and this is the safe
/// superset.
fn mask_name_runs(text: &str) -> String {
    let pieces = pieces(text);
    let is_word = |piece: &str| is_capitalised_word(split_wrap(piece).1);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < pieces.len() {
        let (space, piece) = pieces[i];
        if space || !is_word(piece) {
            out.push_str(piece);
            i += 1;
            continue;
        }
        let mut end = i;
        while split_wrap(pieces[end].1).2.is_empty() {
            let gap_ok = matches!(pieces.get(end + 1), Some(&(true, gap)) if !gap.contains('\n'));
            let next_ok = matches!(
                pieces.get(end + 2),
                Some(&(false, next)) if split_wrap(next).0.is_empty() && is_word(next)
            );
            if !(gap_ok && next_ok) {
                break;
            }
            end += 2;
        }
        if end == i {
            out.push_str(piece);
        } else {
            out.push_str(split_wrap(pieces[i].1).0);
            out.push_str("<name>");
            out.push_str(split_wrap(pieces[end].1).2);
        }
        i = end + 1;
    }
    out
}

/// Replaces each run of [`LONG_NUMBER_DIGITS`] or more ASCII digits with
/// `<number>`.
fn mask_long_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        if digits.len() >= LONG_NUMBER_DIGITS {
            out.push_str("<number>");
        } else {
            out.push_str(digits);
        }
        digits.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut digits, &mut out);
            out.push(c);
        }
    }
    flush(&mut digits, &mut out);
    out
}

/// The excerpt masker (addendum, "The scrub", step 3). It runs after the
/// redactor, which masks only secrets and known absolute path prefixes.
///
/// Path-like tokens go first, so a repository name inside a path leaves with
/// the path's class only, then the repository terms, then name runs and long
/// numbers.
pub fn mask_excerpt(text: &str, repo_terms: &[String]) -> String {
    let mut text: String = pieces(text)
        .into_iter()
        .map(|(space, piece)| {
            if space {
                piece.to_owned()
            } else {
                mask_token(piece)
            }
        })
        .collect();
    for term in repo_terms {
        text = replace_ascii_ci(&text, term, "<repo>");
    }
    mask_long_numbers(&mask_name_runs(&text))
}

/// One text after scrub steps 1 to 3, and the redactor's report on it.
pub(crate) struct ScrubbedText {
    pub(crate) text: String,
    pub(crate) report: RedactionReport,
}

/// Scrub steps 1 to 3 over one whole text.
pub(crate) fn scrub_text(
    redactor: &DeterministicTraceRedactor,
    repo_terms: &[String],
    raw: &str,
) -> ScrubbedText {
    let (redacted, report) = redactor.redact_text(raw);
    ScrubbedText {
        text: mask_excerpt(&redacted, repo_terms),
        report,
    }
}

/// The content flags are what this send decides, so they do not count
/// towards the risk here. The rest of the residual-risk rules apply as the
/// envelope path applies them.
fn no_content_consent() -> ConsentMetadata {
    ConsentMetadata {
        policy_version: String::new(),
        scopes: Vec::new(),
        message_text_included: false,
        tool_payloads_included: false,
        correction_included: false,
        routing_metadata_included: false,
        revocable: false,
    }
}

/// Low residual risk: no condition that raises the protocol's residual risk
/// above Low holds. Judged through the public `residual_risk_basis`, which
/// the protocol pins to its residual-risk rules.
fn is_low_risk(report: &RedactionReport) -> bool {
    residual_risk_basis(&no_content_consent(), report, None).is_empty()
}

/// Scrub step 4, per text: the detector runs again over the masked text, and
/// anything it still finds drops the text.
pub(crate) fn passes_residual_gate(redactor: &DeterministicTraceRedactor, masked: &str) -> bool {
    let (_, report) = redactor.redact_text(masked);
    is_low_risk(&report)
}

fn merge_reports(into: &mut RedactionReport, from: &RedactionReport) {
    for (label, n) in &from.counts {
        let slot = into.counts.entry(label.clone()).or_insert(0);
        *slot = slot.saturating_add(*n);
    }
    for label in &from.pii_labels_present {
        if !into.pii_labels_present.contains(label) {
            into.pii_labels_present.push(label.clone());
        }
    }
    into.blocked_secret_detected |= from.blocked_secret_detected;
    into.key_finding_detected |= from.key_finding_detected;
    into.coverage_incomplete |= from.coverage_incomplete;
}

/// The texts a digest may carry after the scrub, before the cut.
struct ScrubbedSession {
    /// `(prompt index, masked text)` for each selected prompt that passed.
    excerpts: Vec<(usize, String)>,
    final_line: Option<String>,
    /// Selected prompts the gate dropped.
    dropped: u32,
}

fn scrub_session(input: &DigestInput<'_>, selected: &[usize]) -> ScrubbedSession {
    if selected.is_empty() && input.final_message.is_none() {
        return ScrubbedSession {
            excerpts: Vec::new(),
            final_line: None,
            dropped: 0,
        };
    }
    let redactor = scrub_redactor(&input.scrub);
    let repo = repo_terms(input.scrub.working_dir);
    let mut session_report = RedactionReport::default();
    let mut excerpts = Vec::with_capacity(selected.len());
    let mut dropped = 0u32;
    for &ix in selected {
        let scrubbed = scrub_text(&redactor, &repo, &input.prompts[ix]);
        merge_reports(&mut session_report, &scrubbed.report);
        if passes_residual_gate(&redactor, &scrubbed.text) {
            excerpts.push((ix, scrubbed.text));
        } else {
            dropped = dropped.saturating_add(1);
        }
    }
    let final_line = input.final_message.and_then(|message| {
        let scrubbed = scrub_text(&redactor, &repo, message);
        merge_reports(&mut session_report, &scrubbed.report);
        let line = first_line(&scrubbed.text).to_owned();
        passes_residual_gate(&redactor, &line).then_some(line)
    });
    if !is_low_risk(&session_report) {
        return ScrubbedSession {
            excerpts: Vec::new(),
            final_line: None,
            dropped: to_u32(selected.len()),
        };
    }
    ScrubbedSession {
        excerpts,
        final_line,
        dropped,
    }
}

/// Builds the digest, its bytes and its hash. Pure.
pub fn build_judgement_digest(input: &DigestInput<'_>) -> Result<BuiltDigest, DigestError> {
    let events = sorted_events(input.events);
    let events_truncated = events.len() > MAX_DIGEST_EVENTS;
    let selected = select_prompts(input.prompts.len(), &events);
    let scrubbed = scrub_session(input, &selected);
    let excerpts = scrubbed
        .excerpts
        .into_iter()
        .map(|(ix, text)| PromptExcerpt {
            ix: to_u32(ix),
            text: truncate_chars(&text, PROMPT_EXCERPT_CHARS),
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
        excerpts_dropped: scrubbed.dropped,
        final_line: scrubbed
            .final_line
            .map(|line| truncate_chars(&line, FINAL_LINE_CHARS)),
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
