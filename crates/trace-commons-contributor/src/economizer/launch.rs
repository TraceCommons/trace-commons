//! What a managed launch adds when the economizer is on. Nothing is ever
//! written into the person's folder.
//!
//! | Harness | Hooks | What pruning leaves out |
//! | --- | --- | --- |
//! | Claude Code | `hooks` in the launch's `--settings` | `permissions.deny` there |
//! | Codex | `-c hooks.<Event>=...`, trusted by hash ([`crate::managed::codex_hooks`]) | `-c features.<name>=false` |
//! | OpenCode | a plugin in the launch's isolated profile ([`crate::managed::opencode`]) | `tools.<name> = false` in that profile's config |
//!
//! A launch gets none of it when any of these holds:
//!
//! - the person has not turned the economizer on;
//! - the tool is older than the version this was checked against
//!   ([`MIN_CLAUDE`], [`MIN_CODEX`], [`MIN_OPENCODE`]), or a major version
//!   it was not checked on (OpenCode 2 could not be fetched to check), or,
//!   for Codex, the platform is not Unix, where its hook trust was checked;
//! - the launch resumes a conversation: a changed tool list would make it
//!   rewrite its whole prompt cache;
//! - the session's trace is collected: its transcript would carry the
//!   economizer's context and a shorter tool list, and contributed traces
//!   have no way yet to say so.

use serde_json::{Map, Value, json};
use std::path::Path;

use super::Harness;
use super::store::{Store, repo_key};
use super::tools::Thresholds;
use crate::managed::codex_hooks::Hook;
use crate::managed::statusline::{Shell, quote};

/// The oldest Claude Code the economizer runs under: the first this was
/// written against that reports failed calls through `PostToolUseFailure`.
pub const MIN_CLAUDE: (u32, u32, u32) = (2, 1, 300);
/// The oldest Codex: the version its hook trust keys were checked on.
pub const MIN_CODEX: (u32, u32, u32) = (0, 164, 0);
/// The oldest OpenCode: the version its plugin events, and the profile
/// isolation that keeps every other plugin out once `--pure` is dropped,
/// were checked on.
pub const MIN_OPENCODE: (u32, u32, u32) = (1, 19, 5);
/// Seconds a tool waits for one hook run.
pub const HOOK_TIMEOUT_SECS: u64 = 10;
/// Codex 0.164 clamps a `SessionEnd` hook's timeout to 3 seconds, and the
/// hash it trusts is of the clamped declaration, so one declared longer
/// reads as modified and never runs (checked with its `hooks/list`).
pub const CODEX_SESSION_END_TIMEOUT_SECS: u64 = 3;
/// The hook events the economizer listens to on Claude Code.
pub const CLAUDE_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "SessionEnd",
];
/// The hook events on Codex, which reports a failed call through
/// `PostToolUse` like any other.
pub const CODEX_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
    "Stop",
    "SessionEnd",
];

/// What the launcher knows about one launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launch<'a> {
    pub harness: Harness,
    pub version: Option<(u32, u32, u32)>,
    pub resumed: bool,
    pub collecting: bool,
    pub cwd: &'a Path,
}

impl Launch<'_> {
    /// Whether the economizer may run in this launch, the person's choice
    /// aside.
    pub fn eligible(&self) -> bool {
        let version = match self.harness {
            Harness::Claude => self
                .version
                .is_some_and(|v| v.0 == MIN_CLAUDE.0 && v >= MIN_CLAUDE),
            Harness::Codex => {
                cfg!(unix)
                    && self
                        .version
                        .is_some_and(|v| v.0 == MIN_CODEX.0 && v >= MIN_CODEX)
            }
            Harness::Opencode => self
                .version
                .is_some_and(|v| v.0 == MIN_OPENCODE.0 && v >= MIN_OPENCODE),
        };
        version && !self.resumed && !self.collecting
    }
}

/// The hook command: this executable's `economizer-hook`, for the folder's
/// key and the harness. `None` for a path that is not UTF-8.
fn hook_command(store: &Store, exe: &Path, launch: &Launch<'_>, shell: Shell) -> Option<String> {
    Some(format!(
        "{} economizer-hook --state {} --repo {} --harness {}",
        quote(exe.to_str()?, shell),
        quote(store.root().to_str()?, shell),
        quote(&repo_key(launch.cwd, launch.harness), shell),
        launch.harness.as_str(),
    ))
}

/// What pruning leaves out of this launch.
fn left_out(store: &Store, launch: &Launch<'_>) -> Vec<String> {
    store
        .repo(&repo_key(launch.cwd, launch.harness))
        .tools
        .deny_list(launch.harness, &Thresholds::default())
}

fn active(store: &Store, launch: &Launch<'_>, harness: Harness) -> bool {
    launch.harness == harness && store.enabled() && launch.eligible()
}

