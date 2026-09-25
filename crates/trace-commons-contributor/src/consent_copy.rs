//! The consent surface's words, in one place, for all three shells.
//!
//! Three sentences for the per-session gate, and the highest-value three in
//! the app: they are the safety claim shown at the instant of consent, above
//! an irreversible button. Three more, the `AUTO_*` constants, are the claim
//! made once at the Flow 1 grant, when no per-session gate will follow. They stood as four literals in each shell, because every shell
//! also kept its own copy of the branch that picks between the two help
//! sentences; [`gate_help`] is that fourth literal's replacement, and it is
//! not a sentence. Until this module existed the claim was written three
//! times --
//! `windows/src/TraceCommons.Interop/ReadGate.cs`,
//! `macos/Sources/TCShellCore/ReadGate.swift` and the GTK shell's
//! `copy.rs` -- and held together by a Rust test that opened the other two
//! shells' source files and grepped them for the exact text. That scaffold
//! is O(n) hand-written needles and only ever covered the sentences
//! somebody remembered to add.
//!
//! Not to be confused with `crate::consent`, which validates upload-claim
//! consent scopes and holds no copy at all.
//!
//! # What crosses the boundary
//!
//! The sentences cross already assembled, and so does the *branch*. A shell
//! does not receive two help sentences and choose between them: it calls
//! [`gate_help`] -- across the ABI, `tc_consent_gate_help` -- and receives
//! the chosen one. Three native copies of a two-way branch drift apart
//! silently while every string they return stays identical, which is the
//! failure this module exists to remove.
//!
//! GTK links this crate directly and re-exports these names; the macOS and
//! Windows shells reach them through `tc_consent_copy` and
//! `tc_consent_gate_help`.

/// The sentence that replaced the acknowledgement checkbox.
///
/// `Contribute` used to wait on three things: a pinned preview, the
/// "Exactly what would be sent" text having been on screen, and an
/// acknowledgement ticked by hand. Two of them are gone -- the checkbox as
/// friction, and the transcript-shown condition with it, because a queue
/// row's Submit approves the same session with no preview opened at all, so
/// the gate never stood between anybody and a blind approval.
///
/// What the checkbox *said* is not gone. It is this sentence, and it keeps
/// both halves of what the old gate was honest about: scrubbing is
/// pattern-based and may have missed something, and nothing in the app can
/// tell whether anyone read anything. Do not shorten it for layout; change
/// the layout.
pub const GATE_STATEMENT: &str = "\"Exactly what would be sent\" is the exact text that would leave this machine. Pattern-based scrubbing may have missed something in it, and nothing here checks that you looked.";

/// The tooltip on an armed `Contribute`.
///
/// The whole claim in four words: this button sends this session, and it
/// does not do anything else.
pub const GATE_READY_HELP: &str = "Sends this session. Nothing else.";

/// Why `Contribute` is off.
///
/// An approval binds to the envelope a preview pinned, and a preview built
/// without an enrollment pinned nothing, so there is nothing for an
/// approval to cover. Saying that beats a button that fails when pressed.
///
/// # The divergence this sentence settles
///
/// Windows said this ("This device isn't connected yet...") and macOS said
/// something else ("This preview hasn't loaded yet, so there is nothing
/// here to contribute.") -- two shells, two different explanations of why
/// the same button is off, because the two shells were also testing two
/// different conditions. This wording is the one that survived, for two
/// reasons: it names the condition both shells now test (an enrolled,
/// pinned preview), and the GTK shell already prints a near-identical
/// sentence in `UNENROLLED_PREVIEW`, so choosing it leaves one story rather
/// than two. The macOS condition moved to match; see the migration plan's
/// Task 7.
pub const GATE_NOT_PINNED_HELP: &str = "This device isn't connected yet, so this preview was built without your identity and nothing here can be contributed.";

