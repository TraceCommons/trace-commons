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
//!
//! # Void notices
//!
//! The same module holds the notice a shell shows when R6 of the
//! connect-and-forget design voids a grant (`status.grant_voids`): that
//! automatic contributing stopped, for which project or for new projects,
//! why, and that it can be turned back on. [`void_notice_for_wire`] takes
//! one element of that list and returns the finished notice; across the ABI
//! it is `tc_grant_void_notice`. It is consent copy -- the other half of the
//! arming disclosure -- which is why it lives here.

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
/// Nothing in this crate checks R1 yet: the gate that will is planned
/// (`daemon::automatic_gate`, #1012, shipping unenforced first), and R1
/// enforcement is still pending. What keeps this copy off screen today is
/// only that no shell renders it. Until R1 can be met and is enforced, no
/// route qualifies and no shell may show it.
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
/// per-string export would let a shell take some of the six strings and
/// hand-write the rest, and several of them (the gate statement and the
/// three `auto_*` sentences) are claims about what leaves the machine.
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

/// The heading over a void notice's reasons.
pub const VOID_REASONS_HEADING: &str = "What changed";

/// The button that records a void notice as shown.
///
/// It acknowledges and does nothing else. Turning automatic contributing
/// back on is a separate act with its own disclosure, never a side effect of
/// dismissing the sentence that says it stopped.
pub const VOID_ACKNOWLEDGE: &str = "Got it";

/// What a project's void notice says happened.
pub const VOID_PROJECT_BODY: &str = "It was turned on under settings that have since changed, so it now asks before contributing. Its new sessions wait for you to review them, and nothing more is sent from it on its own.";

/// What the Flow 1 grant's void notice says happened.
pub const VOID_GRANT_BODY: &str = "It was turned on under settings that have since changed, so new projects now ask before contributing. Nothing is sent from a new project on its own.";

/// How a project is armed again, and what that means.
pub const VOID_PROJECT_REARM: &str = "You can turn automatic contributing back on for this project. Doing so agrees to the new settings.";

/// How the Flow 1 grant is given again, and what that means.
pub const VOID_GRANT_REARM: &str = "You can turn automatic contributing back on for new projects. Doing so agrees to the new settings.";

/// The title of a void this build cannot place: a `kind` it does not know,
/// or a project void without a label. It says what is certain -- automatic
/// contributing stopped -- and does not guess for what.
pub const VOID_UNPLACED_TITLE: &str = "Automatic contributing stopped";

/// What an unplaced void's notice says happened.
pub const VOID_UNPLACED_BODY: &str = "Settings it was turned on under have since changed, so it now asks before contributing. Check your projects to see which now ask first.";

/// How an unplaced void is turned back on.
pub const VOID_UNPLACED_REARM: &str = "You can turn automatic contributing back on wherever you want it. Doing so agrees to the new settings.";

/// The sentence for a reason label this build does not know.
///
/// A newer daemon may void for a reason an older shell has no sentence for.
/// The notice is still shown -- the grant still stopped -- with this line in
/// place of the one it lacks, rather than dropped for want of a sentence.
pub const VOID_REASON_UNKNOWN: &str = "Something else it was turned on under changed.";

/// The title of a void notice: the project it stopped for, or new projects
/// when it was the Flow 1 grant that stopped.
#[must_use]
pub fn void_notice_title(project_label: Option<&str>) -> String {
    match project_label {
        Some(label) => format!("Automatic contributing stopped for {label}"),
        None => "Automatic contributing stopped for new projects".to_string(),
    }
}

/// One reason label from the daemon's void rule, as a sentence.
///
/// The labels are the fixed set `daemon::grant_terms` defines and the audit
/// records (`docs/contributor-daemon-ipc-v1_1.md`, `auto-upload-voided`).
/// Each sentence names the change in terms of who would see a session or
/// what would leave with it, because that is what the grant was consent to.
#[must_use]
pub fn void_reason_line(label: &str) -> &'static str {
    match label {
        "destination-changed" => "Your sessions would now go to a different Trace Commons server.",
        "identity-changed" => {
            "Your sessions would now be sent under a different account, identity or device."
        }
        "scopes-widened" => "You now allow your contributions to be used in more ways.",
        "privacy-filter-changed" => {
            "The privacy filter that reads your sessions before they are sent was added, removed or changed."
        }
        "receipt-endpoint-changed" => {
            "Your AI provider would now be asked for receipts for the calls in your sessions, which tells it they are being contributed."
        }
        "witness-changed" => {
            "A different witness, the service that checks and scrubs your sessions before they are contributed, would now read them."
        }
        "witness-measurement-admitted" => {
            "A new version of the witness, the service that checks and scrubs your sessions before they are contributed, was approved to read them."
        }
        "attested-bodies-on" => {
            "The full text of your attested AI calls would now be sent with your sessions."
        }
        _ => VOID_REASON_UNKNOWN,
    }
}

