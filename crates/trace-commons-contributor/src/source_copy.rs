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
    SOURCE_CLAUDE_CODE, SOURCE_CLINE, SOURCE_CODEX, SOURCE_GEMINI_CLI, SOURCE_OPENCODE,
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
    OpenCode,
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
            "opencode" => Some(Self::OpenCode),
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
            Self::OpenCode => "OpenCode",
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
            Self::OpenCode => SOURCE_OPENCODE,
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
    if tool == SourceTool::OpenCode {
        return match source_mode {
            "watch" => "OpenCode sessions are watched in the selected folder",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P71).
            "off" => "OpenCode collection turned off",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P74).
            "unset" => "No OpenCode folder selected; only managed launches are read",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P87).
            _ => "OpenCode settings could not be confirmed",
        }
        .into();
    }
    match source_mode {
        "watch" => format!("{name} traces folder set"),
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P72).
        "off" => format!("{name} marked not used, so nothing is opened for it"),
        _ if undeclared_scans_conventional(tool.adapter_name()) => {
            format!("{name} traces read from the usual place")
        }
        _ => format!("{name} is not set up, so nothing is opened for it"),
    }
}

/// Refusal for traces saved by an OpenCode version the reader does not
/// accept, shared by all native health surfaces. The versions match the
/// readers' `READABLE_VERSIONS` in `trace-commons-contributor-sources`.
/// Approved by the owner 2026-11-10; "this app" per the 2026-11-02 rule that
/// the commons' name never appears in user-visible copy.
pub const OPENCODE_VERSION_TITLE: &str =
    "Some OpenCode traces are from a version this app can't read yet.";
pub const OPENCODE_VERSION_DETAIL: &str = "This app reads traces saved by OpenCode 1.18.29 and \
     1.19.0 to 1.19.5. Traces from other versions are left out.";

/// Shared copy and source-policy metadata for editable native settings rows.
#[derive(serde::Serialize)]
pub struct SourceSettingsCopy {
    pub heading: &'static str,
    pub explanation: &'static str,
    pub save_failed: &'static str,
    /// A permission change that could not be confirmed; Uses' Refresh sits
    /// beside it. Approved by the owner 2026-11-04 (Settings copy pass 2, P139).
    pub consent_save_failed: &'static str,
    pub unavailable: &'static str,
    pub selected_folder: &'static str,
    pub no_candidate: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub watch_candidate: &'static str,
    pub choose_folder: &'static str,
    /// The accessible name of a row's Watch, decline and folder buttons:
    /// `{action}` is the button's visible words, `{tool}` the row's tool,
    /// so the same button on every row is told apart (copy-guide rule 5).
    /// DRAFT, NEEDS APPROVAL (2026-11-04).
    pub tool_action_accessibility_name: &'static str,
    pub retry: &'static str,
    /// The line under a found folder, saying what agreeing would mean.
    pub evidence: SourceEvidenceCopy,
    pub opencode_version_title: &'static str,
    pub opencode_version_detail: &'static str,
    pub tools: std::collections::BTreeMap<&'static str, SourceSettingsToolCopy>,
    /// The row for a declared folder of exported traces
    /// (`trajectory_source`), shown while `get_settings` reports
    /// `trajectory_source_mode` as `watch` or `off`.
    pub trajectory: TrajectorySettingsCopy,
}

/// The line under a folder discovery found, and how long ago its newest
/// trace was written. Moved verbatim from the macOS shell
/// (`SourceCandidate.evidence`, copy-guide rule 8); the shell picks the
/// line and fills it. "Not found" and "found but empty" stay apart: one
/// says the agent is not used, the other that it is and has no trace yet.
#[derive(serde::Serialize)]
pub struct SourceEvidenceCopy {
    pub missing: &'static str,
    pub empty: &'static str,
    pub count_one: &'static str,
    /// `{count}` traces.
    pub count: &'static str,
    /// `{traces}`: `count_one` or `count`; `{age}`: one of the ages below.
    pub most_recent: &'static str,
    /// `{evidence}`: the line so far, when an environment variable moved
    /// the folder.
    pub relocated: &'static str,
    pub age_just_now: &'static str,
    /// `{n}` minutes.
    pub age_minutes: &'static str,
    pub age_hour: &'static str,
    /// `{n}` hours.
    pub age_hours: &'static str,
    pub age_yesterday: &'static str,
    /// `{n}` days.
    pub age_days: &'static str,
}

