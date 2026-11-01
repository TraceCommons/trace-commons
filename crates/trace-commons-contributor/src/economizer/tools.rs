//! Tool pruning: stop sending the definitions of built-in tools a folder's
//! sessions never use.
//!
//! Every loaded tool's definition goes out with every request. No tool's
//! session log records which tools it loaded, so pruning considers only a
//! fixed list per harness ([`candidates`]) of what is loaded by default and
//! a coding session can do without. The work surface (shell, read, edit,
//! search), skills, plan mode and questions to the person are never on it.
//!
//! - Claude Code: single built-in tools, denied by name.
//! - Codex: feature packages (`-c features.<name>=false`). A hook reports
//!   the tool a call used, not its package, so a package counts as used
//!   when a call's tool name contains one of its words ([`used_candidates`]),
//!   and every package needs [`SLOW_FACTOR`] times the evidence.
//! - OpenCode: single built-in tools, turned off in its config.
//!
//! A candidate is denied once it has gone unused for
//! [`Thresholds::prompts`] prompts spread over at least
//! [`Thresholds::min_sessions`] sessions, or for [`Thresholds::sessions`]
//! sessions of any length. A candidate whose loss would be felt (web
//! access, notebooks) needs [`SLOW_FACTOR`] times as long. A tool used even
//! once is never denied, and neither is one the person gave back.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::Harness;

/// Built-in Claude Code tools pruning may deny, with how much more evidence
/// each needs (1 or [`SLOW_FACTOR`]).
pub const CLAUDE_CANDIDATES: &[(&str, u32)] = &[
    ("Agent", 1),
    ("CronCreate", 1),
    ("CronDelete", 1),
    ("CronList", 1),
    ("EnterWorktree", 1),
    ("ExitWorktree", 1),
    ("ListAgents", 1),
    ("Monitor", 1),
    ("NotebookEdit", SLOW_FACTOR),
    ("PushNotification", 1),
    ("ScheduleWakeup", 1),
    ("SendMessage", 1),
    ("TaskOutput", 1),
    ("TaskStop", 1),
    ("TodoWrite", 1),
    ("WebFetch", SLOW_FACTOR),
    ("WebSearch", SLOW_FACTOR),
    ("Workflow", 1),
];

/// Codex feature packages pruning may turn off (`codex features list`,
/// 0.164.0), each with the words that mark a call as using it.
pub const CODEX_CANDIDATES: &[(&str, u32, &[&str])] = &[
    ("multi_agent", SLOW_FACTOR, &["agent"]),
    ("goals", SLOW_FACTOR, &["goal"]),
    ("view_image", SLOW_FACTOR, &["view_image"]),
    (
        "image_generation",
        SLOW_FACTOR,
        &["image_gen", "generate_image"],
    ),
];

/// Built-in OpenCode tools pruning may turn off.
pub const OPENCODE_CANDIDATES: &[(&str, u32)] =
    &[("todowrite", 1), ("task", 1), ("webfetch", SLOW_FACTOR)];

/// The candidates for `harness`, with their evidence factor.
pub fn candidates(harness: Harness) -> Vec<(&'static str, u32)> {
    match harness {
        Harness::Claude => CLAUDE_CANDIDATES.to_vec(),
        Harness::Codex => CODEX_CANDIDATES
            .iter()
            .map(|(name, factor, _)| (*name, *factor))
            .collect(),
        Harness::Opencode => OPENCODE_CANDIDATES.to_vec(),
    }
}

/// The candidates a call to `tool` counts as using.
pub fn used_candidates(harness: Harness, tool: &str) -> Vec<&'static str> {
    match harness {
        Harness::Claude | Harness::Opencode => candidates(harness)
            .into_iter()
            .filter(|(name, _)| *name == tool)
            .map(|(name, _)| name)
            .collect(),
        Harness::Codex => {
            let tool = tool.to_ascii_lowercase();
            CODEX_CANDIDATES
                .iter()
                .filter(|(_, _, words)| words.iter().any(|word| tool.contains(word)))
                .map(|(name, _, _)| *name)
                .collect()
        }
    }
}

/// How many times the usual evidence a candidate whose loss would be felt
/// needs.
pub const SLOW_FACTOR: u32 = 3;

/// How long a candidate must go unused. TokenEconomizer's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub prompts: u32,
    pub min_sessions: u32,
    pub sessions: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            prompts: 10,
            min_sessions: 2,
            sessions: 5,
        }
    }
}

/// What the folder's sessions showed about one candidate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEvidence {
    /// Prompts since it was last used.
    pub unused_prompts: u32,
    /// Sessions since it was last used.
    pub unused_sessions: u32,
    /// It has been used in this folder; never denied after that.
    pub used: bool,
    /// The person gave it back; never denied again.
    pub restored: bool,
}

/// Every candidate's evidence in one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLedger {
    pub tools: BTreeMap<String, ToolEvidence>,
}

impl ToolLedger {
    /// Fold one finished session: `prompts` prompts, and the tools its calls
    /// used. A session with no prompt teaches nothing.
    pub fn fold_session(&mut self, harness: Harness, prompts: u32, used: &BTreeSet<String>) {
        if prompts == 0 {
            return;
        }
        let used: BTreeSet<&str> = used
            .iter()
            .flat_map(|tool| used_candidates(harness, tool))
            .collect();
        for (name, _) in candidates(harness) {
            let evidence = self.tools.entry(name.to_owned()).or_default();
            if used.contains(name) {
                evidence.used = true;
                evidence.unused_prompts = 0;
                evidence.unused_sessions = 0;
            } else {
                evidence.unused_prompts = evidence.unused_prompts.saturating_add(prompts);
                evidence.unused_sessions = evidence.unused_sessions.saturating_add(1);
            }
        }
    }

    /// The candidates to deny in the next session, sorted.
    pub fn deny_list(&self, harness: Harness, thresholds: &Thresholds) -> Vec<String> {
        candidates(harness)
            .into_iter()
            .filter(|(name, factor)| {
                self.tools.get(*name).is_some_and(|evidence| {
                    !evidence.used
                        && !evidence.restored
                        && ((evidence.unused_prompts >= thresholds.prompts * factor
                            && evidence.unused_sessions >= thresholds.min_sessions)
                            || evidence.unused_sessions >= thresholds.sessions * factor)
                })
            })
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    /// Give every denied tool back, for good. Returns the tools given back.
    pub fn restore(&mut self, harness: Harness, thresholds: &Thresholds) -> Vec<String> {
        let denied = self.deny_list(harness, thresholds);
        for name in &denied {
            if let Some(evidence) = self.tools.get_mut(name) {
                evidence.restored = true;
            }
        }
        denied
    }
}