/// What runs on every automatically contributed session, split by how
/// far each half can be trusted.
///
/// For the Flow 1 grant screen (the connect-and-forget design,
/// `docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`).
/// No shell renders it yet; it crosses now so that every shell has it before
/// the screen that uses it is built, rather than each shell writing its own.
///
/// **Shown only where a model pass runs on every automatic session.** On the
/// client the model pass is optional: it runs with `pii_filter = near-ai` or
/// `TRACE_PRIVACY_FILTER_BACKEND` set, and on a default config only the fixed
/// patterns run, which would make the second sentence untrue. The spec's R1
/// makes the witness's certified full pipeline a precondition of automatic
/// contribution, so a shell renders this only on the grant screen for a route
/// where that holds, never as a description of what a default config does.
/// Until R1 can be met (the gate in `daemon::automatic_gate`), no route
/// qualifies and no shell shows it.
///
/// The model clause is conditional on purpose -- "when a model recognises
/// it", not "by a model". [`AUTO_SCRUB_LIMIT`] says the model is not
/// reliable, and an unconditional promise here would contradict it two lines
/// down. "Employers" is deliberately absent: the classifier has no
/// organisation label, so an employer named in prose is never looked for.
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. Everything else -- names, people, addresses, account numbers, a password typed into a sentence -- is removed when a model recognises it.";

/// The limit of both halves, and the descendant of [`GATE_STATEMENT`]'s
/// second clause.
pub const AUTO_SCRUB_LIMIT: &str = "The patterns are reliable for the formats they cover and blind to everything else. The model is not reliable. Nothing here checks whether either of them was right.";

/// The one fact automatic contribution adds, and the one the old gate never
/// had to state.
///
/// This is the sentence the product will most want to soften and the one
/// that must not be. It is the whole difference between contributing
/// automatically and reviewing each session. Do not shorten it for layout;
/// change the layout.
pub const AUTO_NO_REVIEW: &str = "No one looks at a session before it is sent, including you.";

/// Every fixed string on this surface, in one payload.
///
/// Shaped for the C ABI: `tc_consent_copy` serialises this and hands the
/// shell one owned JSON object. One call and not one per string -- a
/// per-string export would let a shell take two of the three sentences and
/// hand-write the third, and the third is a claim about what leaves the
/// machine.
///
/// No version field, deliberately. A version implies a shell that can serve
/// two of them, and the cdylib and the shell ship together in one DMG, one
/// MSIX, one Flatpak. What is actually needed -- detection of a field that
/// stopped being exported -- is what each shell's refuse-on-any-empty-field
/// decode already does.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ConsentCopy {
    pub gate_statement: &'static str,
    pub ready_help: &'static str,
    pub not_pinned_help: &'static str,
    pub auto_scrub_scope: &'static str,
    pub auto_scrub_limit: &'static str,
    pub auto_no_review: &'static str,
}

/// The payload, built from the constants above.
#[must_use]
pub fn consent_copy() -> ConsentCopy {
    ConsentCopy {
        gate_statement: GATE_STATEMENT,
        ready_help: GATE_READY_HELP,
        not_pinned_help: GATE_NOT_PINNED_HELP,
        auto_scrub_scope: AUTO_SCRUB_SCOPE,
        auto_scrub_limit: AUTO_SCRUB_LIMIT,
        auto_no_review: AUTO_NO_REVIEW,
    }
}