/// Claude Code: the keys this launch adds to its `--settings`, or none.
pub fn claude_settings(
    store: &Store,
    exe: &Path,
    launch: &Launch<'_>,
) -> Option<Map<String, Value>> {
    if !active(store, launch, Harness::Claude) {
        return None;
    }
    claude_settings_for(store, exe, launch, Shell::current())
}

fn claude_settings_for(
    store: &Store,
    exe: &Path,
    launch: &Launch<'_>,
    shell: Shell,
) -> Option<Map<String, Value>> {
    let command = hook_command(store, exe, launch, shell)?;
    let mut hooks = Map::new();
    for event in CLAUDE_EVENTS {
        hooks.insert(
            (*event).to_owned(),
            json!([{"hooks": [{"type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS}]}]),
        );
    }
    let mut settings = Map::new();
    settings.insert("hooks".to_owned(), Value::Object(hooks));
    let deny = left_out(store, launch);
    if !deny.is_empty() {
        settings.insert("permissions".to_owned(), json!({ "deny": deny }));
    }
    Some(settings)
}

/// What a Codex launch adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPlan {
    pub hooks: Vec<Hook>,
    /// Feature packages to turn off.
    pub features_off: Vec<String>,
}

impl CodexPlan {
    /// The `-c features.<name>=false` arguments.
    pub fn feature_args(&self) -> Vec<String> {
        self.features_off
            .iter()
            .flat_map(|name| ["-c".to_owned(), format!("features.{name}=false")])
            .collect()
    }
}

/// Codex: the hooks and feature switches this launch adds, or none.
pub fn codex(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<CodexPlan> {
    if !active(store, launch, Harness::Codex) {
        return None;
    }
    codex_for(store, exe, launch)
}

fn codex_for(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<CodexPlan> {
    // Codex runs a hook command through `sh -c` on Unix, the only platform
    // it is enabled on.
    let command = hook_command(store, exe, launch, Shell::Posix)?;
    Some(CodexPlan {
        hooks: CODEX_EVENTS
            .iter()
            .map(|event| Hook {
                event,
                command: command.clone(),
                timeout: if *event == "SessionEnd" {
                    CODEX_SESSION_END_TIMEOUT_SECS
                } else {
                    HOOK_TIMEOUT_SECS
                },
            })
            .collect(),
        features_off: left_out(store, launch),
    })
}

/// What an OpenCode launch adds: our plugin, the only one the launch
/// loads, and the tools its configuration turns off
/// ([`crate::managed::opencode`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodePlan {
    /// The hook program and its arguments, run once per event with the
    /// event's JSON on stdin. No shell is involved.
    pub argv: Vec<String>,
    /// Built-in tools to turn off.
    pub tools_off: Vec<String>,
}

/// The plugin's source, before its settings are filled in.
const OPENCODE_PLUGIN: &str = include_str!("opencode_plugin.js");
const OPENCODE_SETTINGS: &str = "__TRACE_COMMONS_SETTINGS__";

impl OpencodePlan {
    /// The plugin for the managed session `session`: its settings are a
    /// JSON literal, which is also a JavaScript one, so no value can
    /// become code.
    pub fn plugin(&self, session: &str) -> String {
        let settings = json!({ "argv": self.argv, "session": session });
        OPENCODE_PLUGIN.replacen(OPENCODE_SETTINGS, &settings.to_string(), 1)
    }

    /// `tools.<name> = false` for each tool left out.
    pub fn tools(&self) -> Map<String, Value> {
        self.tools_off
            .iter()
            .map(|name| (name.clone(), Value::Bool(false)))
            .collect()
    }
}

/// OpenCode: the plugin's hook program and the tools to turn off, or none.
pub fn opencode(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<OpencodePlan> {
    if !active(store, launch, Harness::Opencode) {
        return None;
    }
    opencode_for(store, exe, launch)
}

fn opencode_for(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<OpencodePlan> {
    Some(OpencodePlan {
        argv: vec![
            exe.to_str()?.to_owned(),
            "economizer-hook".to_owned(),
            "--state".to_owned(),
            store.root().to_str()?.to_owned(),
            "--repo".to_owned(),
            repo_key(launch.cwd, Harness::Opencode),
            "--harness".to_owned(),
            Harness::Opencode.as_str().to_owned(),
        ],
        tools_off: left_out(store, launch),
    })
}

#[cfg(test)]
pub(super) mod testing {
    use super::*;

    pub fn claude(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<Map<String, Value>> {
        claude_settings_for(store, exe, launch, Shell::Posix)
    }

    pub fn codex(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<CodexPlan> {
        codex_for(store, exe, launch)
    }

    pub fn opencode(store: &Store, exe: &Path, launch: &Launch<'_>) -> Option<OpencodePlan> {
        opencode_for(store, exe, launch)
    }
}
