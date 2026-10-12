//! On-device failure detectors for one session: frontier failure missions,
//! slice F1 (`docs/superpowers/specs/2026-10-12-frontier-failure-missions-design.md`).
//!
//! Each detector reads the adapter-neutral events of a loaded session and
//! names either a [`TraceFailureMode`] or one of two signals with no mode of
//! its own (a revert, a takeover). The result is a *candidate*: it stays on
//! this device. It reaches an envelope's `failure_modes` only after the
//! contributor confirms it (slice F2), and nothing in this crate does that
//! yet. [`FailureCandidate`] deliberately does not implement `Serialize`, so
//! it cannot be dropped into a payload without a deliberate change, and
//! `tests::no_envelope_path_reads_the_detectors` holds every other module
//! to not calling it.
//!
//! Pure: no I/O, no clock, no copy. The retry and edit-fail-edit detectors
//! reuse the Patterns logic (`patterns::retry_links`,
//! `patterns::edit_fail_edit_cycles`, `patterns::session_patterns` for
//! re-reads) over call rows built here from the events, under a throwaway
//! digest key that never leaves this function.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use serde_json::Value;
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, DigestKey, ToolCallRecord, ToolKind, TurnRecord, UsageSeries, args_key, path_key,
};
use trace_commons_protocol::trace_contribution::TraceFailureMode;

use super::markers::{MarkerKind, usage_markers};
use super::outcomes::TestReportEvidence;
use super::patterns::{edit_fail_edit_cycles, is_cycle_edit, retry_links, session_patterns};
use crate::source::{SessionEvent, SessionEventKind};

/// Identical calls, counting the first, that make a retry loop.
pub const RETRY_LOOP_CALLS: usize = 3;
/// Edit, failed command, edit rounds on one file that make an edit loop.
pub const EDIT_LOOP_ROUNDS: usize = 3;

/// What fired. Ordered for a stable set; the order means nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureSignal {
    /// One call, same tool and arguments, made [`RETRY_LOOP_CALLS`] or more
    /// times in a row with only lookups between.
    RetriedIdenticalCalls,
    /// [`EDIT_LOOP_ROUNDS`] or more edit, failed command, edit rounds on
    /// one file.
    EditFailEditLoop,
    /// The session's last verification -- a test or build command, or an
    /// imported test report -- failed.
    LastVerificationFailed,
    /// The person interrupted a model turn and then wrote a new instruction.
    InterruptedAndRewritten,
    /// A `git checkout`, `git restore` or `git reset --hard` undid files the
    /// model had edited. No failure mode of its own.
    Reverted,
    /// The last change to a file the model edited was made outside the
    /// model. No failure mode of its own.
    TakenOver,
    /// After context crossed the long-context threshold, the model read
    /// again a file it had already read.
    RereadAfterLongContext,
}

/// One session's detector output, held on this device. Not `Serialize`, on
/// purpose -- see the module comment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureCandidate {
    pub failure_modes: BTreeSet<TraceFailureMode>,
    pub signals: BTreeSet<FailureSignal>,
}

impl FailureCandidate {
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    pub fn reverted(&self) -> bool {
        self.signals.contains(&FailureSignal::Reverted)
    }

    pub fn taken_over(&self) -> bool {
        self.signals.contains(&FailureSignal::TakenOver)
    }

    fn fire(&mut self, signal: FailureSignal, mode: Option<TraceFailureMode>) {
        self.signals.insert(signal);
        if let Some(mode) = mode {
            self.failure_modes.insert(mode);
        }
    }
}

/// Run every F1 detector over one session's events. `test_reports` are the
/// test reports imported for this session, if any (`insights::outcomes`).
pub fn detect_failure_signals(
    events: &[SessionEvent],
    test_reports: &[TestReportEvidence],
) -> FailureCandidate {
    let calls = model_calls(events);
    let mut found = FailureCandidate::default();

    let records: Vec<ToolCallRecord> = calls.iter().map(|call| call.record.clone()).collect();
    if retried_identical_calls(&records) {
        found.fire(
            FailureSignal::RetriedIdenticalCalls,
            Some(TraceFailureMode::LoopingOrRepetition),
        );
    }
    if edit_fail_edit_loop(&records) {
        found.fire(
            FailureSignal::EditFailEditLoop,
            Some(TraceFailureMode::LoopingOrRepetition),
        );
    }
    if let Some(claimed_done) = last_verification_failed(events, &calls, test_reports) {
        found.fire(
            FailureSignal::LastVerificationFailed,
            Some(if claimed_done {
                TraceFailureMode::MissingVerification
            } else {
                TraceFailureMode::UnrecoverableToolFailure
            }),
        );
    }
    if interrupted_and_rewritten(events) {
        found.fire(
            FailureSignal::InterruptedAndRewritten,
            Some(TraceFailureMode::UserIntentMisread),
        );
    }
    if reverted(events, &calls) {
        found.fire(FailureSignal::Reverted, None);
    }
    if taken_over(events, &calls) {
        found.fire(FailureSignal::TakenOver, None);
    }
    if reread_after_long_context(events, &calls) {
        found.fire(
            FailureSignal::RereadAfterLongContext,
            Some(TraceFailureMode::ContextLoss),
        );
    }
    found
}

/// One tool call the model made, with what the detectors need beside the
/// Patterns row. Local to one detector run.
struct ModelCall<'a> {
    /// Index of the call event in `events`.
    at: usize,
    record: ToolCallRecord,
    command: Option<String>,
    /// Files an edit call changed, as the call named them.
    edited: Vec<String>,
    result: Option<&'a SessionEvent>,
}

impl ModelCall<'_> {
    fn failed(&self) -> bool {
        self.record.success == Some(false)
    }
}