/// The tooltip that explains the current answer.
///
/// THE BRANCH CROSSES, NOT ONLY THE WORDS. A shell that received both
/// sentences and chose between them would be keeping a third copy of this
/// decision, in a third language, with nothing to notice when one of them
/// stops matching. `pinned` is the shell's one condition: a preview that
/// parsed and carries an enrollment.
#[must_use]
pub fn gate_help(pinned: bool) -> &'static str {
    if pinned {
        GATE_READY_HELP
    } else {
        GATE_NOT_PINNED_HELP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statement, character for character.
    ///
    /// Written out here rather than compared to itself: this is the claim
    /// the product makes about redaction at the instant of consent, and the
    /// point of the assertion is that changing the sentence is a decision
    /// somebody has to make twice. This is the assertion the three shells
    /// used to hold one copy of each.
    ///
    /// The expectation is one unbroken literal on purpose. A `\` line
    /// continuation here would swallow the following indentation and pin a
    /// sentence with the wrong spacing in it, which is the shape of bug this
    /// assertion exists to catch.
    #[test]
    fn the_consent_statement_is_exactly_what_was_agreed() {
        assert_eq!(
            GATE_STATEMENT,
            "\"Exactly what would be sent\" is the exact text that would leave this machine. Pattern-based scrubbing may have missed something in it, and nothing here checks that you looked."
        );
    }

    /// The two things the removed checkbox used to make a contributor say
    /// out loud. Neither may quietly drop out of the sentence.
    #[test]
    fn the_statement_keeps_both_halves_of_what_the_checkbox_used_to_say() {
        assert!(GATE_STATEMENT.contains("Pattern-based scrubbing may have missed something"));
        assert!(GATE_STATEMENT.contains("nothing here checks that you looked"));
    }

    /// The branch crosses, not only the words.
    ///
    /// Without this function each shell keeps its own `? :` between the two
    /// help sentences, and three copies of a two-way branch can drift apart
    /// silently while every string stays identical.
    #[test]
    fn the_help_sentence_is_chosen_here_and_not_in_a_shell() {
        assert_eq!(gate_help(true), GATE_READY_HELP);
        assert_eq!(gate_help(false), GATE_NOT_PINNED_HELP);
    }

    /// The not-pinned sentence explains why the button is off without
    /// claiming the app knows something it does not.
    #[test]
    fn the_not_pinned_sentence_names_the_condition_the_shells_actually_test() {
        assert!(GATE_NOT_PINNED_HELP.contains("isn't connected yet"));
        assert!(GATE_NOT_PINNED_HELP.contains("nothing here can be contributed"));
        // Not a promise that pressing it later will work, and not an error.
        assert!(!GATE_NOT_PINNED_HELP.to_lowercase().contains("failed"));
        assert!(!GATE_NOT_PINNED_HELP.to_lowercase().contains("try again"));
    }

    /// Every field of the payload is a non-empty sentence, and the payload
    /// is exactly these six.
    ///
    /// Both shells refuse the whole payload when a field is empty, so an
    /// empty field here would blank a screen rather than fail a build. The
    /// macOS and Windows shells also check that the exported keys are exactly
    /// the ones they decode, so a field added here fails their builds until
    /// they consume it -- which is the point.
    #[test]
    fn the_payload_is_six_non_empty_sentences() {
        let value = serde_json::to_value(consent_copy()).expect("the payload serialises");
        let object = value.as_object().expect("a JSON object");
        let mut keys: Vec<&String> = object.keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "auto_no_review",
                "auto_scrub_limit",
                "auto_scrub_scope",
                "gate_statement",
                "not_pinned_help",
                "ready_help",
            ]
        );
        for (field, value) in object {
            assert!(
                !value.as_str().expect("every field is a string").is_empty(),
                "{field} is empty"
            );
        }
    }

    /// The automatic-contribution sentences, character for character, for
    /// the same reason `GATE_STATEMENT` is pinned: changing a claim about
    /// what leaves the machine should take a decision made twice.
    #[test]
    fn the_automatic_contribution_sentences_are_exactly_what_was_agreed() {
        assert_eq!(
            AUTO_SCRUB_SCOPE,
            "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. Everything else -- names, people, addresses, account numbers, a password typed into a sentence -- is removed when a model recognises it."
        );
        assert_eq!(
            AUTO_SCRUB_LIMIT,
            "The patterns are reliable for the formats they cover and blind to everything else. The model is not reliable. Nothing here checks whether either of them was right."
        );
        assert_eq!(
            AUTO_NO_REVIEW,
            "No one looks at a session before it is sent, including you."
        );
    }

    /// The scope sentence may not promise more than the scrubber does.
    ///
    /// Two overclaims were found in review and each is pinned out: an
    /// unconditional "removed by a model", which the limit sentence
    /// contradicts, and "employers", which the classifier has no label for.
    #[test]
    fn the_scope_sentence_does_not_overclaim() {
        assert!(AUTO_SCRUB_SCOPE.contains("when a model recognises it"));
        assert!(!AUTO_SCRUB_SCOPE.contains("removed by a model"));
        assert!(!AUTO_SCRUB_SCOPE.to_lowercase().contains("employer"));
    }

    /// No review is stated as a fact about everyone, the contributor
    /// included. A softened version ("usually", "may not") would make the
    /// grant a different act from the one described.
    #[test]
    fn no_review_is_stated_without_qualification() {
        assert!(AUTO_NO_REVIEW.starts_with("No one looks at a session before it is sent"));
        assert!(AUTO_NO_REVIEW.contains("including you"));
        for hedge in ["usually", "may ", "might", "generally", "typically"] {
            assert!(!AUTO_NO_REVIEW.contains(hedge), "hedged with {hedge:?}");
        }
    }
}