/// Everything a shell shows for one void, assembled here.
///
/// The branch crosses, as with [`gate_help`]: a shell hands over the
/// project label (or none, for the Flow 1 grant) and the reason labels off
/// the wire, and gets back the finished notice. It never picks between the
/// project and grant wording, and never maps a label to a sentence itself.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct VoidNoticeCopy {
    pub title: String,
    pub body: &'static str,
    pub reasons_heading: &'static str,
    /// One sentence per reason label, in the daemon's order, duplicates
    /// removed. Never empty: a void with no label still says something
    /// changed.
    pub reasons: Vec<&'static str>,
    pub rearm: &'static str,
    pub acknowledge: &'static str,
}

/// The notice for one void. See [`VoidNoticeCopy`].
#[must_use]
pub fn void_notice(project_label: Option<&str>, reasons: &[String]) -> VoidNoticeCopy {
    let mut lines: Vec<&'static str> = Vec::new();
    for reason in reasons {
        let line = void_reason_line(reason);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(VOID_REASON_UNKNOWN);
    }
    let (body, rearm) = match project_label {
        Some(_) => (VOID_PROJECT_BODY, VOID_PROJECT_REARM),
        None => (VOID_GRANT_BODY, VOID_GRANT_REARM),
    };
    VoidNoticeCopy {
        title: void_notice_title(project_label),
        body,
        reasons_heading: VOID_REASONS_HEADING,
        reasons: lines,
        rearm,
        acknowledge: VOID_ACKNOWLEDGE,
    }
}

