//! What the settings screen says about each tool's sessions, in one place,
//! for all three shells.
//!
//! This is the neighbour of [`crate::routing_copy`] and it exists for the
//! same reason: the sentence below was written out three times -- once in
//! the GTK shell, once in the Windows view model, once in the macOS view --
//! and two of the three were wrong in the same way.
//!
//! # The bug this module exists to remove
//!
//! `get_settings` reports `*_root_configured`, which is `mode == "watch"`.
//! It is therefore **false for both `off` and `unset`**, and a shell that
//! branches on it prints one sentence for two different facts:
//!
//! - `unset` -- nobody was asked, so the daemon watches the conventional
//!   location. Sessions ARE being read. "read from the usual place" is
//!   true.
//! - `off` -- the contributor said they do not use this tool. No adapter is
//!   constructed and there is no fallback. Nothing is read. The same
//!   sentence is a **false statement in the fail-open direction**, on the
//!   one screen somebody checks to confirm a tool is not being read.
//!
//! `*_source_mode` carries the three-way answer, is already on the wire
//! (`daemon/ipc.rs`'s `redacted_settings`) and is already parsed by all
//! three shells. Nothing here needs a protocol change; it needs the words
//! to branch on the mode instead of on the boolean.
//!
//! # The mirror-image bug, which is worse
//!
//! `unset` is not "declared nothing". For `claude-code` and `codex` it is a
//! live scan of the contributor's real home
//! (`source::Undeclared::Conventional`). Telling that contributor nothing is
//! being read would be false in the fail-*closed* direction, which is the
//! worse of the two. The three modes get three sentences and
//! [`the_three_modes_never_share_a_sentence`] pins that they stay three.
//!
//! # What crosses the boundary
//!
//! Finished sentences, assembled here, exactly as `routing_copy` does it.
//! The tool's name is interpolated on this side from
//! [`crate::routing_copy`]'s own tool words, so a shell cannot pass
//! "Claude" and get a fourth spelling of the product's name. GTK links this
//! crate; macOS and Windows call `tc_source_check_line`.

use crate::routing_copy::{TOOL_CLAUDE, TOOL_CLINE, TOOL_CODEX, TOOL_GEMINI};

/// The tools the settings screen has a session-source row for.
///
/// A key rather than a free string across the ABI: the name in the sentence
/// comes from [`crate::routing_copy`], so the settings screen and the Tools
/// surface cannot come to call the same tool two things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTool {
    Claude,
    Codex,
    Gemini,
    Cline,
}

impl SourceTool {
    /// The wire key, as `get_settings` spells it in `*_source_mode`.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "cline" => Some(Self::Cline),
            _ => None,
        }
    }

    /// The tool's name as every surface in this app already spells it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => TOOL_CLAUDE,
            Self::Codex => TOOL_CODEX,
            Self::Gemini => TOOL_GEMINI,
            Self::Cline => TOOL_CLINE,
        }
    }
}

