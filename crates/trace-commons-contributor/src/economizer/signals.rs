//! One finished tool call, read from a Claude Code hook, and the failure
//! signals the loop guard and the facts read off it.
//!
//! Pure: no I/O. Claude Code reports a call that failed through
//! `PostToolUseFailure` with the text the model saw (`error`), and a call
//! that succeeded through `PostToolUse` with its `tool_response`. Codex
//! reports both through `PostToolUse`, so a command's exit status is read
//! from its response wherever one is given (`exit_code`, or an
//! `Exit code N` line). A non-zero exit is a failure for any command.
//!
//! A piped command (`pytest | tail`) exits 0 whatever the tests did, so a
//! check whose exit is unknown or piped is also read for failure markers,
//! skipping lines that report a pass (`PASSED`, `... ok`). Only a check's:
//! any other command may be printing source (`cat`, `rg Exception`,
//! `git diff`), where `error:` is content.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// How much of a call's output is read. Failures show early.
pub const OUTPUT_CAP: usize = 20_000;

/// What a call did, as far as the economizer cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    /// Files were written: `Edit`, `MultiEdit`, `Write`, `NotebookEdit`,
    /// Codex's `apply_patch`, OpenCode's `edit`, `write` and `patch`.
    Edit { paths: Vec<String> },
    /// A shell command ran: `Bash`.
    Command { command: String },
    /// A file or search was read: `Read`, `Grep`, `Glob`. Its output is file
    /// content, so an `assert` in it is not a failure.
    Content,
    /// Any other tool.
    Other,
}

/// One finished tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub tool: String,
    pub kind: CallKind,
    /// What the model saw, capped at [`OUTPUT_CAP`] bytes.
    pub output: String,
    /// Claude Code reported the call as failed (`PostToolUseFailure`).
    pub failed: bool,
    /// A command's exit status, when the harness gave one.
    pub exit_code: Option<i64>,
}

impl ToolCall {
    /// The call a hook event describes. `failed` is true for
    /// `PostToolUseFailure`, whose `error` is the output.
    pub fn from_hook(input: &Value, failed: bool) -> Option<Self> {
        let tool = input.get("tool_name")?.as_str()?.to_owned();
        let args = input.get("tool_input").unwrap_or(&Value::Null);
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let kind = match tool.as_str() {
            "Edit" | "MultiEdit" | "Write" | "edit" | "write" => CallKind::Edit {
                paths: vec![first(&[text("file_path"), text("filePath")]).to_owned()],
            },
            "NotebookEdit" => CallKind::Edit {
                paths: vec![text("notebook_path").to_owned()],
            },
            "apply_patch" | "patch" => CallKind::Edit {
                paths: patch_paths(&strings(args)),
            },
            "Bash" | "bash" | "shell" | "exec_command" => {
                let command = first(&[text("command"), text("cmd")]).to_owned();
                CallKind::Command { command }
            }
            "Read" | "Grep" | "Glob" | "read" | "grep" | "glob" | "list" => CallKind::Content,
            _ => CallKind::Other,
        };
        let response = input.get("tool_response").unwrap_or(&Value::Null);
        let output = if failed {
            input
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            response_text(response)
        };
        let exit_code = match kind {
            CallKind::Command { .. } => {
                response_exit_code(response).or_else(|| exit_code_line(&output))
            }
            _ => None,
        };
        Some(Self {
            tool,
            kind,
            output: cap(output),
            failed,
            exit_code,
        })
    }

    /// Whether the call's outcome was a failure: reported as failed, a
    /// command that exited non-zero, or a check whose exit is unknown or
    /// piped and whose output carries a failure marker. Another command's
    /// output is not read for markers: it may be file content.
    pub fn is_failure(&self) -> bool {
        if self.failed {
            return true;
        }
        let CallKind::Command { command } = &self.kind else {
            return false;
        };
        match self.exit_code {
            Some(code) if code != 0 => true,
            // A simple command's exit is its own: a check that exited 0
            // passed, whatever its output logs. A pipe, a sequence or a
            // second line may hide the check's exit behind another's.
            Some(_) if is_simple_command(command) => false,
            _ => self.is_check() && marker_line(&self.output).is_some(),
        }
    }

    /// Whether the call ran a check: tests, a build or a type check.
    pub fn is_check(&self) -> bool {
        match &self.kind {
            CallKind::Command { command } => is_check_command(command),
            _ => false,
        }
    }
}

/// The first non-empty of `values`.
fn first<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// Every string in `value`, joined: where a patch tool's input may keep
/// its patch.
fn strings(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(strings).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => map.values().map(strings).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The files a patch in Codex's format writes (`*** Update File: <path>`,
/// `*** Add File:`, `*** Delete File:`), as Codex sends it either to its
/// `apply_patch` tool or inside a shell command.
pub fn patch_paths(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            ["*** Update File:", "*** Add File:", "*** Delete File:"]
                .iter()
                .find_map(|marker| line.strip_prefix(marker))
                .map(|path| path.trim().to_owned())
        })
        .filter(|path| !path.is_empty())
        .collect()
}