/// The notice for one element of `status.grant_voids`, as it came off the
/// wire.
///
/// The `kind` branch lives here with the rest, so no shell decides between
/// the project and the grant wording: `automatic_grant` gets the grant's
/// notice, and `project` the project's, titled with its `project_label`.
/// Any other object -- a kind this build does not know, or a project void
/// without a label -- gets the unplaced notice, which says automatic
/// contributing stopped without guessing for what. It is still a void, and
/// a shell left to word the fallback itself would be one more copy of it.
/// `None` only for a value that is not an object, which is not a void
/// element at all.
#[must_use]
pub fn void_notice_for_wire(void: &serde_json::Value) -> Option<VoidNoticeCopy> {
    let object = void.as_object()?;
    let reasons: Vec<String> = object
        .get("reasons")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|r| r.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let label = object
        .get("project_label")
        .and_then(serde_json::Value::as_str)
        .filter(|l| !l.is_empty());
    match (
        object.get("kind").and_then(serde_json::Value::as_str),
        label,
    ) {
        (Some("automatic_grant"), _) => Some(void_notice(None, &reasons)),
        (Some("project"), Some(label)) => Some(void_notice(Some(label), &reasons)),
        _ => {
            let placed = void_notice(None, &reasons);
            Some(VoidNoticeCopy {
                title: VOID_UNPLACED_TITLE.to_string(),
                body: VOID_UNPLACED_BODY,
                rearm: VOID_UNPLACED_REARM,
                ..placed
            })
        }
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

    /// Every reason the daemon's void rule can give has its own sentence.
    /// Listed from the daemon's constants, so a label added there without a
    /// sentence here fails this test rather than reaching a contributor as
    /// "something else changed".
    #[test]
    fn every_void_reason_the_daemon_gives_has_its_own_sentence() {
        use crate::daemon::grant_terms as g;
        let labels = [
            g::VOID_DESTINATION,
            g::VOID_IDENTITY,
            g::VOID_SCOPES_WIDENED,
            g::VOID_FILTER,
            g::VOID_RECEIPT_ENDPOINT,
            g::VOID_WITNESS,
            g::VOID_MEASUREMENT_ADMITTED,
            g::VOID_ATTESTED_BODIES,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for label in labels {
            let line = void_reason_line(label);
            assert_ne!(line, VOID_REASON_UNKNOWN, "{label} has no sentence");
            assert!(seen.insert(line), "{label} shares a sentence");
        }
    }

    /// The notice says plainly that automatic contributing stopped, for
    /// what, why, and that it can be turned back on -- the four things R6's
    /// ship condition asks a shell to say.
    #[test]
    fn a_project_notice_says_it_stopped_why_and_how_to_rearm() {
        let notice = void_notice(Some("api"), &["witness-measurement-admitted".to_string()]);
        assert_eq!(notice.title, "Automatic contributing stopped for api");
        assert!(notice.body.contains("asks before contributing"));
        assert!(
            notice
                .body
                .contains("nothing more is sent from it on its own")
        );
        assert_eq!(
            notice.reasons,
            vec![void_reason_line("witness-measurement-admitted")]
        );
        assert!(notice.rearm.contains("turn automatic contributing back on"));
        assert!(notice.rearm.contains("agrees to the new settings"));
        assert_eq!(notice.acknowledge, VOID_ACKNOWLEDGE);
    }

    /// The Flow 1 grant's notice is about new projects, not one project.
    #[test]
    fn the_grant_notice_is_about_new_projects() {
        let notice = void_notice(None, &["destination-changed".to_string()]);
        assert_eq!(
            notice.title,
            "Automatic contributing stopped for new projects"
        );
        assert_eq!(notice.body, VOID_GRANT_BODY);
        assert_eq!(notice.rearm, VOID_GRANT_REARM);
    }

    /// A label this build does not know still produces a notice, and an
    /// empty list still says something changed: the grant stopped either
    /// way, and a notice dropped for want of a sentence is a silent void.
    #[test]
    fn an_unknown_or_missing_reason_still_produces_a_notice() {
        let unknown = void_notice(Some("api"), &["from-a-newer-daemon".to_string()]);
        assert_eq!(unknown.reasons, vec![VOID_REASON_UNKNOWN]);
        let none = void_notice(Some("api"), &[]);
        assert_eq!(none.reasons, vec![VOID_REASON_UNKNOWN]);
        let repeated = void_notice(
            Some("api"),
            &["scopes-widened".to_string(), "scopes-widened".to_string()],
        );
        assert_eq!(repeated.reasons.len(), 1);
    }

    /// The copy is a notice, not a control: it never tells the contributor
    /// the grant is still on, and its button does not re-arm.
    #[test]
    fn the_acknowledge_button_does_not_claim_to_rearm() {
        let lower = VOID_ACKNOWLEDGE.to_lowercase();
        assert!(!lower.contains("turn on"));
        assert!(!lower.contains("keep"));
    }

    /// The wire's `kind` picks the wording here, not in a shell.
    #[test]
    fn the_wire_kind_picks_the_wording() {
        let project = void_notice_for_wire(&serde_json::json!({
            "id": 1, "kind": "project", "project_id": "p", "project_label": "api",
            "reasons": ["witness-changed"], "voided_at": "2026-09-26T00:00:00Z",
        }))
        .expect("a project notice");
        assert_eq!(
            project,
            void_notice(Some("api"), &["witness-changed".to_string()])
        );

        let grant = void_notice_for_wire(&serde_json::json!({
            "id": 2, "kind": "automatic_grant", "project_id": null, "project_label": null,
            "reasons": ["scopes-widened"], "voided_at": "2026-09-26T00:00:00Z",
        }))
        .expect("a grant notice");
        assert_eq!(grant, void_notice(None, &["scopes-widened".to_string()]));
    }

    /// A void this build cannot place still gets a notice, from here: it
    /// says automatic contributing stopped without guessing whether it was a
    /// project or the grant for new projects. Without this, each shell would
    /// write its own fallback sentence, which is the drift this module
    /// exists to prevent -- the macOS wording guard refuses one.
    #[test]
    fn a_void_that_cannot_be_placed_gets_a_notice_that_does_not_guess() {
        for value in [
            serde_json::json!({ "kind": "folder", "reasons": ["scopes-widened"] }),
            serde_json::json!({ "kind": "project", "reasons": ["scopes-widened"] }),
            serde_json::json!({ "kind": "project", "project_label": "", "reasons": [] }),
            serde_json::json!({ "reasons": [] }),
        ] {
            let notice = void_notice_for_wire(&value).unwrap_or_else(|| panic!("{value}"));
            assert_eq!(notice.title, VOID_UNPLACED_TITLE, "{value}");
            assert_eq!(notice.body, VOID_UNPLACED_BODY);
            assert_eq!(notice.rearm, VOID_UNPLACED_REARM);
            assert!(!notice.reasons.is_empty());
        }
        let placed = void_notice_for_wire(&serde_json::json!({
            "kind": "folder", "reasons": ["scopes-widened"],
        }))
        .unwrap();
        assert_eq!(placed.reasons, vec![void_reason_line("scopes-widened")]);
    }

    /// Only a value that is not an object at all is refused: that is not a
    /// void element, and there is nothing to say about it.
    #[test]
    fn a_value_that_is_not_an_element_is_refused() {
        for value in [
            serde_json::json!("project"),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            assert!(void_notice_for_wire(&value).is_none(), "{value}");
        }
    }
}