/// The Watched folders row for a declared folder of exported traces. It is
/// no tool's store, so it is not one of `tools`: discovery never offers it,
/// and its exports are never sent unattended -- the watcher holds every one
/// for a person, whatever the folder's rule (`watcher::visit_session`'s
/// `from_trajectory`), and counts it as a decision owed. Owner decision
/// 2026-10-05: the declaration does not ship without this row.
///
/// Approved 2026-10-07, with the decline button cut to two words (owner
/// ruling: a button is an action verb, or a verb and an object).
#[derive(serde::Serialize)]
pub struct TrajectorySettingsCopy {
    /// The row's name.
    pub title: &'static str,
    /// What the folder's exports are subject to.
    pub explanation: &'static str,
    /// `trajectory_source_mode` is `watch`.
    pub watching: &'static str,
    /// `trajectory_source_mode` is `off`.
    pub off: &'static str,
    /// The off switch: writes `trajectory_source` `{"mode":"off"}`.
    pub decline: &'static str,
}

#[derive(serde::Serialize)]
pub struct SourceSettingsToolCopy {
    pub key: &'static str,
    pub decline: String,
    pub unset_scans_conventional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choose_folder: Option<&'static str>,
}

#[must_use]
pub fn source_settings_copy() -> SourceSettingsCopy {
    let tools = [
        ("claude", SourceTool::Claude),
        ("codex", SourceTool::Codex),
        ("gemini", SourceTool::Gemini),
        ("cline", SourceTool::Cline),
        ("opencode", SourceTool::OpenCode),
    ]
    .into_iter()
    .map(|(key, tool)| {
        (
            tool.adapter_name(),
            SourceSettingsToolCopy {
                key,
                // Two words at most (owner ruling, 2026-10-07): the row
                // already names the tool, so the button need not.
                decline: if tool == SourceTool::OpenCode {
                    "Don't read".into()
                } else {
                    "Not used".into()
                },
                explanation: (tool == SourceTool::OpenCode)
                    .then_some("Watch local OpenCode sessions or a folder of exports."),
                // Approved by the owner 2026-11-04 (Settings copy pass 2, P88).
                choose_folder: (tool == SourceTool::OpenCode).then_some("Choose folder…"),
                unset_scans_conventional: undeclared_scans_conventional(tool.adapter_name()),
            },
        )
    })
    .collect();
    SourceSettingsCopy {
        opencode_version_title: OPENCODE_VERSION_TITLE,
        opencode_version_detail: OPENCODE_VERSION_DETAIL,
        heading: "Watched folders",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P75).
        explanation: "Turning a tool off does not remove traces already queued.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P76).
        save_failed: "Couldn't confirm that change; the folders shown may be out of date.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P139).
        consent_save_failed: "Couldn't confirm that change. The ticks may be out of date.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P89).
        unavailable: "Folder settings aren't available.",
        selected_folder: "Selected folder",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P77).
        no_candidate: "No standard location found.",
        watch_candidate: "Watch",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P80).
        choose_folder: "Choose folder…",
        tool_action_accessibility_name: "{tool}: {action}",
        retry: "Retry",
        evidence: SourceEvidenceCopy {
            missing: "Not found on this machine",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P81).
            empty: "Found, no traces yet",
            count_one: "1 trace",
            count: "{count} traces",
            most_recent: "{traces}, most recent {age}",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P90).
            relocated: "{evidence} (set by an environment variable)",
            age_just_now: "just now",
            age_minutes: "{n} minutes ago",
            age_hour: "1 hour ago",
            age_hours: "{n} hours ago",
            age_yesterday: "yesterday",
            age_days: "{n} days ago",
        },
        tools,
        trajectory: TrajectorySettingsCopy {
            title: "Exported traces",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P73).
            explanation: "Exports here always wait for you; none is sent automatically.",
            watching: "A folder of exported traces is read.",
            off: "No folder of exported traces is read.",
            decline: "Stop reading",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kristi b#10 and the owner decision of 2026-10-05: a declared folder
    /// of exported traces has a Watched folders row of its own, saying that
    /// its exports always wait for a person, with a control to stop reading
    /// it. It names no path.
    #[test]
    fn a_trajectory_folder_has_a_row_that_says_exports_wait_for_you() {
        let payload = serde_json::to_value(source_settings_copy()).unwrap();
        let row = &payload["trajectory"];
        for key in ["title", "explanation", "watching", "off", "decline"] {
            assert!(
                row[key].as_str().is_some_and(|s| !s.trim().is_empty()),
                "{key}: {row}"
            );
        }
        assert!(
            row["explanation"]
                .as_str()
                .unwrap()
                .contains("wait for you")
        );
        assert_ne!(row["watching"], row["off"]);
    }

    /// The banner's version list is the readers' list: 1.18.29, then the
    /// contiguous 1.19 releases written as a range.
    fn assert_opencode_detail_names_every_readable_version() {
        let versions = crate::source::opencode::READABLE_VERSIONS;
        let minors: Vec<&str> = versions
            .iter()
            .copied()
            .filter(|v| v.starts_with("1.19."))
            .collect();
        let others: Vec<&str> = versions
            .iter()
            .copied()
            .filter(|v| !v.starts_with("1.19."))
            .collect();
        assert_eq!(others, ["1.18.29"]);
        let patches: Vec<u32> = minors.iter().map(|v| v[5..].parse().unwrap()).collect();
        assert!(
            patches.windows(2).all(|w| w[1] == w[0] + 1),
            "1.19 releases must be contiguous to be written as a range"
        );
        let expected = format!(
            "OpenCode 1.18.29 and {} to {}.",
            minors[0],
            minors[minors.len() - 1]
        );
        assert!(
            OPENCODE_VERSION_DETAIL.contains(&expected),
            "{OPENCODE_VERSION_DETAIL} lacks {expected}"
        );
    }

    #[test]
    fn export_refusal_payload_preserves_supported_version_and_scope() {
        let payload = serde_json::to_value(source_settings_copy()).unwrap();
        assert_eq!(payload["opencode_version_title"], OPENCODE_VERSION_TITLE);
        assert_eq!(payload["opencode_version_detail"], OPENCODE_VERSION_DETAIL);
        assert_opencode_detail_names_every_readable_version();
        // One refusal, one cause: the saving version. Not the database
        // format, and "trace", never "session".
        for text in [OPENCODE_VERSION_TITLE, OPENCODE_VERSION_DETAIL] {
            assert!(!text.contains("database"), "{text}");
            assert!(!text.to_lowercase().contains("session"), "{text}");
        }
    }

    #[test]
    fn settings_metadata_uses_adapter_policy_and_retains_queue_effects() {
        let copy = source_settings_copy();
        assert_eq!(copy.tools.len(), 5);
        for (adapter, tool) in &copy.tools {
            assert_eq!(
                tool.unset_scans_conventional,
                undeclared_scans_conventional(adapter)
            );
            let source = SourceTool::from_key(tool.key).expect("known source");
            // Owner dropped "Previously queued traces are not removed" from
            // each off line 2026-11-04 (Settings copy pass 2, P71, P72); the
            // section explanation (P75) says it once.
            assert!(
                source_check_line(source, "off").ends_with("turned off")
                    || source_check_line(source, "off")
                        .ends_with("marked not used, so nothing is opened for it")
            );
            assert!(
                copy.explanation
                    .contains("does not remove traces already queued")
            );
            if source == SourceTool::OpenCode {
                assert_eq!(tool.decline, "Don't read");
                assert!(
                    tool.explanation
                        .unwrap()
                        .contains("local OpenCode sessions")
                );
            } else {
                assert_eq!(tool.decline, "Not used");
            }
        }
        let payload = serde_json::to_value(copy).unwrap();
        assert_eq!(
            payload["tools"]["claude-code"]["unset_scans_conventional"],
            true
        );
        assert_eq!(
            payload["tools"]["gemini-cli"]["unset_scans_conventional"],
            false
        );
    }

    #[test]
    fn opencode_unknown_does_not_claim_off_and_exports_are_explicit() {
        for mode in ["", "future", "unavailable"] {
            let line = source_check_line(SourceTool::OpenCode, mode);
            assert!(line.contains("could not be confirmed"));
            assert_ne!(line, source_check_line(SourceTool::OpenCode, "off"));
        }
        assert!(source_check_line(SourceTool::OpenCode, "unset").contains("only managed launches"));
        assert!(source_check_line(SourceTool::OpenCode, "off").contains("turned off"));
        assert_opencode_detail_names_every_readable_version();
        // One refusal, one cause: the saving version. Not the database
        // format, and "trace", never "session".
        for text in [OPENCODE_VERSION_TITLE, OPENCODE_VERSION_DETAIL] {
            assert!(!text.contains("database"), "{text}");
            assert!(!text.to_lowercase().contains("session"), "{text}");
        }
        assert_eq!(SourceTool::from_key("opencode"), Some(SourceTool::OpenCode));
    }

    /// The defect, pinned per mode. `off` and `unset` were one sentence;
    /// they are three facts and they get three sentences.
    #[test]
    fn each_mode_gets_its_own_sentence() {
        assert_eq!(
            source_check_line(SourceTool::Claude, "watch"),
            "Claude Code traces folder set"
        );
        assert_eq!(
            source_check_line(SourceTool::Claude, "unset"),
            "Claude Code traces read from the usual place"
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
            "Cline traces folder set"
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