/// The tool kind for a call, across adapters. Claude Code's names come from
/// the Patterns allow-list; the others map onto the same kinds.
fn tool_kind(name: &str) -> ToolKind {
    match ToolKind::from_tool_name(name) {
        ToolKind::Other => match name {
            // Codex, Gemini CLI, Cline, OpenCode.
            "shell" | "exec_command" | "local_shell" | "shell_command" | "container.exec"
            | "run_shell_command" | "execute_command" | "bash" => ToolKind::Bash,
            "read_file" | "read" | "view_file" => ToolKind::Read,
            "write_file" | "write_to_file" | "write" => ToolKind::Write,
            "replace" | "replace_in_file" | "edit" | "edit_file" | "apply_patch" => ToolKind::Edit,
            "grep" | "search_file_content" | "search_files" => ToolKind::Grep,
            "glob" | "list_files" => ToolKind::Glob,
            _ => ToolKind::Other,
        },
        kind => kind,
    }
}

/// The shell command a Bash-kind call ran, from the argument shapes the
/// adapters record: `command` as a string (Claude Code, Gemini CLI, Cline,
/// OpenCode), `command` as an argv (Codex `shell`, where `bash -lc <script>`
/// is unwrapped), or `cmd` (Codex `exec_command`).
fn command_of(input: &Value) -> Option<String> {
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(command) => Some(command.clone()),
        Value::Array(argv) => {
            let argv: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
            match argv.as_slice() {
                [shell, flag, script]
                    if matches!(*shell, "bash" | "sh" | "zsh") && matches!(*flag, "-c" | "-lc") =>
                {
                    Some((*script).to_string())
                }
                _ if !argv.is_empty() => Some(argv.join(" ")),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The files an edit call names: a path argument, or the file headers of a
/// Codex `apply_patch` body.
fn edited_paths(input: &Value) -> Vec<String> {
    for key in [
        "file_path",
        "notebook_path",
        "path",
        "filePath",
        "absolute_path",
    ] {
        if let Some(path) = input.get(key).and_then(Value::as_str) {
            if !path.is_empty() {
                return vec![path.to_string()];
            }
        }
    }
    let patch = match input {
        Value::String(patch) => Some(patch.as_str()),
        other => other.get("input").and_then(Value::as_str),
    };
    patch
        .map(|patch| {
            patch
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("*** Update File: ")
                        .or_else(|| line.strip_prefix("*** Add File: "))
                        .or_else(|| line.strip_prefix("*** Delete File: "))
                })
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn throwaway_key() -> DigestKey {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    DigestKey::from_bytes(bytes)
}

/// The model's tool calls in order, each paired with its result by the
/// harness's own call id. A call with no id, or no result, is unpaired, and
/// an unpaired call never counts as failed or as a retry.
fn model_calls(events: &[SessionEvent]) -> Vec<ModelCall<'_>> {
    let key = throwaway_key();
    let mut results: HashMap<&str, &SessionEvent> = HashMap::new();
    for event in events {
        if event.kind == SessionEventKind::ToolResult {
            if let Some(id) = event.tool_call_id.as_deref() {
                results.entry(id).or_insert(event);
            }
        }
    }
    let mut turn: Option<u32> = None;
    let mut calls = Vec::new();
    for (at, event) in events.iter().enumerate() {
        if event.token_counts.is_some() {
            turn = Some(turn.map_or(0, |t| t + 1));
        }
        if event.kind != SessionEventKind::ToolCall {
            continue;
        }
        let tool = tool_kind(event.tool_name.as_deref().unwrap_or(""));
        let result = event
            .tool_call_id
            .as_deref()
            .and_then(|id| results.get(id).copied());
        let success = result.and_then(|r| r.success.or(r.exit_code.map(|code| code == 0)));
        let edited = if is_cycle_edit(tool) || tool == ToolKind::NotebookEdit {
            edited_paths(&event.structured)
        } else {
            Vec::new()
        };
        let read_path = (tool == ToolKind::Read)
            .then(|| edited_paths(&event.structured).into_iter().next())
            .flatten();
        let path = edited.first().cloned().or(read_path);
        let Ok(args) = args_key(&key, tool, &event.structured) else {
            continue;
        };
        calls.push(ModelCall {
            at,
            record: ToolCallRecord {
                turn_ordinal: turn.unwrap_or(0),
                tool,
                args_key: args,
                path_key: path.as_deref().map(|p| path_key(&key, p)),
                path_ext: None,
                result_bytes: None,
                success,
                paired: result.is_some(),
            },
            command: (tool == ToolKind::Bash)
                .then(|| command_of(&event.structured))
                .flatten(),
            edited,
            result,
        });
    }
    calls
}

/// A chain of [`RETRY_LOOP_CALLS`] identical calls: a retry whose earlier
/// call was itself a retry, and so on.
fn retried_identical_calls(calls: &[ToolCallRecord]) -> bool {
    let earlier: BTreeMap<usize, usize> = retry_links(calls).into_iter().collect();
    earlier.keys().any(|&index| {
        let mut length = 1;
        let mut at = index;
        while let Some(&previous) = earlier.get(&at) {
            length += 1;
            at = previous;
        }
        length >= RETRY_LOOP_CALLS
    })
}

/// [`EDIT_LOOP_ROUNDS`] edit-fail-edit rounds on one file.
fn edit_fail_edit_loop(calls: &[ToolCallRecord]) -> bool {
    let mut per_file: BTreeMap<_, usize> = BTreeMap::new();
    for (start, _) in edit_fail_edit_cycles(calls) {
        if let Some(file) = calls[start].path_key {
            *per_file.entry(file).or_default() += 1;
        }
    }
    per_file.values().any(|&rounds| rounds >= EDIT_LOOP_ROUNDS)
}

/// One `&&`, `||`, `;`, `|` or newline separated piece of a command, as
/// whitespace tokens with leading `FOO=bar`, `sudo`, `env` and `time`
/// dropped. No shell quoting is interpreted.
fn command_segments(command: &str) -> Vec<Vec<&str>> {
    command
        .split(['\n', ';', '|', '&'])
        .map(|segment| {
            let mut tokens: Vec<&str> = segment.split_whitespace().collect();
            while let Some(first) = tokens.first() {
                let assignment = first
                    .split_once('=')
                    .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'));
                if assignment || matches!(*first, "sudo" | "env" | "time" | "command") {
                    tokens.remove(0);
                } else {
                    break;
                }
            }
            tokens
        })
        .filter(|tokens| !tokens.is_empty())
        .collect()
}

/// Whether a command runs tests or a build: the fixed list of runners the
/// detector recognises. Anything else is not a verification.
fn is_verification_command(command: &str) -> bool {
    command_segments(command).iter().any(|tokens| {
        let program = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);
        let rest: Vec<&str> = tokens[1..]
            .iter()
            .copied()
            .filter(|t| !t.starts_with('+'))
            .collect();
        let first = rest.first().copied().unwrap_or("");
        let second = rest.get(1).copied().unwrap_or("");
        match program {
            "cargo" => matches!(first, "test" | "build" | "check" | "clippy" | "nextest"),
            "npm" | "pnpm" | "yarn" | "bun" => {
                matches!(first, "test" | "t" | "build")
                    || (first == "run"
                        && matches!(second, "test" | "build" | "check" | "typecheck"))
            }
            "npx" | "pnpx" | "bunx" => {
                matches!(first, "jest" | "vitest" | "tsc" | "mocha" | "playwright")
            }
            "jest" | "vitest" | "mocha" | "pytest" | "tsc" | "ctest" | "rspec" | "phpunit"
            | "xcodebuild" | "gradle" | "gradlew" | "mvn" | "mvnw" => true,
            "python" | "python3" => first == "-m" && matches!(second, "pytest" | "unittest"),
            "go" => matches!(first, "test" | "build" | "vet"),
            "make" => matches!(first, "" | "test" | "check" | "build" | "all" | "ci"),
            "swift" => matches!(first, "test" | "build"),
            "dotnet" => matches!(first, "test" | "build"),
            "deno" | "mix" => first == "test",
            "bundle" => first == "exec" && second == "rspec",
            _ => false,
        }
    })
}

/// Words a final assistant message uses to say the work is finished. A
/// narrow, fixed list: the detector reads a claim, not a judgement.
const COMPLETION_CLAIMS: &[&str] = &[
    "done",
    "fixed",
    "complete",
    "completed",
    "finished",
    "resolved",
    "implemented",
    "successfully",
    "tests pass",
    "tests are passing",
    "all tests pass",
    "now passes",
    "now works",
    "should work now",
    "should now work",
];

fn claims_done(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let joined = format!(" {} ", words.join(" "));
    COMPLETION_CLAIMS
        .iter()
        .any(|claim| joined.contains(&format!(" {claim} ")))
}

/// `Some(claimed_done)` when the session's last verification failed:
/// whichever is later of the last test or build command the model ran and
/// the latest imported test report. `claimed_done` is whether the
/// session's final assistant message, after that verification, claims the
/// work is finished.
fn last_verification_failed(
    events: &[SessionEvent],
    calls: &[ModelCall<'_>],
    test_reports: &[TestReportEvidence],
) -> Option<bool> {
    let in_session = calls.iter().rev().find(|call| {
        call.command.as_deref().is_some_and(is_verification_command) && call.result.is_some()
    });
    let report = test_reports.iter().max_by_key(|report| report.observed_at);
    let in_session_at: Option<DateTime<Utc>> = in_session.and_then(|call| {
        call.result
            .and_then(|r| r.timestamp)
            .or(events[call.at].timestamp)
    });
    // A report counts as the last verification when the session ran none,
    // or when both are dated and the report is later.
    let report_is_last = match (report, in_session) {
        (Some(_), None) => true,
        (Some(report), Some(_)) => in_session_at.is_some_and(|at| report.observed_at > at),
        (None, _) => false,
    };
    let (failed, after) = if report_is_last {
        (report?.failed > 0, in_session.map_or(0, |call| call.at))
    } else {
        let call = in_session?;
        (call.failed(), call.at)
    };
    if !failed {
        return None;
    }
    let final_message = events[after..]
        .iter()
        .rev()
        .find(|e| e.kind == SessionEventKind::Assistant)
        .and_then(|e| e.content.as_deref());
    Some(final_message.is_some_and(claims_done))
}

const INTERRUPT_MARKER: &str = "[Request interrupted by user";

/// A user turn the person typed: not an interrupt marker, and not one of the
/// harness's own `<...>` wrappers (a local command, a reminder).
fn is_typed_instruction(event: &SessionEvent) -> bool {
    event.kind == SessionEventKind::User
        && event.content.as_deref().is_some_and(|text| {
            let text = text.trim();
            !text.is_empty() && !text.starts_with(INTERRUPT_MARKER) && !text.starts_with('<')
        })
}

/// Claude Code's interrupt marker, then a new instruction from the person.
fn interrupted_and_rewritten(events: &[SessionEvent]) -> bool {
    let Some(first_interrupt) = events.iter().position(|e| {
        e.kind == SessionEventKind::User
            && e.content
                .as_deref()
                .is_some_and(|t| t.trim().starts_with(INTERRUPT_MARKER))
    }) else {
        return false;
    };
    events[first_interrupt + 1..]
        .iter()
        .any(is_typed_instruction)
}

fn normalize(path: &str) -> String {
    let path = path.trim().trim_matches(['"', '\'']).replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    path.trim_end_matches('/').to_string()
}

/// Whether a git path argument covers a file the model edited. Model edits
/// are usually absolute and git arguments relative to the repository, so a
/// relative argument matches as a path suffix or a directory prefix of the
/// edited file. `.` and `:/` cover everything.
fn covers(argument: &str, edited: &str) -> bool {
    let argument = normalize(argument);
    let edited = normalize(edited);
    if matches!(argument.as_str(), "" | "." | "*" | ":/") {
        return true;
    }
    edited == argument
        || edited.ends_with(&format!("/{argument}"))
        || edited.starts_with(&format!("{argument}/"))
        || edited.contains(&format!("/{argument}/"))
}

enum RevertTarget {
    Everything,
    Paths(Vec<String>),
}

/// The working-tree files a git command throws away, if it is a revert:
/// `git checkout [<tree>] [--] <paths>`, `git restore <paths>` (not
/// `--staged` alone), or `git reset --hard`.
fn git_revert(tokens: &[&str]) -> Option<RevertTarget> {
    if tokens.first().map(|t| t.rsplit('/').next().unwrap_or(t)) != Some("git") {
        return None;
    }
    let mut rest = &tokens[1..];
    while let Some(first) = rest.first() {
        match *first {
            "-C" | "-c" | "--git-dir" | "--work-tree" => rest = rest.get(2..)?,
            option if option.starts_with('-') => rest = &rest[1..],
            _ => break,
        }
    }
    let (command, args) = rest.split_first()?;
    let after_dashes = |args: &[&str]| -> Option<Vec<String>> {
        let at = args.iter().position(|a| *a == "--")?;
        Some(args[at + 1..].iter().map(|a| a.to_string()).collect())
    };
    let operands = |args: &[&str], valued: &[&str]| -> Vec<String> {
        let mut out = Vec::new();
        let mut skip = false;
        for arg in args {
            if skip {
                skip = false;
            } else if valued.contains(arg) {
                skip = true;
            } else if !arg.starts_with('-') {
                out.push(arg.to_string());
            }
        }
        out
    };
    match *command {
        "checkout" => {
            if args.iter().any(|a| matches!(*a, "-b" | "-B" | "--orphan")) {
                return None;
            }
            Some(RevertTarget::Paths(
                after_dashes(args).unwrap_or_else(|| operands(args, &[])),
            ))
        }
        "restore" => {
            let staged_only = args.iter().any(|a| matches!(*a, "--staged" | "-S"))
                && !args.iter().any(|a| matches!(*a, "--worktree" | "-W"));
            if staged_only {
                return None;
            }
            Some(RevertTarget::Paths(
                after_dashes(args).unwrap_or_else(|| operands(args, &["--source", "-s"])),
            ))
        }
        "reset" if args.contains(&"--hard") => Some(RevertTarget::Everything),
        _ => None,
    }
}

/// A command the person ran themselves, through Claude Code's `!` shell.
fn person_command(event: &SessionEvent) -> Option<&str> {
    if event.kind != SessionEventKind::User {
        return None;
    }
    let text = event.content.as_deref()?;
    let start = text.find("<bash-input>")? + "<bash-input>".len();
    let end = text[start..].find("</bash-input>")? + start;
    Some(&text[start..end])
}

/// A revert, by the model or the person, of files the model had edited
/// earlier in the session.
fn reverted(events: &[SessionEvent], calls: &[ModelCall<'_>]) -> bool {
    let by_event: HashMap<usize, &ModelCall<'_>> = calls.iter().map(|c| (c.at, c)).collect();
    let mut edited: Vec<String> = Vec::new();
    for (at, event) in events.iter().enumerate() {
        let command = match by_event.get(&at) {
            Some(call) => {
                edited.extend(call.edited.iter().cloned());
                call.command.as_deref()
            }
            None => person_command(event),
        };
        let Some(command) = command else {
            continue;
        };
        for tokens in command_segments(command) {
            match git_revert(&tokens) {
                Some(RevertTarget::Everything) if !edited.is_empty() => return true,
                Some(RevertTarget::Paths(paths))
                    if paths
                        .iter()
                        .any(|arg| edited.iter().any(|file| covers(arg, file))) =>
                {
                    return true;
                }
                _ => {}
            }
        }
    }
    false
}

fn same_file(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    !a.is_empty()
        && !b.is_empty()
        && (a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}")))
}

/// The last change to some file the model edited was made outside it.
fn taken_over(events: &[SessionEvent], calls: &[ModelCall<'_>]) -> bool {
    let by_event: HashMap<usize, &ModelCall<'_>> = calls.iter().map(|c| (c.at, c)).collect();
    // Per file the model edited: whether the latest change was outside it.
    let mut files: Vec<(String, bool)> = Vec::new();
    for (at, event) in events.iter().enumerate() {
        if let Some(call) = by_event.get(&at) {
            for path in &call.edited {
                match files.iter_mut().find(|(file, _)| same_file(file, path)) {
                    Some(entry) => entry.1 = false,
                    None => files.push((path.clone(), false)),
                }
            }
        }
        if let Some(path) = event.edited_outside_model.as_deref() {
            for entry in files.iter_mut().filter(|(file, _)| same_file(file, path)) {
                entry.1 = true;
            }
        }
    }
    files.iter().any(|(_, outside)| *outside)
}

/// Context crossed the long-context threshold, and at or after that turn
/// the model read again a file it had already read. Needs the stated usage
/// a turn's context is made of; an adapter that does not record it never
/// fires this.
fn reread_after_long_context(events: &[SessionEvent], calls: &[ModelCall<'_>]) -> bool {
    let mut turns = Vec::new();
    for event in events {
        let Some((input, output)) = event.token_counts else {
            continue;
        };
        let served = event.served_by.as_ref();
        turns.push(TurnRecord {
            ordinal: u32::try_from(turns.len()).unwrap_or(u32::MAX),
            at: event.timestamp,
            uncached_input: served.map(|_| input),
            cache_read: served.map(|s| s.cache_read_tokens),
            cache_write: served.map(|s| CacheWrite::Split {
                m5: s.cache_write_5m_tokens,
                h1: s.cache_write_1h_tokens,
            }),
            output: served.map(|_| output),
            model_label_ix: None,
            gap_secs: None,
            msg_key: None,
        });
    }
    let Some(crossed) = usage_markers(&turns)
        .into_iter()
        .find(|marker| marker.kind == MarkerKind::CrossedLongContext)
        .map(|marker| marker.turn_ordinal)
    else {
        return false;
    };
    let series = UsageSeries {
        turns,
        tool_calls: calls.iter().map(|call| call.record.clone()).collect(),
        truncated: false,
    };
    session_patterns(&series)
        .reread_events
        .iter()
        .any(|reread| reread.turn_ordinal >= crossed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::outcomes::TestEvidenceProvenance;
    use crate::source::ServedBy;
    use serde_json::json;

    fn call(id: &str, tool: &str, input: Value) -> SessionEvent {
        SessionEvent {
            kind: SessionEventKind::ToolCall,
            tool_name: Some(tool.into()),
            tool_call_id: Some(id.into()),
            structured: input,
            model: Some("claude-opus-5".into()),
            ..Default::default()
        }
    }

    fn result(id: &str, success: bool) -> SessionEvent {
        SessionEvent::tool_result(None, Some("out".into()), Some(id.into()), Some(success))
    }

    fn bash(id: &str, command: &str, success: bool) -> Vec<SessionEvent> {
        vec![
            call(id, "Bash", json!({ "command": command })),
            result(id, success),
        ]
    }

    fn edit(id: &str, path: &str) -> Vec<SessionEvent> {
        vec![
            call(
                id,
                "Edit",
                json!({"file_path": path, "old_string": id, "new_string": "b"}),
            ),
            result(id, true),
        ]
    }

    fn read(id: &str, path: &str) -> Vec<SessionEvent> {
        vec![
            call(id, "Read", json!({ "file_path": path })),
            result(id, true),
        ]
    }

    fn user(text: &str) -> SessionEvent {
        SessionEvent::user(text, None)
    }

    fn assistant(text: &str) -> SessionEvent {
        SessionEvent {
            kind: SessionEventKind::Assistant,
            content: Some(text.into()),
            model: Some("claude-opus-5".into()),
            ..Default::default()
        }
    }

    fn session(parts: Vec<Vec<SessionEvent>>) -> Vec<SessionEvent> {
        parts.into_iter().flatten().collect()
    }

    fn detect(events: &[SessionEvent]) -> FailureCandidate {
        detect_failure_signals(events, &[])
    }

    fn only(signal: FailureSignal, mode: Option<TraceFailureMode>) -> FailureCandidate {
        let mut expected = FailureCandidate::default();
        expected.fire(signal, mode);
        expected
    }

    // --- retried identical calls ---

    #[test]
    fn three_identical_calls_in_a_row_are_a_retry_loop() {
        let events = session(vec![
            bash("1", "cargo run", true),
            bash("2", "cargo run", true),
            bash("3", "cargo run", true),
        ]);
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::RetriedIdenticalCalls,
                Some(TraceFailureMode::LoopingOrRepetition)
            )
        );
    }

    #[test]
    fn two_identical_calls_or_three_broken_by_work_are_not() {
        let twice = session(vec![
            bash("1", "cargo run", true),
            bash("2", "cargo run", true),
        ]);
        assert!(detect(&twice).is_empty());
        let broken = session(vec![
            bash("1", "cargo run", true),
            bash("2", "cargo run", true),
            edit("e", "/w/src/lib.rs"),
            bash("3", "cargo run", true),
        ]);
        assert!(detect(&broken).is_empty());
        let unanswered = vec![
            call("1", "Bash", json!({"command": "cargo run"})),
            call("2", "Bash", json!({"command": "cargo run"})),
            call("3", "Bash", json!({"command": "cargo run"})),
        ];
        assert!(detect(&unanswered).is_empty());
    }

    // --- edit, fail, edit ---

    fn edit_fail_rounds(file_of_round: &[&str]) -> Vec<SessionEvent> {
        let mut parts = Vec::new();
        for (round, file) in file_of_round.iter().enumerate() {
            parts.push(edit(&format!("e{round}"), file));
            parts.push(bash(
                &format!("b{round}"),
                &format!("./check {round}"),
                false,
            ));
        }
        parts.push(edit("last", file_of_round.last().unwrap()));
        session(parts)
    }

    #[test]
    fn three_edit_fail_edit_rounds_on_one_file_are_an_edit_loop() {
        let events = edit_fail_rounds(&["/w/a.rs", "/w/a.rs", "/w/a.rs"]);
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::EditFailEditLoop,
                Some(TraceFailureMode::LoopingOrRepetition)
            )
        );
    }

    #[test]
    fn two_rounds_or_rounds_spread_over_files_are_not() {
        assert!(detect(&edit_fail_rounds(&["/w/a.rs", "/w/a.rs"])).is_empty());
        let spread = session(vec![
            edit("1", "/w/a.rs"),
            bash("b1", "./check 1", false),
            edit("2", "/w/a.rs"),
            edit("3", "/w/b.rs"),
            bash("b2", "./check 2", false),
            edit("4", "/w/b.rs"),
            edit("5", "/w/c.rs"),
            bash("b3", "./check 3", false),
            edit("6", "/w/c.rs"),
        ]);
        assert!(detect(&spread).is_empty());
    }

    /// Codex records a shell exit code and no verdict for most calls; the
    /// code is what makes the command a failure here.
    #[test]
    fn an_exit_code_alone_marks_the_failed_command() {
        let mut parts = Vec::new();
        for round in 0..3 {
            parts.push(vec![
                call(
                    &format!("p{round}"),
                    "apply_patch",
                    json!(format!(
                        "*** Begin Patch\n*** Update File: src/a.rs\n@@ {round}"
                    )),
                ),
                result(&format!("p{round}"), true),
            ]);
            parts.push(vec![
                call(
                    &format!("s{round}"),
                    "shell",
                    json!({"command": ["bash", "-lc", format!("./run {round}")]}),
                ),
                SessionEvent {
                    exit_code: Some(1),
                    ..SessionEvent::tool_result(None, None, Some(format!("s{round}")), None)
                },
            ]);
        }
        parts.push(vec![
            call("plast", "apply_patch", json!("*** Update File: src/a.rs\n")),
            result("plast", true),
        ]);
        assert!(
            detect(&session(parts))
                .signals
                .contains(&FailureSignal::EditFailEditLoop)
        );
    }

    // --- last verification failed ---

    #[test]
    fn a_failed_last_test_then_a_claim_of_done_is_missing_verification() {
        let events = session(vec![
            edit("e", "/w/src/lib.rs"),
            bash("t", "cd /w && cargo test -p core", false),
            vec![assistant("All done -- the fix is complete.")],
        ]);
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::LastVerificationFailed,
                Some(TraceFailureMode::MissingVerification)
            )
        );
    }

    #[test]
    fn a_failed_last_build_with_no_claim_is_an_unrecoverable_failure() {
        let events = session(vec![
            bash("t", "npm run build", false),
            vec![assistant(
                "I could not get the build to pass; it needs a newer SDK.",
            )],
        ]);
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::LastVerificationFailed,
                Some(TraceFailureMode::UnrecoverableToolFailure)
            )
        );
    }

    #[test]
    fn a_later_passing_test_or_a_failed_non_test_command_is_not() {
        let recovered = session(vec![
            bash("1", "cargo test", false),
            edit("e", "/w/a.rs"),
            bash("2", "cargo test", true),
            vec![assistant("Done.")],
        ]);
        assert!(detect(&recovered).is_empty());
        let not_a_test = session(vec![
            bash("1", "cargo test", true),
            bash("2", "ls /nonexistent", false),
            vec![assistant("Done.")],
        ]);
        assert!(detect(&not_a_test).is_empty());
        // "abandoned" is not "done".
        assert!(!claims_done("I abandoned the approach"));
        assert!(claims_done("Fixed. All tests pass now."));
    }

    fn report(failed: u64, observed_at: DateTime<Utc>) -> TestReportEvidence {
        TestReportEvidence {
            schema_version: 1,
            runner: "cargo test".into(),
            passed: 10,
            failed,
            skipped: 0,
            observed_at,
            commit_id: None,
            artifact_digest: "0".repeat(64),
            imported_at: observed_at,
            provenance: TestEvidenceProvenance::ImportedReport,
        }
    }

    #[test]
    fn an_imported_failing_report_is_the_last_verification_when_later() {
        let now = Utc::now();
        let mut events = session(vec![
            bash("t", "cargo test", true),
            vec![assistant("Finished.")],
        ]);
        events[1].timestamp = Some(now - chrono::Duration::minutes(5));
        let found = detect_failure_signals(&events, &[report(2, now)]);
        assert_eq!(
            found,
            only(
                FailureSignal::LastVerificationFailed,
                Some(TraceFailureMode::MissingVerification)
            )
        );
        // An earlier failing report is overtaken by the passing run.
        let earlier =
            detect_failure_signals(&events, &[report(2, now - chrono::Duration::hours(1))]);
        assert!(earlier.is_empty());
        // A passing report is not a failure.
        assert!(detect_failure_signals(&events, &[report(0, now)]).is_empty());
        // With no command in the session, the report decides alone.
        assert!(
            detect_failure_signals(&[assistant("hm")], &[report(1, now)])
                .signals
                .contains(&FailureSignal::LastVerificationFailed)
        );
    }

    // --- interrupt, then a rewritten instruction ---

    #[test]
    fn an_interrupt_followed_by_a_new_instruction_is_a_misread() {
        let events = vec![
            user("add caching to the client"),
            assistant("I'll start by rewriting the server."),
            user("[Request interrupted by user for tool use]"),
            user("no -- only the client, leave the server alone"),
        ];
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::InterruptedAndRewritten,
                Some(TraceFailureMode::UserIntentMisread)
            )
        );
    }

    #[test]
    fn an_interrupt_with_no_new_instruction_or_no_interrupt_is_not() {
        let stopped = vec![
            user("add caching"),
            assistant("working"),
            user("[Request interrupted by user]"),
            user("<local-command-stdout>bye</local-command-stdout>"),
        ];
        assert!(detect(&stopped).is_empty());
        let plain = vec![
            user("add caching"),
            assistant("ok"),
            user("thanks, now docs"),
        ];
        assert!(detect(&plain).is_empty());
    }

    // --- revert ---

    #[test]
    fn a_checkout_restore_or_hard_reset_of_model_edits_is_a_revert() {
        for command in [
            "git checkout -- src/lib.rs",
            "git checkout HEAD src/lib.rs",
            "git restore src/lib.rs",
            "git -C /w restore .",
            "cd /w && git reset --hard",
        ] {
            let events = session(vec![edit("e", "/w/src/lib.rs"), bash("g", command, true)]);
            assert_eq!(
                detect(&events),
                only(FailureSignal::Reverted, None),
                "{command}"
            );
        }
        // The person's own `!` shell counts too.
        let by_person = session(vec![
            edit("e", "/w/src/lib.rs"),
            vec![user("<bash-input>git restore src/lib.rs</bash-input>")],
        ]);
        assert!(detect(&by_person).reverted());
    }

    #[test]
    fn a_branch_switch_an_unstage_or_an_unrelated_file_is_not() {
        for command in [
            "git checkout main",
            "git checkout -b fix",
            "git restore --staged src/lib.rs",
            "git restore src/other.rs",
            "git reset HEAD~1",
            "git status",
        ] {
            let events = session(vec![edit("e", "/w/src/lib.rs"), bash("g", command, true)]);
            assert!(!detect(&events).reverted(), "{command}");
        }
        // A revert before the model edited anything reverts nothing of its.
        let first = session(vec![
            bash("g", "git reset --hard", true),
            edit("e", "/w/src/lib.rs"),
        ]);
        assert!(!detect(&first).reverted());
    }

    // --- takeover ---

    fn edited_outside(path: &str) -> SessionEvent {
        SessionEvent {
            edited_outside_model: Some(path.into()),
            ..SessionEvent::opaque("attachment", None)
        }
    }

    #[test]
    fn the_person_making_the_last_edit_to_a_model_file_is_a_takeover() {
        let events = session(vec![
            edit("e", "/w/src/lib.rs"),
            vec![edited_outside("/w/src/lib.rs")],
        ]);
        assert_eq!(detect(&events), only(FailureSignal::TakenOver, None));
        assert!(detect(&events).taken_over());
    }

    #[test]
    fn an_outside_edit_the_model_followed_or_on_another_file_is_not() {
        let model_last = session(vec![
            edit("e1", "/w/src/lib.rs"),
            vec![edited_outside("/w/src/lib.rs")],
            edit("e2", "/w/src/lib.rs"),
        ]);
        assert!(detect(&model_last).is_empty());
        let other_file = session(vec![
            edit("e", "/w/src/lib.rs"),
            vec![edited_outside("/w/README.md")],
        ]);
        assert!(detect(&other_file).is_empty());
    }

    // --- re-read after long context ---

    fn turn(context: u32) -> SessionEvent {
        SessionEvent {
            kind: SessionEventKind::Assistant,
            content: Some("…".into()),
            token_counts: Some((context, 10)),
            served_by: Some(ServedBy {
                model: "claude-opus-5".into(),
                cache_read_tokens: 0,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_reread_after_crossing_long_context_is_context_loss() {
        let events = session(vec![
            vec![turn(150_000)],
            read("r1", "/w/src/lib.rs"),
            vec![turn(210_000)],
            read("r2", "/w/src/lib.rs"),
        ]);
        assert_eq!(
            detect(&events),
            only(
                FailureSignal::RereadAfterLongContext,
                Some(TraceFailureMode::ContextLoss)
            )
        );
    }

    #[test]
    fn a_reread_below_long_context_or_a_crossing_without_one_is_not() {
        let short = session(vec![
            vec![turn(50_000)],
            read("r1", "/w/src/lib.rs"),
            vec![turn(60_000)],
            read("r2", "/w/src/lib.rs"),
        ]);
        assert!(detect(&short).is_empty());
        let no_reread = session(vec![
            vec![turn(150_000)],
            read("r1", "/w/src/lib.rs"),
            vec![turn(210_000)],
            read("r2", "/w/src/main.rs"),
        ]);
        assert!(detect(&no_reread).is_empty());
        // A re-read before the crossing is ordinary, not context loss.
        let before = session(vec![
            vec![turn(50_000)],
            read("r1", "/w/src/lib.rs"),
            vec![turn(60_000)],
            read("r2", "/w/src/lib.rs"),
            vec![turn(210_000)],
            read("r3", "/w/src/main.rs"),
        ]);
        assert!(detect(&before).is_empty());
        // Without stated usage there is no context to cross.
        let mut unpriced = session(vec![
            vec![turn(150_000)],
            read("r1", "/w/src/lib.rs"),
            vec![turn(210_000)],
            read("r2", "/w/src/lib.rs"),
        ]);
        for event in &mut unpriced {
            event.served_by = None;
        }
        assert!(detect(&unpriced).is_empty());
    }

    // --- the candidate stays on this device ---

    /// A Claude Code session on disk where every detector fires, as the
    /// adapter reads it.
    fn every_signal_session() -> (tempfile::TempDir, crate::source::SessionTranscript) {
        use crate::source::TraceSource;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        let session = "22222222-2222-2222-2222-222222222222";
        let base = json!({"sessionId": session, "cwd": "/Users/testuser/code/myproj",
            "version": "2.0.1", "timestamp": "2026-10-12T10:00:00Z"});
        let mut lines: Vec<Value> = Vec::new();
        let mut n = 0;
        let mut push = |mut record: Value| {
            n += 1;
            for (key, value) in base.as_object().unwrap() {
                record[key] = value.clone();
            }
            record["uuid"] = json!(format!("u{n}"));
            lines.push(record);
        };
        let usage = |input: u32| {
            json!({"input_tokens": input, "output_tokens": 5,
            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0,
            "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0}})
        };
        let tool = |id: &str, name: &str, input: Value, tokens: u32| {
            json!({"type": "assistant", "message": {"id": format!("m{id}"), "model": "claude-opus-5",
                "usage": usage(tokens),
                "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}})
        };
        let answer = |id: &str, is_error: bool, text: &str| {
            json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "is_error": is_error, "content": text}]}})
        };
        let lib = "/Users/testuser/code/myproj/src/lib.rs";
        let parser = "/Users/testuser/code/myproj/src/parser.rs";
        push(
            json!({"type": "user", "message": {"role": "user", "content": "make the tests pass"}}),
        );
        for attempt in 0..3 {
            push(tool(
                &format!("c{attempt}"),
                "Bash",
                json!({"command": "cargo run"}),
                100_000,
            ));
            push(answer(&format!("c{attempt}"), false, "listening"));
        }
        push(tool("r1", "Read", json!({"file_path": parser}), 150_000));
        push(answer("r1", false, "fn parse() {}"));
        for round in 0..3 {
            push(tool(
                &format!("e{round}"),
                "Edit",
                json!({"file_path": lib, "old_string": format!("{round}"), "new_string": "x"}),
                160_000,
            ));
            push(answer(&format!("e{round}"), false, "ok"));
            push(tool(
                &format!("t{round}"),
                "Bash",
                json!({"command": "cargo test"}),
                170_000,
            ));
            push(answer(
                &format!("t{round}"),
                true,
                "Exit code 101\ntest failed",
            ));
        }
        push(tool(
            "e9",
            "Edit",
            json!({"file_path": lib, "old_string": "9", "new_string": "y"}),
            210_000,
        ));
        push(answer("e9", false, "ok"));
        push(tool("r2", "Read", json!({"file_path": parser}), 220_000));
        push(answer("r2", false, "fn parse() {}"));
        push(
            json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "text", "text": "[Request interrupted by user]"}]}}),
        );
        push(
            json!({"type": "user", "message": {"role": "user", "content": "no, only fix the parser"}}),
        );
        push(
            json!({"type": "attachment", "attachment": {"type": "edited_text_file",
            "filename": lib, "snippet": "fn a() { fixed by hand }"}}),
        );
        push(tool(
            "g",
            "Bash",
            json!({"command": "git checkout -- src/lib.rs"}),
            230_000,
        ));
        push(answer("g", false, ""));
        push(tool(
            "t9",
            "Bash",
            json!({"command": "cargo test"}),
            240_000,
        ));
        push(answer("t9", true, "Exit code 101\nstill failing"));
        push(
            json!({"type": "assistant", "message": {"id": "mz", "model": "claude-opus-5",
            "content": [{"type": "text", "text": "Done -- all fixed."}]}}),
        );
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(project.join(format!("{session}.jsonl")), body).unwrap();
        let src = crate::source::claude_code::ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let transcript = src.load(&r).unwrap();
        (dir, transcript)
    }

    #[test]
    fn every_detector_fires_on_the_fixture_session() {
        let (_dir, transcript) = every_signal_session();
        let found = detect(&transcript.events);
        let all: BTreeSet<FailureSignal> = [
            FailureSignal::RetriedIdenticalCalls,
            FailureSignal::EditFailEditLoop,
            FailureSignal::LastVerificationFailed,
            FailureSignal::InterruptedAndRewritten,
            FailureSignal::Reverted,
            FailureSignal::TakenOver,
            FailureSignal::RereadAfterLongContext,
        ]
        .into();
        assert_eq!(found.signals, all);
        for mode in [
            TraceFailureMode::LoopingOrRepetition,
            TraceFailureMode::MissingVerification,
            TraceFailureMode::UserIntentMisread,
            TraceFailureMode::ContextLoss,
        ] {
            assert!(found.failure_modes.contains(&mode), "{mode:?}");
        }
    }

    /// Detector output reaches no envelope. Two halves: a session on which
    /// the detectors fire builds an envelope with no failure mode anywhere
    /// and without the outside-edit path, and no module outside this one
    /// (and its `mod` line) names this module -- in this crate or in the C
    /// ABI crate the shells call -- so no envelope, upload or preview path
    /// can be calling it. Wiring it in (F2) has to change this
    /// test on purpose.
    #[tokio::test]
    async fn no_envelope_path_reads_the_detectors() {
        let (_dir, transcript) = every_signal_session();
        assert!(!detect(&transcript.events).failure_modes.is_empty());

        let cfg = crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            consent_scopes_chosen: Some(true),
            witness_origin: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        };
        let raw = crate::envelope::build_raw_contribution(&transcript, &cfg, Utc::now());
        assert!(raw.outcome.failure_modes.is_empty());
        assert!(raw.events.iter().all(|e| e.failure_modes.is_empty()));
        let redactor =
            crate::envelope::build_redactor_with(&cfg, transcript.cwd.as_deref(), None).unwrap();
        let envelope = crate::envelope::redact_to_envelope(&redactor, raw)
            .await
            .unwrap();
        assert!(envelope.outcome.failure_modes.is_empty());
        assert!(envelope.events.iter().all(|e| e.failure_modes.is_empty()));
        let text = serde_json::to_string(&envelope).unwrap();
        for name in [
            "looping_or_repetition",
            "missing_verification",
            "unrecoverable_tool_failure",
            "user_intent_misread",
            "context_loss",
            "edited_outside_model",
            "fixed by hand",
        ] {
            assert!(!text.contains(name), "{name} reached the envelope");
        }

        // This crate and the C ABI the desktop shells call through: every
        // preview and upload path the shells reach lives in one of the two.
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let mut callers = Vec::new();
        let mut stack = vec![
            crates.join("trace-commons-contributor/src"),
            crates.join("trace-commons-contributor-ffi/src"),
        ];
        assert!(stack.iter().all(|dir| dir.is_dir()), "{stack:?}");
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let rel = path
                        .strip_prefix(&crates)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    // Code only: a doc comment naming this module is not a
                    // caller.
                    let body: String = std::fs::read_to_string(&path)
                        .unwrap()
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let allowed = match rel.as_str() {
                        "trace-commons-contributor/src/insights/failure_signals.rs" => true,
                        // The `pub mod failure_signals;` line, and nothing else.
                        "trace-commons-contributor/src/insights.rs" => {
                            body.matches("failure_signals").count() == 1
                                && body.contains("pub mod failure_signals;")
                        }
                        _ => false,
                    };
                    if !allowed
                        && (body.contains("failure_signals")
                            || body.contains("detect_failure_signals"))
                    {
                        callers.push(rel);
                    }
                }
            }
        }
        assert_eq!(callers, Vec::<String>::new());
    }
}
