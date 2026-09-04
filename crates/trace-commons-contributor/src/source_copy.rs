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
//! - `unset` -- nobody was asked. What happens then is the adapter's own
//!   [`crate::source::Undeclared`] policy: `claude-code` and `codex` watch
//!   the conventional location, so sessions ARE being read and "read from
//!   the usual place" is true of them; `gemini-cli` and `cline` construct
//!   no adapter at all and open nothing. One sentence for both would be
//!   false for one of them whichever one it claimed.
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
//! `unset` is not "declared nothing" for every tool. For `claude-code` and
//! `codex` it is a live scan of the contributor's real home
//! (`source::Undeclared::Conventional`). Telling that contributor nothing is
//! being read would be false in the fail-*closed* direction, which is the
//! worse of the two. So `unset` branches on the adapter's policy rather
//! than being one sentence, and
//! [`the_modes_never_share_a_sentence`] pins that every sentence a tool can
//! render stays distinct from the others.
//!
//! # What crosses the boundary
//!
//! Finished sentences, assembled here, exactly as `routing_copy` does it.
//! The tool's name is interpolated on this side from
//! [`crate::routing_copy`]'s own tool words, so a shell cannot pass
//! "Claude" and get a fourth spelling of the product's name. GTK links this
//! crate; macOS and Windows call `tc_source_check_line`.

use crate::routing_copy::{TOOL_CLAUDE, TOOL_CLINE, TOOL_CODEX, TOOL_GEMINI};
use crate::source::{
    SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI,
    undeclared_scans_conventional,
};

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

    /// The adapter name this tool's sessions are read by, which is what
    /// [`crate::source::undeclared_scans_conventional`] is keyed on. Not the
    /// wire key: `claude` on the settings wire is `claude-code` in the
    /// registration table, and the sentence must follow the table.
    fn adapter_name(self) -> &'static str {
        match self {
            Self::Claude => SOURCE_CLAUDE_CODE,
            Self::Codex => SOURCE_CODEX,
            Self::Gemini => SOURCE_GEMINI_CLI,
            Self::Cline => SOURCE_CLINE,
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
/// - `unset` -- nobody was asked, and what that means is the adapter's own
///   [`crate::source::Undeclared`] policy. For a tool whose undeclared
///   policy is a conventional scan, says sessions are read, because they
///   are. For one that constructs no adapter when undeclared, says the tool
///   is not set up and nothing is opened -- saying "read from the usual
///   place" there is the same fail-open falsehood, on the same screen, that
///   this module was written to remove.
/// - `off` -- the contributor said they do not use this tool. Says nothing
///   is opened for it.
///
/// The two "nothing is opened" sentences share that clause on purpose: they
/// are two reasons for one fact, and a shell that confused them would still
/// be telling the contributor the truth about whether anything is read.
/// They are still distinct sentences, and neither is a substring of the
/// other, because the contributor is owed the reason.
///
/// # An unknown mode reads as `unset`
///
/// Deliberate, and it is the safe direction. The field is `#[serde(default)]`
/// in every shell, so an older daemon that does not send `*_source_mode` at
/// all yields an empty string -- and an older daemon is one whose `off`
/// declaration this build cannot see. Falling back to the `off` sentence
/// there would tell a contributor nothing is read from a tool that is being
/// scanned. Falling back to the `unset` sentence is the pre-existing
/// behaviour and claims no privacy for the tools whose undeclared policy is
/// a scan.
///
/// For a `Nothing`-policy tool the fallback does claim that nothing is
/// opened, and that is still safe: no released daemon has ever scanned a
/// conventional location for one undeclared. Gemini CLI and Cline took that
/// policy in the commit that introduced them, precisely because every shell
/// already shipped declares only claude and codex and carries no field for
/// anything newer.
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
        _ if undeclared_scans_conventional(tool.adapter_name()) => {
            format!("{name} sessions read from the usual place")
        }
        _ => format!("{name} is not set up, so nothing is opened for it"),
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
        // Gemini CLI and Cline construct no adapter when undeclared, so the
        // scan sentence would be false for them in the fail-open direction.
        assert_eq!(
            source_check_line(SourceTool::Gemini, "unset"),
            "Gemini CLI is not set up, so nothing is opened for it"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "watch"),
            "Cline sessions folder set"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "unset"),
            "Cline is not set up, so nothing is opened for it"
        );
        assert_eq!(
            source_check_line(SourceTool::Cline, "off"),
            "Cline marked not used, so nothing is opened for it"
        );
    }

    /// No two modes may render the same sentence, and none may be another
    /// with a word bolted on: a substring relation is how a `contains` check
    /// comes to match the wrong branch. The two "nothing is opened"
    /// sentences share a clause and neither contains the other, which this
    /// checks rather than assumes.
    #[test]
    fn the_modes_never_share_a_sentence() {
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
            // the scan claim back into a branch that opens nothing by other
            // words. The `unset` branch of a tool that constructs no adapter
            // when undeclared is held to exactly the same rule as `off`: the
            // fact it reports is the same fact.
            let mut opens_nothing = vec![&off];
            if !crate::source::undeclared_scans_conventional(tool.adapter_name()) {
                opens_nothing.push(&unset);
            }
            for line in opens_nothing {
                assert!(!line.contains("usual place"), "claims a scan: {line}");
                assert!(!line.contains("folder set"), "claims a folder: {line}");
                assert!(
                    !line.to_lowercase().contains("read"),
                    "uses the verb a scan uses: {line}"
                );
            }
        }
    }

    /// A mode word this build does not know reads as `unset`, never as
    /// `off`. An older daemon sends no `*_source_mode` at all, and every
    /// shell defaults that to the empty string. For Claude Code and Codex
    /// that keeps the scan sentence, which is the fail-open direction and
    /// the safe one here.
    #[test]
    fn an_unknown_mode_never_claims_that_nothing_is_read() {
        let unset = source_check_line(SourceTool::Claude, "unset");
        assert!(unset.contains("usual place"));
        for mode in ["", "watching", "OFF", "disabled", "unknown"] {
            for tool in [SourceTool::Claude, SourceTool::Codex] {
                assert_eq!(
                    source_check_line(tool, mode),
                    source_check_line(tool, "unset"),
                    "mode {mode:?} did not fall back to the unset sentence"
                );
            }
        }
    }

    /// The sentence follows the registration table, not a second copy of it
    /// kept here. If an adapter's [`crate::source::Undeclared`] policy is
    /// ever changed, this is what makes the words change with it.
    #[test]
    fn the_unset_sentence_follows_the_adapters_undeclared_policy() {
        for tool in [
            SourceTool::Claude,
            SourceTool::Codex,
            SourceTool::Gemini,
            SourceTool::Cline,
        ] {
            let scans = crate::source::undeclared_scans_conventional(tool.adapter_name());
            assert_eq!(
                source_check_line(tool, "unset").contains("read from the usual place"),
                scans,
                "the unset sentence disagrees with {}'s undeclared policy",
                tool.adapter_name()
            );
        }
        // The policies as the table has them today, so that a change to the
        // table is a deliberate change to what the screen says.
        assert!(crate::source::undeclared_scans_conventional(
            SOURCE_CLAUDE_CODE
        ));
        assert!(crate::source::undeclared_scans_conventional(SOURCE_CODEX));
        assert!(!crate::source::undeclared_scans_conventional(
            SOURCE_GEMINI_CLI
        ));
        assert!(!crate::source::undeclared_scans_conventional(SOURCE_CLINE));
        // A name this build has no adapter for scans nothing.
        assert!(!crate::source::undeclared_scans_conventional("near"));
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