/// The text of a successful call's `tool_response`: a Bash call's stdout
/// and stderr, or whatever single string the tool returned.
fn response_text(response: &Value) -> String {
    match response {
        Value::String(text) => text.clone(),
        Value::Object(map) => {
            let mut parts = Vec::new();
            for key in ["stdout", "stderr", "output", "content", "result", "error"] {
                match map.get(key) {
                    Some(Value::String(text)) if !text.is_empty() => parts.push(text.clone()),
                    Some(Value::Array(items)) => parts.extend(
                        items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .map(str::to_owned),
                    ),
                    _ => {}
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

/// Whether `command` is one command, whose exit status is its own: no
/// pipe, sequence, background job or second line.
fn is_simple_command(command: &str) -> bool {
    !command.trim().contains(['|', ';', '&', '\n'])
}

/// The exit status a response object carries: `exit_code`, `exitCode`, or
/// either under `metadata`.
fn response_exit_code(response: &Value) -> Option<i64> {
    let read = |value: &Value| {
        ["exit_code", "exitCode"]
            .iter()
            .find_map(|key| value.get(key).and_then(Value::as_i64))
    };
    read(response).or_else(|| response.get("metadata").and_then(read))
}

/// The exit status an output reports on its own line near the top:
/// `Exit code 1`, `Exit code: 1` or `Process exited with code 1`.
fn exit_code_line(output: &str) -> Option<i64> {
    output.lines().take(8).find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("Exit code")
            .or_else(|| line.strip_prefix("Process exited with code"))?;
        rest.trim_start_matches(':').trim().parse().ok()
    })
}

fn cap(mut text: String) -> String {
    if text.len() > OUTPUT_CAP {
        let mut end = OUTPUT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Substrings that mark a line of output as a failure, across the common
/// test runners, compilers and shells.
const MARKERS: &[&str] = &[
    "Traceback (most recent call last)",
    "Error:",
    "error:",
    "error[",
    "Exception",
    "FAILED",
    "FAIL:",
    "--- FAIL",
    "panicked at",
    "not ok ",
    "npm ERR!",
    "command not found",
    "is not recognized as",
    "No such file or directory",
    "No module named",
];

/// The first line of `text` that carries a failure marker and does not
/// report a pass.
pub fn marker_line(text: &str) -> Option<&str> {
    text.lines()
        .find(|line| !passing_line(line) && MARKERS.iter().any(|marker| line.contains(marker)))
}

/// A line that reports a passing test, whatever its name says:
/// `test_raises_TimeoutException PASSED`, `test error_path ... ok`,
/// `--- PASS: TestError`, Go's `ok  pkg 0.1s`.
fn passing_line(line: &str) -> bool {
    let line = line.trim();
    line.contains(" PASSED")
        || line.ends_with(" ... ok")
        || line.starts_with("--- PASS")
        || line.starts_with("ok ")
}

/// A digest of how a failing output failed: its first failure line, with
/// numbers, addresses and paths removed, so two runs that fail the same way
/// match. `None` when there is no line to go on. Only the digest is kept.
pub fn failure_signature(output: &str) -> Option<String> {
    let line = marker_line(output).or_else(|| {
        output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("Exit code"))
    })?;
    let normalized = line
        .split_whitespace()
        .filter(|word| !word.contains('/') && !word.contains('\\') && !word.starts_with("0x"))
        .map(|word| {
            word.chars()
                .filter(|c| !c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(short_digest("failure", &normalized))
}

/// A short, domain-separated digest, for keys that must not hold the text
/// they stand for.
pub fn short_digest(domain: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"trace-commons-economizer\0");
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher.update(text.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

/// Programs and subcommands that check work: tests, builds, type checks.
const CHECKS: &[&[&str]] = &[
    &["pytest"],
    &["py.test"],
    &["unittest"],
    &["tox"],
    &["nox"],
    &["cargo", "test"],
    &["cargo", "check"],
    &["cargo", "build"],
    &["cargo", "clippy"],
    &["cargo", "nextest"],
    &["go", "test"],
    &["go", "build"],
    &["go", "vet"],
    &["npm", "test"],
    &["npm", "run", "test"],
    &["npm", "run", "build"],
    &["pnpm", "test"],
    &["yarn", "test"],
    &["jest"],
    &["vitest"],
    &["mocha"],
    &["tsc"],
    &["make", "test"],
    &["make", "check"],
    &["rspec"],
    &["mvn", "test"],
    &["gradle", "test"],
    &["swift", "test"],
    &["swift", "build"],
    &["dotnet", "test"],
    &["ctest"],
];

/// Whether `command` runs one of [`CHECKS`] anywhere in it.
pub fn is_check_command(command: &str) -> bool {
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
        .filter(|word| !word.is_empty())
        .map(|word| word.rsplit('/').next().unwrap_or(word))
        .collect();
    CHECKS.iter().any(|check| {
        words
            .windows(check.len())
            .any(|window| window.iter().zip(check.iter()).all(|(a, b)| a == b))
    })
}