/// One tool's session-source row, in words, from `*_source_mode`.
///
/// The three modes and what each one is allowed to claim:
///
/// - `watch` -- the contributor pointed us at a folder. Says so, and does
///   not name it: the path never crosses the socket and there is nothing
///   here to print even if it did.
/// - `unset` -- nobody was asked, and the conventional location is being
///   scanned. Says that sessions are read, because they are.
/// - `off` -- the contributor said they do not use this tool. Says nothing
///   is opened for it.
///
/// # An unknown mode reads as `unset`
///
/// Deliberate, and it is the safe direction. The field is `#[serde(default)]`
/// in every shell, so an older daemon that does not send `*_source_mode` at
/// all yields an empty string -- and an older daemon is one whose `off`
/// declaration this build cannot see. Falling back to the `off` sentence
/// there would tell a contributor nothing is read from a tool that is being
/// scanned. Falling back to the `unset` sentence is the pre-existing
/// behaviour and claims no privacy.
///
/// # The `off` sentence is not built as a negation
///
/// "Private" is a substring of "Not private", and a `contains` check on this
/// surface has matched the wrong branch that way before. The `off` sentence
/// therefore shares no phrase with the other two -- not "folder set", not
/// "usual place", not the verb "read" -- rather than being either of them
/// with a "not" in front. [`the_three_modes_never_share_a_sentence`] pins
/// it.
#[must_use]
pub fn source_check_line(tool: SourceTool, source_mode: &str) -> String {
    let name = tool.name();
    match source_mode {
        "watch" => format!("{name} sessions folder set"),
        "off" => format!("{name} marked not used, so nothing is opened for it"),
        _ => format!("{name} sessions read from the usual place"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect, pinned per mode. `off` and `unset` were one sentence;
    /// they are three facts and they get three sentences.
    #[test]
    fn each_mode_gets_its_own_sentence() {
        assert_eq!(
            source_check_line(SourceTool::Claude, "watch"),
            "Claude Code sessions folder set"
        );
        assert_eq!(
            source_check_line(SourceTool::Claude, "unset"),
            "Claude Code sessions read from the usual place"
        );
        assert_eq!(
            source_check_line(SourceTool::Claude, "off"),
            "Claude Code marked not used, so nothing is opened for it"
        );
        assert_eq!(
            source_check_line(SourceTool::Codex, "off"),
            "Codex marked not used, so nothing is opened for it"
        );
        assert_eq!(
            source_check_line(SourceTool::Gemini, "unset"),
            "Gemini CLI sessions read from the usual place"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "watch"),
            "Cline sessions folder set"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "unset"),
            "Cline sessions read from the usual place"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "off"),
            "Cline marked not used, so nothing is opened for it"
        );
    }

    /// `off` must not say what `unset` says, `unset` must not say what `off`
    /// says, and neither may be the other with a word bolted on: a substring
    /// relation is how a `contains` check comes to match the wrong branch.
    #[test]
    fn the_three_modes_never_share_a_sentence() {
        for tool in [
            SourceTool::Claude,
            SourceTool::Codex,
            SourceTool::Gemini,
            SourceTool::Cline,
        ] {
            let watch = source_check_line(tool, "watch");
            let unset = source_check_line(tool, "unset");
            let off = source_check_line(tool, "off");
            for (a, b) in [
                (&watch, &unset),
                (&watch, &off),
                (&unset, &off),
                (&unset, &watch),
                (&off, &watch),
                (&off, &unset),
            ] {
                assert_ne!(a, b, "two modes render the same sentence: {a}");
                assert!(
                    !b.contains(a.as_str()),
                    "one mode's sentence contains another's: {b:?} contains {a:?}"
                );
            }
            // And the specific phrases, so that a rewrite cannot quietly put
            // the `unset` claim back into the `off` branch by other words.
            assert!(!off.contains("usual place"), "off claims a scan: {off}");
            assert!(!off.contains("folder set"), "off claims a folder: {off}");
            assert!(
                !off.to_lowercase().contains("read"),
                "off uses the verb the other two use: {off}"
            );
        }
    }

    /// A mode word this build does not know reads as `unset`, never as
    /// `off`. An older daemon sends no `*_source_mode` at all, and every
    /// shell defaults that to the empty string.
    #[test]
    fn an_unknown_mode_never_claims_that_nothing_is_read() {
        let unset = source_check_line(SourceTool::Claude, "unset");
        for mode in ["", "watching", "OFF", "disabled", "unknown"] {
            assert_eq!(
                source_check_line(SourceTool::Claude, mode),
                unset,
                "mode {mode:?} did not fall back to the unset sentence"
            );
        }
    }

    /// The name in the sentence is the Tools surface's name, so the two
    /// screens cannot come to spell a tool differently.
    #[test]
    fn the_tool_names_are_the_ones_the_tools_surface_uses() {
        assert_eq!(SourceTool::Claude.name(), TOOL_CLAUDE);
        assert_eq!(SourceTool::Codex.name(), TOOL_CODEX);
        assert_eq!(SourceTool::Gemini.name(), TOOL_GEMINI);
        assert_eq!(SourceTool::Cline.name(), TOOL_CLINE);
    }

    /// The keys are the ones `get_settings` uses, and anything else is
    /// `None` rather than a default tool -- a shell that asked for a tool
    /// this build does not have must get a refusal, not Claude Code's
    /// sentence under some other tool's heading.
    #[test]
    fn only_the_four_wire_keys_name_a_tool() {
        assert_eq!(SourceTool::from_key("claude"), Some(SourceTool::Claude));
        assert_eq!(SourceTool::from_key("codex"), Some(SourceTool::Codex));
        assert_eq!(SourceTool::from_key("gemini"), Some(SourceTool::Gemini));
        assert_eq!(SourceTool::from_key("cline"), Some(SourceTool::Cline));
        for key in ["", "Claude", "claude-code", "gemini-cli", "Cline", "near"] {
            assert_eq!(SourceTool::from_key(key), None, "{key:?} named a tool");
        }
    }
}
