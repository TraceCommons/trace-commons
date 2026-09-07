//! Holds all three shells to ONE harness-state table and ONE outcome table.
//!
//! `tc_harness_state_line`, `tc_harness_last_call_line` and
//! `tc_harness_outcome_line` make the sentence for a tool's state, the
//! sentence saying when a call last arrived, and the sentence a plan's
//! outcome carries, reachable from every shell. `tests/abi.rs` proves those
//! three exports agree
//! with `private_inference_copy` field for field. What it cannot prove is
//! that a shell actually asks: a shell that kept its own `switch` from a
//! state onto a payload field would pass every test in this repository while
//! quietly being the second -- and third -- place the mapping lives.
//!
//! So this file reads the three shells' source and requires each of them to
//! reach the shared table and to hold no table of its own. Between the two
//! files, "all three shells resolve the same state to the same sentence" is
//! checked end to end: each shell resolves only through the export, and the
//! export resolves only through the payload.
//!
//! It is a source scan, and it is deliberately blunt: naming one of the
//! per-state or per-outcome copy fields anywhere in a shell's harness file
//! fails, because
//! there is no longer a legitimate reason for a shell to name one. The
//! scanner strips comments and string literals first, so the prose above a
//! rule may still explain the rule.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // <repo>/crates/trace-commons-contributor-ffi
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate is two directories below the repo root")
        .to_path_buf()
}

/// The per-state and per-outcome sentence fields, in every spelling a shell
/// has for them.
///
/// Naming one of these in a shell is what a local table looks like. The
/// payload struct that DEFINES them is not scanned -- three shells each
/// carrying the whole payload is the point of the payload -- only the files
/// that used to choose between them.
///
/// THE OUTCOME SENTENCES ARE IN HERE FOR THE SAME REASON THE STATE ONES ARE,
/// and they arrived later: `unparseable` was the only outcome with a
/// sentence anywhere, and all three shells wrote that one arm themselves --
/// macOS in `outcomeSentence`, Windows in a `== Unparseable` branch, GNOME
/// in a `Some(PlanOutcome::Unparseable)` arm. Three copies of one decision,
/// with the other four outcomes drawing an empty preview. Ask
/// `tc_harness_outcome_line`; do not put the arms back.
const SENTENCE_FIELDS: &[&str] = &[
    // Swift
    "harnessNotConnected",
    "harnessUnreadableConfig",
    "harnessPlanNothingToChange",
    "harnessPlanEntryUnusable",
    "harnessPlanNoConfigPath",
    "harnessConnectedNothingSeen",
    "harnessAnswering",
    // C#
    "HarnessNotConnected",
    "HarnessUnreadableConfig",
    "HarnessPlanNothingToChange",
    "HarnessPlanEntryUnusable",
    "HarnessPlanNoConfigPath",
    "HarnessConnectedNothingSeen",
    "HarnessAnswering",
    // Rust
    "HARNESS_NOT_CONNECTED",
    "HARNESS_UNREADABLE_CONFIG",
    "HARNESS_PLAN_NOTHING_TO_CHANGE",
    "HARNESS_PLAN_ENTRY_UNUSABLE",
    "HARNESS_PLAN_NO_CONFIG_PATH",
    "HARNESS_CONNECTED_NOTHING_SEEN",
    "HARNESS_ANSWERING",
];

/// The way a shell would spell a money amount if it were formatting one
/// itself.
///
/// The amount is not chosen between fields, it is ASSEMBLED, so the failure
/// this catches is a different shape from a local table: a shell that
/// formatted its own dollars would round in its own way, put the currency in
/// its own place, and -- the part that matters -- decide for itself what an
/// absent figure looks like. That last decision is the one that turns
/// "nobody could measure this" into "$0.00", which is the defect this whole
/// surface is built to avoid. Ask `tc_harness_spend_line`.
const AMOUNT_FORMATTERS: &[&str] = &[
    // Swift
    ".currency(",
    "NumberFormatter",
    // C#
    "ToString(\"C",
    "\"C2\"",
    // Rust
    "${:.2}",
];

/// One shell's harness surface: where it decides, and what reaching the
/// shared table looks like from there.
struct Shell {
    /// The file that used to hold the local map.
    path: &'static str,
    /// Tokens that must all appear in it: its way of asking the one table.
    must_reach: &'static [&'static str],
}

