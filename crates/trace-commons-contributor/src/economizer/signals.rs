//! One finished tool call, read from a Claude Code hook, and the failure
//! signals the loop guard and the facts read off it.
//!
//! Pure: no I/O. Claude Code reports a call that failed through
//! `PostToolUseFailure` with the text the model saw (`error`), and a call
//! that succeeded through `PostToolUse` with its `tool_response`. A Bash
//! call carries no exit code there, and a piped command (`pytest | tail`)
//! exits 0 whatever the tests did, so a command's output is also read for
//! failure markers.

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
        let output = if failed {
            input
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            response_text(input.get("tool_response").unwrap_or(&Value::Null))
        };
        Some(Self {
            tool,
            kind,
            output: cap(output),
            failed,
        })
    }

    /// Whether the call's outcome was a failure: reported as failed, or a
    /// command whose output carries a failure marker.
    pub fn is_failure(&self) -> bool {
        self.failed
            || matches!(self.kind, CallKind::Command { .. }) && marker_line(&self.output).is_some()
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

/// The first line of `text` that carries a failure marker.
pub fn marker_line(text: &str) -> Option<&str> {
    text.lines()
        .find(|line| MARKERS.iter().any(|marker| line.contains(marker)))
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
