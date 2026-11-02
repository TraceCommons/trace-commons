//! The economizer: makes managed coding sessions (Claude Code, Codex and
//! OpenCode) cheaper by learning from the earlier sessions in the same
//! folder.
//!
//! It follows the design of TokenEconomizer (Li et al., "TokenEconomizer:
//! Experience-Driven Black-Box Cost Control for Coding Agents", 2026,
//! arXiv 2610.22732), written independently here in Rust. It has three
//! parts, each driven only by what the tool's hooks report ([`hook`]):
//!
//! - [`tools`]: a built-in tool (on Codex, a feature package) that goes
//!   unused for long enough is left out of the next sessions, so its
//!   definition stops being sent with every request.
//! - [`loop_guard`]: a turn that keeps hitting the same failure, and runs
//!   past how long this folder's turns usually take, gets one redirect.
//! - [`facts`]: a command spelling that fails as "not found" and then works
//!   another way, seen in three sessions, is told to the next sessions.
//!
//! What reaches the agent is model context only (`additionalContext`):
//! nothing is shown to the person, and nothing is written into their
//! folder. What is left out rides on the launch's own arguments or its
//! isolated profile ([`launch`]). The whole
//! economizer is off until the person turns it on (`near-ai economizer
//! on`), and a launch skips it for a resumed conversation and for a
//! session whose trace is collected ([`launch`]).
//!
//! State is local, owner-only, under `<config dir>/economizer`, and keyed
//! by a digest of the folder's path ([`store`]). It holds tool names,
//! counts, failure digests and program names, never file contents,
//! command output or paths.

pub mod cli;
pub mod facts;
pub mod hook;
pub mod launch;
pub mod loop_guard;
pub mod signals;
pub mod store;
pub mod summary;
pub mod tools;

/// The coding tool a session runs in. Each learns on its own: tool
/// evidence, facts and the envelope are kept per folder and harness.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    Claude,
    Codex,
    Opencode,
}

impl From<crate::managed::ToolId> for Harness {
    fn from(tool: crate::managed::ToolId) -> Self {
        match tool {
            crate::managed::ToolId::Claude => Self::Claude,
            crate::managed::ToolId::Codex => Self::Codex,
            crate::managed::ToolId::Opencode => Self::Opencode,
        }
    }
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

#[cfg(test)]
mod tests;