/// The three shells, plus the two macOS files that carry its call across the
/// ABI -- `TCShellCore` is deliberately testable without linking the dylib,
/// so the symbol itself lives one layer out and would otherwise go unchecked.
const SHELLS: &[Shell] = &[
    Shell {
        path: "macos/Sources/TCShellCore/HarnessSurface.swift",
        must_reach: &["stateLine", "lastCallLine", "outcomeLine", "spendLine"],
    },
    Shell {
        path: "macos/Sources/TCBridge/TCHarness.swift",
        must_reach: &[
            "tc_harness_state_line",
            "tc_harness_last_call_line",
            "tc_harness_outcome_line",
            "tc_harness_spend_line",
        ],
    },
    Shell {
        path: "macos/Sources/TraceCommonsApp/AppModel.swift",
        must_reach: &["stateLine:", "lastCallLine:", "outcomeLine:", "spendLine:"],
    },
    Shell {
        path: "windows/src/TraceCommons.Interop/HarnessSurface.cs",
        must_reach: &[
            "NativeMethods.tc_harness_state_line",
            "NativeMethods.tc_harness_last_call_line",
            "NativeMethods.tc_harness_outcome_line",
            "NativeMethods.tc_harness_spend_line",
        ],
    },
    Shell {
        path: "windows/src/TraceCommons.Interop/NativeMethods.cs",
        must_reach: &[
            "tc_harness_state_line",
            "tc_harness_last_call_line",
            "tc_harness_outcome_line",
            "tc_harness_spend_line",
        ],
    },
    Shell {
        path: "crates/trace-commons-contributor-gtk/src/ui/private_inference.rs",
        must_reach: &[
            "harness_state_line",
            "harness_last_call_line",
            "harness_outcome_line",
            "harness_spend_line",
        ],
    },
];

/// Code with comments and string literals removed.
///
/// Both are dropped so a rule's own explanation, and any test fixture that
/// spells a sentence out, cannot be mistaken for a mapping. Handles `//`,
/// `/* */` and double-quoted strings with backslash escapes -- which is
/// every construct these six files use.
fn code_only(source: &str) -> String {
    let bytes: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
        } else if c == '"' {
            i += 1;
            while i < bytes.len() && bytes[i] != '"' {
                if bytes[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            out.push(' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn read(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is unreadable: {e}", path.display()))
}

/// No shell holds a state table or an outcome table of its own.
#[test]
fn no_shell_maps_a_harness_state_or_outcome_to_a_sentence_itself() {
    let root = repo_root();
    for shell in SHELLS {
        let code = code_only(&read(&root, shell.path));
        for field in SENTENCE_FIELDS {
            assert!(
                !code.contains(field),
                "{} names the sentence field `{field}`. That is a local \
                 harness table growing back: the sentence for a state, and \
                 the sentence for a plan outcome, are each decided once -- in \
                 `trace_commons_contributor::private_inference_copy::harness_state_line` \
                 and `::harness_outcome_line` -- and reached through \
                 `tc_harness_state_line` and `tc_harness_outcome_line`. Ask \
                 for one; do not choose between the payload's fields here.",
                shell.path
            );
        }
    }
}

/// Every shell reaches the one table, and the when-line with it.
///
/// The second half is the half that was broken: `harness_last_call_line`
/// existed in Rust and was not exported, so the shell that links the crate
/// natively drew the line and the two that go through the ABI drew nothing.
#[test]
fn every_shell_reaches_the_shared_table_and_the_when_line() {
    let root = repo_root();
    for shell in SHELLS {
        let code = code_only(&read(&root, shell.path));
        for token in shell.must_reach {
            assert!(
                code.contains(token),
                "{} no longer reaches `{token}`. All three shells render this \
                 surface, and each one that stops asking is one that has \
                 started deciding.",
                shell.path
            );
        }
    }
}

/// The three exports are declared in both copies of the header.
///
/// `abi_header_surface.rs` already holds the headers to the Rust surface;
/// this states the specific expectation in the specific place, so a header
/// edit that dropped them fails with a sentence about this surface.
#[test]
fn both_header_copies_declare_the_sentence_calls() {
    let root = repo_root();
    for header in [
        "crates/trace-commons-contributor-ffi/include/trace_commons.h",
        "macos/Sources/CTraceCommons/include/trace_commons.h",
    ] {
        let text = read(&root, header);
        assert!(
            text.contains("char*       tc_harness_state_line(const char* state);"),
            "{header} does not declare tc_harness_state_line"
        );
        assert!(
            text.contains("char*       tc_harness_last_call_line(int64_t seconds_ago);"),
            "{header} does not declare tc_harness_last_call_line"
        );
        assert!(
            text.contains("char*       tc_harness_outcome_line(const char* outcome);"),
            "{header} does not declare tc_harness_outcome_line"
        );
        assert!(
            text.contains("char*       tc_harness_spend_line(int64_t micros);"),
            "{header} does not declare tc_harness_spend_line"
        );
    }
}

/// No shell formats a money amount of its own.
#[test]
fn no_shell_assembles_an_amount_itself() {
    let root = repo_root();
    for shell in SHELLS {
        let code = read(&root, shell.path);
        for marker in AMOUNT_FORMATTERS {
            assert!(
                !code.contains(marker),
                "{} formats money itself (`{marker}`). The amount, its \
                 rounding, its window and -- above all -- what an UNKNOWN \
                 figure looks like are decided once, in \
                 `private_inference_copy::harness_spend_line`, and reached \
                 through `tc_harness_spend_line`. A shell that formats its \
                 own is a shell that will one day render not-known as zero.",
                shell.path
            );
        }
    }
}
