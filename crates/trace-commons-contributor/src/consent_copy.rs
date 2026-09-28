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
//!
//! # Witness capacity notice
//!
//! And the notice for approved sessions held because the privacy witness is
//! busy (`status.witness_capacity`, health label `witness-saturated`): why
//! they wait, that nothing is sent until the witness can check them, and
//! that nothing was lost. [`witness_capacity_notice_for_wire`] takes the
//! status object; across the ABI it is `tc_witness_capacity_notice`. Here
//! because it is the answer to "is anything going out without being
//! checked", which is a consent question.

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
///
/// The bearer-token sentence describes the cue-gated contextual-entropy
/// pass in `trace_commons_protocol::trace_contribution`
/// (`contextual_entropy_secret_ranges`), which is the only thing that
/// catches a bearer token outside the named formats in text: the cue regex
/// must match right before the value, the value must clear
/// `ENTROPY_BITS_MIN` (3.2 bits/char, which no value under 10 characters
/// can reach), and UUIDs and known ID prefixes are allowlisted. A value glued
/// onto "Bearer" with no separator is one token with no cue before it.
/// Pinned against the redactor by
/// `the_scrub_sentences_match_what_the_redactor_does`.
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. A bearer token in any other format is removed when it is long, looks random and follows \"Bearer\" after a space or colon, unless it is shaped like a UUID or another known kind of ID; one that is short, or run straight onto \"Bearer\", can get through. Everything else -- names, people, addresses, account numbers, a password typed into a sentence -- is removed when a model recognises it.";

/// The limit of both halves, and the descendant of [`GATE_STATEMENT`]'s
/// second clause.
///
/// "In text" is deliberate. On a string of text, the deterministic passes
/// are the named formats (secrets, PEM blocks, emails, paths) plus the
/// cue-gated contextual-entropy pass, and nothing else. Structured tool
/// payloads also get field-name rules (`redaction::redact_sensitive_json`)
/// and per-tool field rules, which remove whole values by where they sit
/// rather than by what they look like; this sentence makes no claim about
/// those.
pub const AUTO_SCRUB_LIMIT: &str = "The patterns are reliable for the formats they cover. Beyond those, in text they catch only a long, random-looking value right after a word like \"password\" or \"token\", and are blind to everything else. The model is not reliable. Nothing here checks whether either of them was right.";

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
/// back on is the other button, [`VOID_REARM_ACTION`], never a side effect of
/// dismissing the sentence that says it stopped.
pub const VOID_ACKNOWLEDGE: &str = "Got it";

/// What a project's void notice says happened.
pub const VOID_PROJECT_BODY: &str = "It was turned on under settings that have since changed, so it now asks before contributing. Its new sessions wait for you to review them, and nothing more is sent from it on its own.";

/// What the Flow 1 grant's void notice says happened.
pub const VOID_GRANT_BODY: &str = "It was turned on under settings that have since changed, so new projects now ask before contributing. Nothing is sent from a new project on its own.";

/// How a project is armed again, and what that means. Shown beside
/// [`VOID_REARM_ACTION`]: pressing that button is the fresh consent R6 asks
/// for, and this is the sentence that says so.
pub const VOID_PROJECT_REARM: &str = "You can turn automatic contributing back on for this project. Doing so agrees to the new settings.";

/// The button on a project's void notice that arms the project again, under
/// the terms now in force. A shell sends it as `set_project_mode` with the
/// element's `project_id` and `auto_upload` -- the same call, and the same
/// `armed-auto-upload` audit row, as arming the project by hand. Arming
/// clears the notice.
pub const VOID_REARM_ACTION: &str = "Turn back on";

/// Shown when the daemon refuses the re-arm (no config to record terms from,
/// a project it no longer knows, the unknown bucket). The notice stays, and
/// the project is exactly as it was.
pub const VOID_REARM_FAILED: &str =
    "It could not be turned back on, so it still asks first. Nothing was changed.";

/// What the Flow 1 grant's void notice says about projects, in place of a
/// re-grant line.
///
/// Worded to match `ProjectPolicy::sweep_grants`: the sweep that voids the
/// grant also voids every armed project whose own recorded terms widened --
/// usually all of them, since they are compared against the same terms --
/// and each of those gets its own notice. A project whose terms still cover
/// what is in force stays armed. So "unaffected" would be false.
///
/// K10: only the Tauri client can give the grant, through its Flow 1
/// screens. This sentence promises no re-grant, so it stays true in the
/// shells that cannot give one (macOS, Windows, GTK). A shell that can asks
/// for [`void_notice_for_wire_with_regrant`], which adds
/// [`VOID_GRANT_REGRANT`] and its button beside this sentence.
pub const VOID_GRANT_PROJECTS: &str = "Projects still set to contribute automatically carry on. Any project that stopped has its own notice.";

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
            "Your AI provider would now be asked for a receipt for the last call in each of your sessions, which tells it they are being contributed."
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
    /// The line after the reasons: how to turn it back on, or, for the Flow
    /// 1 grant, what happens to projects (see [`VOID_GRANT_PROJECTS`]).
    pub rearm: &'static str,
    pub acknowledge: &'static str,
    /// The re-arm button, for a project void that names the `project_id` it
    /// acts on, and `null` otherwise -- the grant and an unplaced void have
    /// no project to arm. A shell draws the button exactly when this is
    /// present.
    pub rearm_action: Option<&'static str>,
    /// What to say when the re-arm is refused; present exactly when
    /// `rearm_action` is.
    pub rearm_failed: Option<&'static str>,
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
    let (body, rearm, action) = match project_label {
        Some(_) => (VOID_PROJECT_BODY, VOID_PROJECT_REARM, true),
        None => (VOID_GRANT_BODY, VOID_GRANT_PROJECTS, false),
    };
    VoidNoticeCopy {
        title: void_notice_title(project_label),
        body,
        reasons_heading: VOID_REASONS_HEADING,
        reasons: lines,
        rearm,
        acknowledge: VOID_ACKNOWLEDGE,
        rearm_action: action.then_some(VOID_REARM_ACTION),
        rearm_failed: action.then_some(VOID_REARM_FAILED),
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
        (Some("project"), Some(label)) => {
            let notice = void_notice(Some(label), &reasons);
            // The button acts on the element's `project_id`; without one
            // there is nothing for it to arm.
            let has_id = object
                .get("project_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !id.is_empty());
            Some(if has_id {
                notice
            } else {
                VoidNoticeCopy {
                    rearm_action: None,
                    rearm_failed: None,
                    ..notice
                }
            })
        }
        _ => {
            let placed = void_notice(None, &reasons);
            Some(VoidNoticeCopy {
                title: VOID_UNPLACED_TITLE.to_string(),
                body: VOID_UNPLACED_BODY,
                rearm: VOID_UNPLACED_REARM,
                rearm_action: None,
                rearm_failed: None,
                ..placed
            })
        }
    }
}

/// How the Flow 1 grant is given again, for a shell that can give it.
///
/// **DRAFT, NEEDS APPROVAL.** Shown after [`VOID_GRANT_PROJECTS`], in the
/// spirit of [`VOID_PROJECT_REARM`]. The re-grant is not one click: the
/// button opens the grant screens again (scope, path, both disclosures, the
/// grant), because what changed is exactly what those screens disclose.
/// Giving the grant again clears the notice (`ProjectPolicy::grant_automatic`).
pub const VOID_GRANT_REGRANT: &str = "You can turn automatic contributing back on for new projects. You go through the same choices again, and doing so agrees to the new settings.";

/// The button beside [`VOID_GRANT_REGRANT`]. It opens the grant screens; it
/// gives nothing by itself.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const VOID_GRANT_REGRANT_ACTION: &str = "Review and turn back on";

/// A void notice for a shell that can give the Flow 1 grant.
///
/// `notice` is exactly [`void_notice_for_wire`]'s, flattened, so the shared
/// fields and their invariants are unchanged (`rearm_action` is still the
/// project button and still absent on the grant's notice). `regrant` and
/// `regrant_action` are present exactly on the grant's notice. A shell that
/// cannot give the grant keeps calling [`void_notice_for_wire`], and never
/// promises a re-grant it has no screen for.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct RegrantVoidNoticeCopy {
    #[serde(flatten)]
    pub notice: VoidNoticeCopy,
    pub regrant: Option<&'static str>,
    pub regrant_action: Option<&'static str>,
}

/// [`void_notice_for_wire`], plus the re-grant on the grant's notice. The
/// `kind` branch stays here, so the shell never decides which notice can be
/// re-granted.
#[must_use]
pub fn void_notice_for_wire_with_regrant(
    void: &serde_json::Value,
) -> Option<RegrantVoidNoticeCopy> {
    let notice = void_notice_for_wire(void)?;
    let is_grant = void.get("kind").and_then(serde_json::Value::as_str) == Some("automatic_grant");
    Some(RegrantVoidNoticeCopy {
        notice,
        regrant: is_grant.then_some(VOID_GRANT_REGRANT),
        regrant_action: is_grant.then_some(VOID_GRANT_REGRANT_ACTION),
    })
}

// ---------------------------------------------------------------------------
// The Flow 1 grant screens (K10, K11)
// ---------------------------------------------------------------------------
//
// Every constant in this section except the `AUTO_SCRUB_*` and
// `AUTO_NO_REVIEW` sentences above is DRAFT, NEEDS APPROVAL: written for the
// Tauri onboarding so that no shell writes its own, and not yet agreed in
// review. Each says so in its own doc comment. Only the Tauri client renders
// them; macOS, Windows and GTK do not take this payload yet.

/// What the fixed patterns remove, for an armed folder where no certified
/// full pipeline ran -- the spec's "deterministic-only arming disclosure".
///
/// **DRAFT, NEEDS APPROVAL.** The spec's Open list says this wording is not
/// yet written. The first two sentences are [`AUTO_SCRUB_SCOPE`]'s, word for
/// word, so `the_scrub_sentences_match_what_the_redactor_does` holds them to
/// the redactor too. The last replaces the model clause: it does not say no
/// model runs (a configured filter or witness may run one), only that nothing
/// confirms one did, which is what `automatic_gate::disclosure` answering
/// `PatternsOnly` means. "Trust relaxes what may be sent, never what may be
/// said."
pub const AUTO_PATTERNS_ONLY_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. A bearer token in any other format is removed when it is long, looks random and follows \"Bearer\" after a space or colon, unless it is shaped like a UUID or another known kind of ID; one that is short, or run straight onto \"Bearer\", can get through. Nothing confirms that a model checked these sessions, so treat names, people, addresses, account numbers and a password typed into a sentence as sent.";

/// The limit of the patterns, for the same route as
/// [`AUTO_PATTERNS_ONLY_SCOPE`].
///
/// **DRAFT, NEEDS APPROVAL.** [`AUTO_SCRUB_LIMIT`] without its model
/// sentence, since on this route no model's result is relied on.
pub const AUTO_PATTERNS_ONLY_LIMIT: &str = "The patterns are reliable for the formats they cover. Beyond those, in text they catch only a long, random-looking value right after a word like \"password\" or \"token\", and are blind to everything else. Nothing here checks whether they were right.";

/// The raw send, and both enclaves: the spec's first "further sentence".
/// Shown only where a pinned witness is configured, since only then does a
/// session leave unredacted.
///
/// **DRAFT, NEEDS APPROVAL.** States the spec's answer rather than its
/// question: the witness does not verify NEAR AI's attestation on each
/// classifier call, which puts the classifier's operator inside the
/// transcript's trust boundary, and the classifier receives the
/// deterministic-pass output rather than the unredacted session.
pub const AUTO_RAW_SEND_BOTH_ENCLAVES: &str = "Each session is sent unredacted to your witness, which redacts it inside an enclave. The witness can pass the text its fixed patterns leave to NEAR AI's privacy filter, which runs in a second enclave with a second operator, NEAR AI. The witness does not check that second enclave's attestation on each call, so NEAR AI's operator is trusted with that text.";

// Where the witness came from -- the spec's second "further sentence" -- is
// no longer one hedged constant here. The config now records the origin
// (`ContributorConfig::witness_origin`), so it is one sentence per origin,
// chosen by `witness_origin_line` from the daemon's `route_disclosure`
// facts. See "The disclosure screens" below.

/// Why the scope picker blocks the grant (R7), and what declining means.
///
/// **DRAFT, NEEDS APPROVAL.** R7: the picker has no default, and a
/// contributor who does not choose gets no grant and lands on Flow 2.
pub const AUTO_SCOPE_REQUIRED: &str = "Automatic contributing needs your choice of how your traces may be used. Nothing is selected for you. If you don't choose, nothing is contributed automatically and each session waits for you.";

/// The automatic path, as the path question offers it.
///
/// **DRAFT, NEEDS APPROVAL.** Worded to `grant_automatic`'s K3 and K4: it
/// arms projects discovered after the grant, and a project with any session
/// on disk at the grant keeps asking, for its new sessions too. "Have
/// sessions", not "already on this computer": the daemon exempts only
/// projects with sessions on disk (`AutomaticGrant::projects_on_disk` in
/// `daemon::policy`), so a folder that exists but holds no session yet is
/// armed when its first one lands.
pub const AUTO_PATH_AUTOMATIC: &str = "Contribute automatically from projects that first appear after you turn this on. Projects that already have sessions on this computer keep asking first.";

/// The ask-first path, as the path question offers it.
///
/// **DRAFT, NEEDS APPROVAL.** "Contributed", not "sent": reviewing with a
/// witness or the privacy scan sends a session somewhere before approval.
pub const AUTO_PATH_ASK_FIRST: &str = "Review each session yourself. Nothing is contributed until you approve it, and you can set a project to contribute automatically later.";

/// What the fixed patterns remove and where they stop, for one disclosure.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ScrubCopy {
    pub scope: &'static str,
    pub limit: &'static str,
}

/// Everything the Flow 1 grant screens say, for one disclosure.
///
/// THE BRANCH CROSSES, as with [`gate_help`]: exactly one of
/// `patterns_only` and `model_scrubbed` is present, the one `disclosure`
/// names, so a shell never holds the model-scrub wording on a route that did
/// not earn it and has no second copy of the choice to get wrong.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct AutomaticGrantCopy {
    /// `patterns_only` or `model_scrubbed`.
    pub disclosure: &'static str,
    pub patterns_only: Option<ScrubCopy>,
    pub model_scrubbed: Option<ScrubCopy>,
    pub no_review: &'static str,
    pub scope_required: &'static str,
    pub path_automatic: &'static str,
    pub path_ask_first: &'static str,
    pub raw_send: &'static str,
}

/// The grant screens' words for what `automatic_gate::disclosure` answered.
#[must_use]
pub fn automatic_grant_copy(
    disclosure: crate::daemon::automatic_gate::Disclosure,
) -> AutomaticGrantCopy {
    use crate::daemon::automatic_gate::Disclosure;
    let (label, patterns_only, model_scrubbed) = match disclosure {
        Disclosure::PatternsOnly => (
            "patterns_only",
            Some(ScrubCopy {
                scope: AUTO_PATTERNS_ONLY_SCOPE,
                limit: AUTO_PATTERNS_ONLY_LIMIT,
            }),
            None,
        ),
        Disclosure::ModelScrubbed => (
            "model_scrubbed",
            None,
            Some(ScrubCopy {
                scope: AUTO_SCRUB_SCOPE,
                limit: AUTO_SCRUB_LIMIT,
            }),
        ),
    };
    AutomaticGrantCopy {
        disclosure: label,
        patterns_only,
        model_scrubbed,
        no_review: AUTO_NO_REVIEW,
        scope_required: AUTO_SCOPE_REQUIRED,
        path_automatic: AUTO_PATH_AUTOMATIC,
        path_ask_first: AUTO_PATH_ASK_FIRST,
        raw_send: AUTO_RAW_SEND_BOTH_ENCLAVES,
    }
}

/// The title of the notice a shell shows while approved sessions wait on a
/// busy witness (`status.witness_capacity`, health label
/// `witness-saturated`).
pub const WITNESS_CAPACITY_TITLE: &str = "Waiting for the privacy witness";

/// The label beside the next retry time, which each shell renders in the
/// contributor's local time from `status.witness_capacity.next_retry_at`.
pub const WITNESS_CAPACITY_NEXT_CHECK: &str = "Next try";

/// The notice for sessions waiting on a busy witness. See
/// [`witness_capacity_notice`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WitnessCapacityCopy {
    pub title: &'static str,
    /// Why the sessions are waiting, that nothing is sent meanwhile, and
    /// that nothing was lost. Counted when the daemon gave a count.
    pub body: String,
    /// Shown with `next_retry_at` when the daemon gave one; omitted with it.
    pub next_check: &'static str,
}

/// The notice for `waiting_sessions` approved sessions held because the
/// privacy witness is busy. Zero words it without a count, for a shell that
/// has the health label and nothing else.
///
/// Not an error and not phrased as one: nothing is broken, nothing left the
/// machine, and the daemon retries on its own.
#[must_use]
pub fn witness_capacity_notice(waiting_sessions: u64) -> WitnessCapacityCopy {
    const REST: &str = "because the privacy witness is busy checking other sessions. Nothing is sent until the witness can check";
    let body = match waiting_sessions {
        0 => format!(
            "Approved sessions are waiting {REST} them. They will be tried again automatically, and nothing has been lost."
        ),
        1 => format!(
            "1 approved session is waiting {REST} it. It will be tried again automatically, and nothing has been lost."
        ),
        n => format!(
            "{n} approved sessions are waiting {REST} them. They will be tried again automatically, and nothing has been lost."
        ),
    };
    WitnessCapacityCopy {
        title: WITNESS_CAPACITY_TITLE,
        body,
        next_check: WITNESS_CAPACITY_NEXT_CHECK,
    }
}

/// [`witness_capacity_notice`] from `status.witness_capacity`, passed
/// through as the object the daemon sent. `None` when nothing is waiting,
/// or for a value that is not that object: there is nothing to show.
#[must_use]
pub fn witness_capacity_notice_for_wire(value: &serde_json::Value) -> Option<WitnessCapacityCopy> {
    let waiting = value.get("waiting_sessions")?.as_u64()?;
    (waiting > 0).then(|| witness_capacity_notice(waiting))
}

// ---------------------------------------------------------------------------
// The disclosure screens (K11): the raw send, both enclaves, where the
// witness came from
// ---------------------------------------------------------------------------
//
// Every constant in this section is DRAFT, NEEDS APPROVAL, like the grant
// screens' above. The facts come from the daemon's `route_disclosure`
// (`crate::disclosure`); these are the words for them. Each sentence states
// only what this client knows or does, and says so where it does not know:
// the witness's classifier is fixed by configuration its measurement covers,
// which this client never reads, and NEAR AI's attestation is checked by
// nobody on the classifier hop.

/// The disclosure panel's title.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_TITLE: &str = "Where your sessions go";

/// The route line when a witness is configured and refusing.
///
/// **DRAFT, NEEDS APPROVAL.** True of every refusing state
/// (`WitnessTrustState::is_refusing`): the submission path refuses before any
/// network call.
pub const DISCLOSURE_ROUTE_WITNESS_REFUSING: &str =
    "A witness is set up but this app cannot check it, so no session is sent until that is fixed.";

/// The route line with no witness configured.
///
/// **DRAFT, NEEDS APPROVAL.** With no witness the redactor runs in this
/// process (`envelope::build_redactor`), and only the redacted envelope is
/// uploaded. What an attached filter receives is the next sentence's job.
pub const DISCLOSURE_ROUTE_LOCAL: &str = "No witness is set up. Sessions are redacted on this computer, and the unredacted session does not leave it.";

/// The route line for a device that is not enrolled.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_ROUTE_NOT_ENROLLED: &str =
    "This computer is not connected to a commons, so nothing is sent.";

/// The route line when the configuration cannot be read.
///
/// **DRAFT, NEEDS APPROVAL.** A client that cannot read its settings sends
/// nothing (`WitnessTrustState::SettingsUnreadable` is a refusal).
pub const DISCLOSURE_ROUTE_SETTINGS_UNREADABLE: &str =
    "This app could not read its settings, so nothing is sent.";

/// Local route, no filter attached.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_LOCAL_FILTER_NONE: &str =
    "No privacy filter is attached, so only the fixed patterns run.";

/// Local route, NEAR AI's hosted privacy filter attached.
///
/// **DRAFT, NEEDS APPROVAL.** The client's NEAR AI adapter
/// (`NearAiPrivacyFilterAdapter`) makes an ordinary HTTPS call and checks no
/// attestation. The same trust statement `AUTO_RAW_SEND_BOTH_ENCLAVES` makes
/// for the witness's hop, made here for this computer's own.
pub const DISCLOSURE_LOCAL_FILTER_NEAR_AI: &str = "The text the fixed patterns leave is then sent from this computer to NEAR AI's privacy filter. This app does not check that service's attestation, so NEAR AI's operator is trusted with that text.";

/// Local route, a privacy-filter endpoint named in the environment.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_LOCAL_FILTER_SELF_HOSTED: &str = "The text the fixed patterns leave is then sent from this computer to a privacy filter named in its environment settings. This app does not check who runs it.";

/// Local route, a local sidecar program named in the environment.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_LOCAL_FILTER_SIDECAR: &str = "The text the fixed patterns leave is then passed to a program on this computer named in its environment settings.";

/// Local route, a filter setting the redactor refuses to build.
///
/// **DRAFT, NEEDS APPROVAL.** `build_redactor_with` refuses an unknown
/// `pii_filter`, and the protocol crate refuses a malformed environment
/// backend; neither falls back to patterns only.
pub const DISCLOSURE_LOCAL_FILTER_INVALID: &str = "The privacy filter setting cannot be used, so sessions are not redacted or sent until it is fixed.";

/// The witness block's heading.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_WITNESS_HEADING: &str = "Your witness";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_WITNESS_ADDRESS_LABEL: &str = "Address";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_WITNESS_SIGNING_LABEL: &str = "Signing key";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_WITNESS_MEASUREMENTS_LABEL: &str = "Pinned measurements";

/// What this client checks of the witness enclave, every time.
///
/// **DRAFT, NEEDS APPROVAL.** `witness::verify::verify_witness` is the only
/// constructor of the `VerifiedWitness` the transport requires: a fresh
/// nonce-bound quote whose measurement must match a pin and whose report
/// data must name the pinned signing address.
pub const DISCLOSURE_WITNESS_CHECK: &str = "Before a session is sent, this app asks the witness for a fresh attestation and sends nothing unless it matches a measurement pinned here and names this signing key.";

/// The second enclave, from what this client can and cannot see.
///
/// **DRAFT, NEEDS APPROVAL.** The classifier backend and endpoint are set
/// in the witness's measured compose file (`deploy/witness`), so they are
/// covered by the measurement, but this client holds only the measurement
/// value.
pub const DISCLOSURE_WITNESS_CLASSIFIER: &str = "Which privacy filter the witness calls is fixed by the setup its measurement covers. This app checks the measurement, but it does not read that setup and never sees the filter's attestation.";

/// Origin: published by the commons and saved at join (spec, point 2).
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_ORIGIN_PUBLISHED_AT_JOIN: &str = "The commons you joined published this witness, and it was saved when you joined, without asking you.";
/// Origin: installed from a connected inference selection (#1019).
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_ORIGIN_CONNECTED_INFERENCE: &str = "You chose this witness when you connected inference, and confirmed installing it on this computer.";
/// Origin: typed into Settings.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_ORIGIN_SETTINGS: &str =
    "This witness was entered in Settings on this computer.";
/// Origin: environment variables at enrollment.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_ORIGIN_ENVIRONMENT: &str =
    "This witness came from this computer's environment settings when it was connected.";
/// Origin unknown: no record, or a record for a different witness.
///
/// **DRAFT, NEEDS APPROVAL.** Replaces the grant screens' earlier
/// `AUTO_WITNESS_ORIGIN`, which had to hedge between two sources because
/// nothing was recorded. Now only a config written before the record
/// existed, or a witness changed by something that does not write one,
/// lands here, and the sentence names no source it cannot know.
pub const DISCLOSURE_ORIGIN_NOT_RECORDED: &str = "This app has no record of how this witness was set up. It was set up before this app kept one, or changed since by something that does not.";

/// Receipts off.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_RECEIPTS_OFF: &str =
    "No receipt endpoint is set, so this app asks no inference provider for receipts.";

/// Receipts on, `inference_receipt_check_attestation` on. Both receipt
/// sentences open with the fetch, because the fetch is itself a disclosure
/// (the spec's separate sentence: "each receipt fetch tells the provider
/// that an exchange is being contributed").
///
/// **DRAFT, NEEDS APPROVAL.** Worded to the config field's own limit: it
/// reads the report's self-description without verifying the quote.
pub const DISCLOSURE_RECEIPTS_CHECKED: &str = "Before a session goes to your witness, this app may ask the inference provider for a signed receipt of the session's last model call. Asking tells the provider that the exchange is being contributed. This app also checks that the receipt was signed by the key NEAR AI's attestation report names, reading the report without verifying its quote.";

/// Receipts on, the attestation check off.
///
/// **DRAFT, NEEDS APPROVAL.** Whether the witness pins receipt signers is
/// its configuration, invisible to this client (IPC contract, "The
/// attested-inference record").
pub const DISCLOSURE_RECEIPTS_UNCHECKED: &str = "Before a session goes to your witness, this app may ask the inference provider for a signed receipt of the session's last model call. Asking tells the provider that the exchange is being contributed. This app does not compare the receipt's signer with NEAR AI's attestation report, and whether your witness does depends on its setup, which this app cannot see.";

/// `ironwire_attested_bodies` on, witness route.
///
/// **DRAFT, NEEDS APPROVAL.** The bodies reach only a witness, never an
/// envelope (`DaemonSettings::ironwire_attested_bodies`).
pub const DISCLOSURE_ATTESTED_BODIES: &str = "When this app holds a verbatim copy of a session's last model call, your prompt and the reply, it sends that to your witness too. It is not part of what the commons receives.";

/// The per-session block's heading and labels.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_HEADING: &str = "What leaves this computer";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_BEFORE_LABEL: &str = "Before redaction";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_AFTER_LABEL: &str = "After redaction";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_BEFORE_WITNESS: &str =
    "Sent whole and unredacted to your witness, which redacts it in its enclave.";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_BEFORE_LOCAL: &str = "Stays on this computer.";
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_NOTHING_SENT: &str = "Nothing is sent.";
/// What the commons receives. "When it is contributed", not "when you
/// approve it": an armed project contributes without an approval.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const DISCLOSURE_SESSION_AFTER: &str = "What the commons receives when this session is contributed. This is what the redacted view shows.";

/// The per-session sentences.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct SessionSendCopy {
    pub heading: &'static str,
    pub before_label: &'static str,
    pub before_line: &'static str,
    pub after_label: &'static str,
    pub after_line: &'static str,
}

/// The witness block.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct WitnessDisclosureCopy {
    pub heading: &'static str,
    pub address_label: &'static str,
    pub signing_label: &'static str,
    pub measurements_label: &'static str,
    pub check: &'static str,
    /// The second enclave. Present only on the witness route: a refusing
    /// witness is sent nothing, so nothing reaches a classifier.
    pub classifier: Option<&'static str>,
    pub origin: &'static str,
}

/// Everything the disclosure panel says, for one set of facts.
///
/// THE BRANCH CROSSES, as with [`AutomaticGrantCopy`]: a block is present
/// only when it is true of the route, so a shell renders what it is given
/// and decides nothing.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct RouteDisclosureCopy {
    pub title: &'static str,
    pub route: &'static str,
    pub witness: Option<WitnessDisclosureCopy>,
    pub local_filter: Option<&'static str>,
    pub receipts: Option<&'static str>,
    pub attested_bodies: Option<&'static str>,
    pub session: SessionSendCopy,
}

/// The origin sentence for a witness.
#[must_use]
pub fn witness_origin_line(origin: crate::config::WitnessOriginView) -> &'static str {
    use crate::config::{WitnessOrigin, WitnessOriginView};
    match origin {
        WitnessOriginView::Recorded(WitnessOrigin::PublishedAtJoin) => {
            DISCLOSURE_ORIGIN_PUBLISHED_AT_JOIN
        }
        WitnessOriginView::Recorded(WitnessOrigin::ConnectedInference) => {
            DISCLOSURE_ORIGIN_CONNECTED_INFERENCE
        }
        WitnessOriginView::Recorded(WitnessOrigin::Settings) => DISCLOSURE_ORIGIN_SETTINGS,
        WitnessOriginView::Recorded(WitnessOrigin::Environment) => DISCLOSURE_ORIGIN_ENVIRONMENT,
        WitnessOriginView::NotRecorded => DISCLOSURE_ORIGIN_NOT_RECORDED,
    }
}

/// The local-route filter sentence.
#[must_use]
pub fn local_filter_line(filter: crate::disclosure::LocalFilter) -> &'static str {
    use crate::disclosure::LocalFilter;
    match filter {
        LocalFilter::None => DISCLOSURE_LOCAL_FILTER_NONE,
        LocalFilter::NearAi => DISCLOSURE_LOCAL_FILTER_NEAR_AI,
        LocalFilter::SelfHosted => DISCLOSURE_LOCAL_FILTER_SELF_HOSTED,
        LocalFilter::Sidecar => DISCLOSURE_LOCAL_FILTER_SIDECAR,
        LocalFilter::Invalid => DISCLOSURE_LOCAL_FILTER_INVALID,
    }
}

/// The disclosure panel's words for the daemon's `route_disclosure` facts.
#[must_use]
pub fn route_disclosure_copy(facts: &crate::disclosure::RouteDisclosure) -> RouteDisclosureCopy {
    use crate::disclosure::Route;
    let sends_to_witness = facts.route == Route::Witness;
    let route = match facts.route {
        Route::Witness => AUTO_RAW_SEND_BOTH_ENCLAVES,
        Route::WitnessRefusing => DISCLOSURE_ROUTE_WITNESS_REFUSING,
        Route::Local => DISCLOSURE_ROUTE_LOCAL,
        Route::NotEnrolled => DISCLOSURE_ROUTE_NOT_ENROLLED,
        Route::SettingsUnreadable => DISCLOSURE_ROUTE_SETTINGS_UNREADABLE,
    };
    let witness = facts.witness.as_ref().map(|w| WitnessDisclosureCopy {
        heading: DISCLOSURE_WITNESS_HEADING,
        address_label: DISCLOSURE_WITNESS_ADDRESS_LABEL,
        signing_label: DISCLOSURE_WITNESS_SIGNING_LABEL,
        measurements_label: DISCLOSURE_WITNESS_MEASUREMENTS_LABEL,
        check: DISCLOSURE_WITNESS_CHECK,
        classifier: sends_to_witness.then_some(DISCLOSURE_WITNESS_CLASSIFIER),
        origin: witness_origin_line(w.origin),
    });
    let receipts = sends_to_witness.then_some(match facts.receipts {
        r if !r.endpoint_configured => DISCLOSURE_RECEIPTS_OFF,
        r if r.check_attestation => DISCLOSURE_RECEIPTS_CHECKED,
        _ => DISCLOSURE_RECEIPTS_UNCHECKED,
    });
    let before_line = match facts.route {
        Route::Witness => DISCLOSURE_SESSION_BEFORE_WITNESS,
        Route::Local => DISCLOSURE_SESSION_BEFORE_LOCAL,
        Route::WitnessRefusing | Route::NotEnrolled | Route::SettingsUnreadable => {
            DISCLOSURE_SESSION_NOTHING_SENT
        }
    };
    RouteDisclosureCopy {
        title: DISCLOSURE_TITLE,
        route,
        witness,
        local_filter: (facts.route == Route::Local)
            .then(|| facts.local_filter.map(local_filter_line))
            .flatten(),
        receipts,
        attested_bodies: (sends_to_witness && facts.attested_bodies)
            .then_some(DISCLOSURE_ATTESTED_BODIES),
        session: SessionSendCopy {
            heading: DISCLOSURE_SESSION_HEADING,
            before_label: DISCLOSURE_SESSION_BEFORE_LABEL,
            before_line,
            after_label: DISCLOSURE_SESSION_AFTER_LABEL,
            after_line: DISCLOSURE_SESSION_AFTER,
        },
    }
}

/// The labels for `certificate_detail`, the per-session record of what the
/// witness was checked against when it reviewed this session.
///
/// **DRAFT, NEEDS APPROVAL.** `verification` is always
/// `verified_at_review` today: the claim is about that moment, not now.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CertificateDetailCopy {
    pub heading: &'static str,
    pub measurement_label: &'static str,
    pub signer_label: &'static str,
    pub verified_at_review: &'static str,
}

/// **DRAFT, NEEDS APPROVAL.**
#[must_use]
pub fn certificate_detail_copy() -> CertificateDetailCopy {
    CertificateDetailCopy {
        heading: "Checked when your witness reviewed this session",
        measurement_label: "Witness measurement",
        signer_label: "Signed by",
        verified_at_review: "This app checked the witness's attestation against your pins when the witness reviewed this session, and checked the certificate's signature against this signing key.",
    }
}

// ---------------------------------------------------------------------------
// Connecting inference (K12)
// ---------------------------------------------------------------------------
//
// The optional onboarding step through which a contributor connects an
// operator-published inference connection, the route to a witness for an
// invited contributor (connect-and-forget design, rev 7). Every sentence
// here is DRAFT, NEEDS APPROVAL. Each is worded to the daemon's contract in
// `docs/contributor-daemon-ipc-v1_1.md`, "Connecting inference". Only the
// Tauri client renders them.

/// What connecting inference offers, and that skipping it is fine.
///
/// **DRAFT, NEEDS APPROVAL.** "Changes nothing" rather than "stays on this
/// computer": a contributor who joined through NEAR AI may already have the
/// commons' witness, and skipping leaves that as it is.
pub const INFERENCE_WHY: &str = "Connecting inference is optional. It is one way to get a witness, a service that redacts your sessions inside an enclave before they are contributed. If you skip it, nothing about how your sessions are redacted changes.";

/// Why the step asks for account sign-in: every connection method presents
/// the account session, never the device key.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_SIGN_IN: &str =
    "Connecting needs your account, so sign in first. Skipping needs nothing.";

/// Account sign-in did not finish.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_SIGN_IN_FAILED: &str = "Sign-in did not finish. Try again, or skip this step.";

/// The offers or the current connection could not be read.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_LOAD_FAILED: &str =
    "The connections could not be read. Try again, or skip this step.";

/// When the commons publishes no offer.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_NONE_OFFERED: &str =
    "The commons you joined offers no inference connection right now. Continuing changes nothing.";

/// A selection grants no other consent.
///
/// **DRAFT, NEEDS APPROVAL.** The IPC contract: no folder, trace,
/// raw-session, consent-scope or standing contribution consent follows from
/// selecting or installing, and no project mode changes.
pub const INFERENCE_GRANTS_NOTHING: &str = "Connecting chooses no folders or sessions, does not change how your traces may be used, and does not turn on automatic contributing.";

/// One installed device per account, said before selecting.
///
/// **DRAFT, NEEDS APPROVAL.** Every select revokes the account's live
/// connection, so installing on another device later removes it here.
pub const INFERENCE_ONE_DEVICE: &str = "A connection works on one device per account. Connecting later from another device removes it from this one.";

/// Said before selecting when the account's selection is not installed on
/// this device, which means another device holds it.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_OTHER_DEVICE: &str = "Your account is connected on another device. Connecting here replaces that connection, and the other device stops using its witness.";

/// Said before installing: the separate step, and that it voids armed grants.
///
/// **DRAFT, NEEDS APPROVAL.** Installing changes the witness, a new recipient
/// under R6, so the watcher's next sweep voids every armed project and the
/// automatic grant given under the old terms, each with its own notice. The
/// IPC contract asks a shell to say so before the contributor confirms.
pub const INFERENCE_INSTALL: &str = "Using this connection's witness on this device is a separate step. It changes who reads your sessions, so any automatic contributing already turned on here stops, and you are asked again before it restarts.";

/// After install.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_INSTALLED: &str =
    "This device uses the witness from your inference connection.";

/// After a select that removed this device's earlier connection's witness
/// (`previous_witness_removed`).
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_PREVIOUS_REMOVED: &str = "The witness this device used before was removed, because choosing again ended that connection.";

/// The selection's revision was retired (`reselection_required`).
///
/// **DRAFT, NEEDS APPROVAL.** An installed witness from a retired revision
/// stays installed until the contributor reselects or disconnects.
pub const INFERENCE_RESELECT: &str = "The connection you chose is no longer offered in that form. Choose again to keep it current; until then, the witness already on this device stays.";

/// A refused or failed select.
///
/// **DRAFT, NEEDS APPROVAL.** A failed select holds nothing and installs
/// nothing on this device. It does not claim the account is unchanged: a
/// response refused after the server recorded the choice would make that
/// false.
pub const INFERENCE_SELECT_FAILED: &str =
    "The connection was not completed, and nothing was set up on this device.";

/// A refused or failed install: every refusal writes nothing.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_INSTALL_FAILED: &str =
    "The witness was not set up on this device, and nothing was written.";

/// What disconnecting does, in the daemon's order.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_DISCONNECT: &str = "Disconnecting removes this connection's witness from this device, then ends the connection on your account.";

/// A disconnect whose server half is still owed (`server_disconnect:
/// "pending"`): the local witness is already gone.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_DISCONNECT_PENDING: &str = "The witness was removed from this device, but the connection on your account could not be ended yet. Sign in and disconnect again to finish.";

/// An offer whose `disclosure_version` this build has no words for.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const INFERENCE_UNKNOWN_DISCLOSURE: &str =
    "This app cannot describe this connection, so it cannot be chosen here.";

/// The disclosure `inference-connection-disclosure-v1` names: what choosing
/// an offer means.
///
/// **DRAFT, NEEDS APPROVAL.** The offer carries a witness (URL, signing
/// address, pins) and, optionally, an inference receipt endpoint; the
/// receipt sentence follows `void_reason_line("receipt-endpoint-changed")`.
/// The server's own description text is never shown.
pub const INFERENCE_DISCLOSURE_V1: &str = "The commons you joined publishes this connection. Once it is set up on this device, sessions you contribute are sent unredacted to its witness first, which redacts them inside an enclave. If the connection includes a receipt address, your AI provider is also asked for a receipt for the last call in each of those sessions, which tells it they are being contributed.";

/// The words for an offer's `disclosure_version`, or `None` for a version
/// this build does not know. A shell that gets `None` does not offer the
/// connection: describing it in another version's words could be false.
#[must_use]
pub fn inference_connection_disclosure(disclosure_version: &str) -> Option<&'static str> {
    (disclosure_version == trace_commons_protocol::inference_connection::DISCLOSURE_VERSION)
        .then_some(INFERENCE_DISCLOSURE_V1)
}

/// Everything the connect-inference step says, except an offer's disclosure,
/// which [`inference_connection_disclosure`] picks per offer.
#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
pub struct InferenceConnectionCopy {
    pub why: &'static str,
    pub sign_in: &'static str,
    pub sign_in_failed: &'static str,
    pub load_failed: &'static str,
    pub none_offered: &'static str,
    pub grants_nothing: &'static str,
    pub one_device: &'static str,
    pub other_device: &'static str,
    pub install: &'static str,
    pub installed: &'static str,
    pub previous_removed: &'static str,
    pub reselect: &'static str,
    pub select_failed: &'static str,
    pub install_failed: &'static str,
    pub disconnect: &'static str,
    pub disconnect_pending: &'static str,
    pub unknown_disclosure: &'static str,
}

/// The connect-inference step's words.
#[must_use]
pub fn inference_connection_copy() -> InferenceConnectionCopy {
    InferenceConnectionCopy {
        why: INFERENCE_WHY,
        sign_in: INFERENCE_SIGN_IN,
        sign_in_failed: INFERENCE_SIGN_IN_FAILED,
        load_failed: INFERENCE_LOAD_FAILED,
        none_offered: INFERENCE_NONE_OFFERED,
        grants_nothing: INFERENCE_GRANTS_NOTHING,
        one_device: INFERENCE_ONE_DEVICE,
        other_device: INFERENCE_OTHER_DEVICE,
        install: INFERENCE_INSTALL,
        installed: INFERENCE_INSTALLED,
        previous_removed: INFERENCE_PREVIOUS_REMOVED,
        reselect: INFERENCE_RESELECT,
        select_failed: INFERENCE_SELECT_FAILED,
        install_failed: INFERENCE_INSTALL_FAILED,
        disconnect: INFERENCE_DISCONNECT,
        disconnect_pending: INFERENCE_DISCONNECT_PENDING,
        unknown_disclosure: INFERENCE_UNKNOWN_DISCLOSURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // K11: the route disclosure
    // -----------------------------------------------------------------

    fn facts(route: crate::disclosure::Route) -> crate::disclosure::RouteDisclosure {
        use crate::config::{WitnessOrigin, WitnessOriginView};
        use crate::disclosure::{LocalFilter, ReceiptFacts, Route, WitnessFacts};
        use crate::witness::status::WitnessTrustState;
        let witness = |state| WitnessFacts {
            state,
            url: "https://witness.invalid".into(),
            signing_address: "0xab".into(),
            pinned_measurements: vec!["mrtd=aa".into()],
            origin: WitnessOriginView::Recorded(WitnessOrigin::PublishedAtJoin),
        };
        crate::disclosure::RouteDisclosure {
            route,
            witness: match route {
                Route::Witness => Some(witness(WitnessTrustState::Pinned)),
                Route::WitnessRefusing => Some(witness(WitnessTrustState::RefusingUnpinned)),
                _ => None,
            },
            local_filter: (route == Route::Local).then_some(LocalFilter::None),
            receipts: ReceiptFacts {
                endpoint_configured: false,
                check_attestation: false,
            },
            attested_bodies: false,
        }
    }

    /// THE BRANCH CROSSES: each route carries only the blocks that are true
    /// of it, so a shell never holds the raw-send sentence for a session
    /// that stays on this computer, or a local-filter sentence for one that
    /// goes to a witness.
    #[test]
    fn each_route_carries_only_its_own_blocks() {
        use crate::disclosure::Route;
        let witness = route_disclosure_copy(&facts(Route::Witness));
        assert_eq!(witness.route, AUTO_RAW_SEND_BOTH_ENCLAVES);
        assert!(witness.local_filter.is_none());
        let w = witness.witness.as_ref().expect("the witness block");
        assert_eq!(w.classifier, Some(DISCLOSURE_WITNESS_CLASSIFIER));
        assert!(witness.receipts.is_some());
        assert_eq!(
            witness.session.before_line,
            DISCLOSURE_SESSION_BEFORE_WITNESS
        );

        let local = route_disclosure_copy(&facts(Route::Local));
        assert_eq!(local.route, DISCLOSURE_ROUTE_LOCAL);
        assert!(local.witness.is_none());
        assert!(local.receipts.is_none());
        assert!(local.attested_bodies.is_none());
        assert_eq!(local.local_filter, Some(DISCLOSURE_LOCAL_FILTER_NONE));
        assert_eq!(local.session.before_line, DISCLOSURE_SESSION_BEFORE_LOCAL);
        assert!(
            !serde_json::to_string(&local)
                .unwrap()
                .contains("unredacted to your witness")
        );

        let refusing = route_disclosure_copy(&facts(Route::WitnessRefusing));
        assert_eq!(refusing.route, DISCLOSURE_ROUTE_WITNESS_REFUSING);
        let w = refusing.witness.as_ref().expect("still shown");
        assert_eq!(w.classifier, None, "nothing reaches a classifier");
        assert!(refusing.receipts.is_none());
        assert_eq!(
            refusing.session.before_line,
            DISCLOSURE_SESSION_NOTHING_SENT
        );

        for (route, line) in [
            (Route::NotEnrolled, DISCLOSURE_ROUTE_NOT_ENROLLED),
            (
                Route::SettingsUnreadable,
                DISCLOSURE_ROUTE_SETTINGS_UNREADABLE,
            ),
        ] {
            let copy = route_disclosure_copy(&facts(route));
            assert_eq!(copy.route, line);
            assert!(copy.witness.is_none() && copy.local_filter.is_none());
            assert_eq!(copy.session.before_line, DISCLOSURE_SESSION_NOTHING_SENT);
        }
    }

    /// Every origin has its own sentence, and "not recorded" never names a
    /// source it cannot know.
    #[test]
    fn each_witness_origin_has_its_own_sentence() {
        use crate::config::{WitnessOrigin, WitnessOriginView};
        let views = [
            WitnessOriginView::Recorded(WitnessOrigin::PublishedAtJoin),
            WitnessOriginView::Recorded(WitnessOrigin::ConnectedInference),
            WitnessOriginView::Recorded(WitnessOrigin::Settings),
            WitnessOriginView::Recorded(WitnessOrigin::Environment),
            WitnessOriginView::NotRecorded,
        ];
        let lines: Vec<&str> = views.iter().map(|v| witness_origin_line(*v)).collect();
        for (i, a) in lines.iter().enumerate() {
            for b in &lines[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // The join case says the contributor was not asked (spec, point 2).
        assert!(lines[0].contains("without asking you"));
        assert!(lines[1].contains("connected inference"));
        assert!(lines[4].contains("no record"));
        for named in ["commons", "Settings", "connected"] {
            assert!(!lines[4].contains(named), "{named}");
        }
    }

    #[test]
    fn each_local_filter_has_its_own_sentence() {
        use crate::disclosure::LocalFilter;
        let lines: Vec<&str> = [
            LocalFilter::None,
            LocalFilter::NearAi,
            LocalFilter::SelfHosted,
            LocalFilter::Sidecar,
            LocalFilter::Invalid,
        ]
        .into_iter()
        .map(local_filter_line)
        .collect();
        for (i, a) in lines.iter().enumerate() {
            for b in &lines[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // The NEAR AI filter is a hop this app does not attest.
        assert!(lines[1].contains("NEAR AI"));
        assert!(lines[1].contains("does not check"));
    }

    /// Receipts: the fetch is a disclosure to the provider (spec, the
    /// separate sentence), and the attestation check is described as the
    /// consistency check it is, not as verification.
    #[test]
    fn the_receipt_sentences_follow_the_configuration() {
        use crate::disclosure::{ReceiptFacts, Route};
        let mut f = facts(Route::Witness);
        let none = route_disclosure_copy(&f).receipts.unwrap();
        assert_eq!(none, DISCLOSURE_RECEIPTS_OFF);
        f.receipts = ReceiptFacts {
            endpoint_configured: true,
            check_attestation: false,
        };
        let unchecked = route_disclosure_copy(&f).receipts.unwrap();
        assert!(unchecked.contains("tells the provider"));
        assert!(unchecked.contains("does not compare"));
        f.receipts.check_attestation = true;
        let checked = route_disclosure_copy(&f).receipts.unwrap();
        assert!(checked.contains("tells the provider"));
        assert!(checked.contains("without verifying its quote"));
    }

    #[test]
    fn attested_bodies_are_said_only_on_the_witness_route_when_on() {
        use crate::disclosure::Route;
        let mut f = facts(Route::Witness);
        assert!(route_disclosure_copy(&f).attested_bodies.is_none());
        f.attested_bodies = true;
        assert_eq!(
            route_disclosure_copy(&f).attested_bodies,
            Some(DISCLOSURE_ATTESTED_BODIES)
        );
        let mut local = facts(Route::Local);
        local.attested_bodies = true;
        assert!(route_disclosure_copy(&local).attested_bodies.is_none());
    }

    /// No disclosure sentence uses the banned destination label, and none
    /// is empty.
    #[test]
    fn the_disclosure_sentences_are_present_and_use_no_banned_label() {
        use crate::disclosure::Route;
        for route in [
            Route::Witness,
            Route::WitnessRefusing,
            Route::Local,
            Route::NotEnrolled,
            Route::SettingsUnreadable,
        ] {
            let wire = serde_json::to_string(&route_disclosure_copy(&facts(route))).unwrap();
            assert!(!wire.to_lowercase().contains("private inference"), "{wire}");
            assert!(!wire.contains("\"\""), "{wire}");
        }
    }

    #[test]
    fn the_certificate_detail_copy_says_what_was_checked_and_when() {
        let copy = certificate_detail_copy();
        assert!(copy.verified_at_review.contains("when"));
        assert!(!copy.heading.is_empty());
        assert!(!copy.measurement_label.is_empty());
        assert!(!copy.signer_label.is_empty());
    }

    /// "Trust relaxes what may be sent, never what may be said": the
    /// patterns-only payload carries neither model-scrub sentence anywhere,
    /// and claims no model removed anything.
    #[test]
    fn the_patterns_only_grant_copy_never_carries_the_model_scrub_wording() {
        use crate::daemon::automatic_gate::Disclosure;
        let copy = automatic_grant_copy(Disclosure::PatternsOnly);
        assert_eq!(copy.disclosure, "patterns_only");
        assert!(copy.model_scrubbed.is_none());
        let wire = serde_json::to_string(&copy).expect("serialises");
        let escaped = |s: &str| serde_json::to_string(s).expect("serialises");
        for sentence in [AUTO_SCRUB_SCOPE, AUTO_SCRUB_LIMIT] {
            let inner = escaped(sentence);
            assert!(!wire.contains(inner.trim_matches('"')));
        }
        assert!(!wire.contains("when a model recognises it"));
        assert!(!wire.contains("The model is not reliable"));
        // The pattern half is the agreed sentence's, so the redactor test
        // holds it.
        let pattern_half = AUTO_SCRUB_SCOPE
            .split(" Everything else")
            .next()
            .expect("the agreed scope sentence");
        assert!(AUTO_PATTERNS_ONLY_SCOPE.starts_with(pattern_half));
    }

    /// The model-scrub route gets the agreed sentences and nothing of the
    /// patterns-only route.
    #[test]
    fn the_model_scrubbed_grant_copy_is_the_agreed_wording() {
        use crate::daemon::automatic_gate::Disclosure;
        let copy = automatic_grant_copy(Disclosure::ModelScrubbed);
        assert_eq!(copy.disclosure, "model_scrubbed");
        assert!(copy.patterns_only.is_none());
        assert_eq!(
            copy.model_scrubbed,
            Some(ScrubCopy {
                scope: AUTO_SCRUB_SCOPE,
                limit: AUTO_SCRUB_LIMIT,
            })
        );
        assert_eq!(copy.no_review, AUTO_NO_REVIEW);
    }

    /// Every grant-screen sentence is non-empty, and the draft sentences do
    /// not soften the no-review sentence or promise a model pass.
    #[test]
    fn the_draft_grant_sentences_do_not_overclaim() {
        for sentence in [
            AUTO_PATTERNS_ONLY_SCOPE,
            AUTO_PATTERNS_ONLY_LIMIT,
            AUTO_RAW_SEND_BOTH_ENCLAVES,
            AUTO_SCOPE_REQUIRED,
            AUTO_PATH_AUTOMATIC,
            AUTO_PATH_ASK_FIRST,
        ] {
            assert!(!sentence.is_empty());
            assert!(!sentence.contains("removed by a model"));
            assert!(!sentence.contains("when a model recognises it"));
        }
        // Two enclaves and two operators, not one.
        assert!(AUTO_RAW_SEND_BOTH_ENCLAVES.contains("unredacted"));
        assert!(AUTO_RAW_SEND_BOTH_ENCLAVES.contains("second enclave"));
        assert!(AUTO_RAW_SEND_BOTH_ENCLAVES.contains("second operator"));
        // No default scope, and declining is not a floor-scope grant.
        assert!(AUTO_SCOPE_REQUIRED.contains("Nothing is selected for you"));
        // The exemption is for projects with sessions on disk at the grant,
        // not every folder that exists: an empty one is armed.
        assert!(AUTO_PATH_AUTOMATIC.contains("already have sessions"));
        assert!(!AUTO_PATH_AUTOMATIC.contains("already on this computer"));
    }

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
            "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. A bearer token in any other format is removed when it is long, looks random and follows \"Bearer\" after a space or colon, unless it is shaped like a UUID or another known kind of ID; one that is short, or run straight onto \"Bearer\", can get through. Everything else -- names, people, addresses, account numbers, a password typed into a sentence -- is removed when a model recognises it."
        );
        assert_eq!(
            AUTO_SCRUB_LIMIT,
            "The patterns are reliable for the formats they cover. Beyond those, in text they catch only a long, random-looking value right after a word like \"password\" or \"token\", and are blind to everything else. The model is not reliable. Nothing here checks whether either of them was right."
        );
        assert_eq!(
            AUTO_NO_REVIEW,
            "No one looks at a session before it is sent, including you."
        );
    }

    /// The two sentences make claims about the deterministic redactor, and
    /// each claim is held to it here, with no model configured.
    ///
    /// `AUTO_SCRUB_LIMIT` says that beyond the named formats, text is caught
    /// only when a long, random-looking value sits right after a cue word.
    /// `AUTO_SCRUB_SCOPE` says which bearer tokens are removed and which can
    /// get through. If the redactor changes so that one of these flips, the
    /// sentence has stopped being true and has to be decided again.
    #[test]
    fn the_scrub_sentences_match_what_the_redactor_does() {
        use trace_commons_protocol::trace_contribution::DeterministicTraceRedactor;

        let redactor = DeterministicTraceRedactor::deterministic_only(Vec::new());
        let removed = |text: &str, value: &str| {
            let (out, _) = redactor.redact_text(text);
            !out.contains(value)
        };
        let random = "q7Vx2LpZ9kWm4Rt8Ns3Hb6Yd";
        let random_long = "q7Vx2LpZ9kWm4Rt8Ns3Hb6YdJc5Gf1Ke";

        // AUTO_SCRUB_LIMIT: a random-looking value right after a cue word is
        // caught without a model...
        assert!(removed(&format!("password: {random}"), random));
        assert!(removed(&format!("token={random}"), random));
        // ...but not one that is not right after it, nor one that does not
        // look random.
        assert!(!removed(&format!("my password is {random} ok"), random));
        let plain = "aaaaaaaaaaaaaaaaaaaa";
        assert!(!removed(&format!("password: {plain}"), plain));

        // AUTO_SCRUB_SCOPE: a long, random bearer token after a space or
        // colon is removed.
        assert!(removed(
            &format!("Authorization: Bearer {random_long}"),
            random_long
        ));
        assert!(removed(&format!("Bearer:{random_long}"), random_long));
        // A bearer token in a known format is removed by its own pattern.
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(removed(&format!("Bearer {jwt}"), jwt));
        // UUID-shaped, a known kind of ID, short, or run onto the word: each
        // can get through.
        let uuid = "3f2b8c1e-9a4d-4e7b-8c21-5d6f7a8b9c0d";
        assert!(!removed(&format!("Bearer {uuid}"), uuid));
        let id_shaped = format!("resp_{random}");
        assert!(!removed(&format!("Bearer {id_shaped}"), &id_shaped));
        let short = "a8Fk2Lq9z";
        assert!(!removed(&format!("Bearer {short}"), short));
        assert!(!removed(&format!("Bearer{random_long}"), random_long));
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

    /// A receipt is sought for one call per session -- the last one, the
    /// only call the witness attests (`trace_commons_protocol::
    /// witness_provenance`) -- not for every call in it.
    #[test]
    fn the_receipt_reason_names_the_last_call_not_every_call() {
        let line = void_reason_line(crate::daemon::grant_terms::VOID_RECEIPT_ENDPOINT);
        assert!(
            !line.contains("the calls in your sessions"),
            "reads as a receipt for every call: {line:?}"
        );
        assert!(
            line.contains("the last call in each of your sessions"),
            "does not name the call a receipt covers: {line:?}"
        );
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
        // The one-click re-arm: the button, and what to say if the daemon
        // refuses it. The sentence beside it (`rearm`) says pressing it
        // agrees to the new settings.
        assert_eq!(notice.rearm_action, Some(VOID_REARM_ACTION));
        assert_eq!(notice.rearm_failed, Some(VOID_REARM_FAILED));
    }

    /// The re-arm button is the fresh consent R6 asks for, so its words are
    /// an action on this project, and the refusal line says nothing changed.
    #[test]
    fn the_rearm_button_turns_it_back_on_and_a_refusal_changes_nothing() {
        assert_eq!(VOID_REARM_ACTION, "Turn back on");
        assert!(VOID_REARM_FAILED.contains("still asks first"));
        assert!(VOID_PROJECT_REARM.contains("agrees to the new settings"));
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
        assert_eq!(notice.rearm, VOID_GRANT_PROJECTS);
        // The shared notice is for every shell, and only Tauri can give the
        // grant again, so it neither offers it nor promises it. Tauri adds
        // the re-grant through `void_notice_for_wire_with_regrant`.
        assert_eq!(notice.rearm_action, None);
        assert_eq!(notice.rearm_failed, None);
        assert!(!notice.rearm.contains("back on"), "{}", notice.rearm);
    }

    /// What the grant's notice says about projects must match the sweep: a
    /// project whose own terms widened is voided with its own notice, and
    /// one still armed carries on. It must not say every project is
    /// unaffected, because the same widening usually voids them too.
    #[test]
    fn the_grant_notice_says_what_happens_to_projects_as_the_sweep_does() {
        assert!(VOID_GRANT_PROJECTS.contains("still set to contribute automatically"));
        assert!(VOID_GRANT_PROJECTS.contains("its own notice"));
        assert!(!VOID_GRANT_PROJECTS.to_lowercase().contains("unaffected"));
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
            // No project to arm, so no button.
            assert_eq!(notice.rearm_action, None, "{value}");
            assert_eq!(notice.rearm_failed, None, "{value}");
        }
        let placed = void_notice_for_wire(&serde_json::json!({
            "kind": "folder", "reasons": ["scopes-widened"],
        }))
        .unwrap();
        assert_eq!(placed.reasons, vec![void_reason_line("scopes-widened")]);
    }

    /// A project void offers the button only when it carries the
    /// `project_id` the button acts on; without one there is nothing to arm.
    #[test]
    fn a_project_void_without_an_id_gets_no_button() {
        let with_id = void_notice_for_wire(&serde_json::json!({
            "kind": "project", "project_id": "p", "project_label": "api", "reasons": [],
        }))
        .unwrap();
        assert_eq!(with_id.rearm_action, Some(VOID_REARM_ACTION));
        for id in [
            serde_json::json!(null),
            serde_json::json!(""),
            serde_json::json!(3),
        ] {
            let without = void_notice_for_wire(&serde_json::json!({
                "kind": "project", "project_id": id, "project_label": "api", "reasons": [],
            }))
            .unwrap();
            assert_eq!(without.title, "Automatic contributing stopped for api");
            assert_eq!(without.rearm_action, None, "{id}");
            assert_eq!(without.rearm_failed, None, "{id}");
        }
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

    /// The saturation notice says the three things a contributor needs:
    /// why sessions are waiting, that nothing is sent meanwhile, and that
    /// nothing was lost. With the count when the daemon gave one, agreeing
    /// in number.
    #[test]
    fn the_witness_capacity_notice_says_why_and_that_nothing_is_sent() {
        let one = witness_capacity_notice(1);
        let many = witness_capacity_notice(3);
        let uncounted = witness_capacity_notice(0);
        assert_eq!(one.title, WITNESS_CAPACITY_TITLE);
        assert!(
            one.body.starts_with("1 approved session is waiting"),
            "{}",
            one.body
        );
        assert!(
            many.body.starts_with("3 approved sessions are waiting"),
            "{}",
            many.body
        );
        assert!(!uncounted.body.contains('0'), "{}", uncounted.body);
        for notice in [&one, &many, &uncounted] {
            let lower = notice.body.to_lowercase();
            assert!(lower.contains("privacy witness is busy"), "{}", notice.body);
            assert!(lower.contains("nothing is sent until"), "{}", notice.body);
            assert!(lower.contains("nothing has been lost"), "{}", notice.body);
            for word in ["error", "failed", "503", "saturated", "capacity"] {
                assert!(!lower.contains(word), "{word} in: {}", notice.body);
            }
            assert_eq!(notice.next_check, WITNESS_CAPACITY_NEXT_CHECK);
        }
    }

    /// The wire object is `status.witness_capacity`, passed through. Nothing
    /// to say when nothing is waiting, or for a value that is not the object.
    #[test]
    fn the_witness_capacity_notice_reads_the_status_object() {
        let wire = serde_json::json!({
            "waiting_sessions": 2, "next_retry_at": "2030-01-01T00:01:00Z",
        });
        assert_eq!(
            witness_capacity_notice_for_wire(&wire),
            Some(witness_capacity_notice(2))
        );
        for value in [
            serde_json::json!({"waiting_sessions": 0, "next_retry_at": null}),
            serde_json::json!({"next_retry_at": null}),
            serde_json::json!({"waiting_sessions": -1}),
            serde_json::json!({"waiting_sessions": "2"}),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            assert_eq!(witness_capacity_notice_for_wire(&value), None, "{value}");
        }
    }

    /// K10: a shell that can give the Flow 1 grant (Tauri) gets the grant
    /// notice with a re-grant sentence and button. The notice itself is the
    /// shared one, unchanged: the project-arming button stays absent, since
    /// there is no project to arm, and the re-grant is a separate field.
    #[test]
    fn a_shell_that_can_regrant_offers_it_on_the_grant_notice() {
        let wire = serde_json::json!({
            "id": 5, "kind": "automatic_grant", "project_id": null,
            "project_label": null, "reasons": ["witness-changed"],
        });
        let copy = void_notice_for_wire_with_regrant(&wire).expect("a notice");
        assert_eq!(copy.notice, void_notice_for_wire(&wire).unwrap());
        assert_eq!(copy.notice.rearm_action, None);
        assert_eq!(copy.regrant, Some(VOID_GRANT_REGRANT));
        assert_eq!(copy.regrant_action, Some(VOID_GRANT_REGRANT_ACTION));
        // Flattened, so a shell reads the same fields plus the two new ones.
        let value = serde_json::to_value(&copy).expect("serialises");
        assert_eq!(value["rearm"], VOID_GRANT_PROJECTS);
        assert_eq!(value["regrant"], VOID_GRANT_REGRANT);
        assert_eq!(value["regrant_action"], VOID_GRANT_REGRANT_ACTION);
    }

    /// Only the grant's notice is re-granted. A project notice keeps its own
    /// "Turn back on", and an unplaced one offers nothing.
    #[test]
    fn only_the_grant_notice_carries_the_regrant() {
        for wire in [
            serde_json::json!({
                "id": 1, "kind": "project", "project_id": "p",
                "project_label": "api", "reasons": ["scopes-widened"],
            }),
            serde_json::json!({ "id": 2, "kind": "folder", "reasons": [] }),
        ] {
            let copy = void_notice_for_wire_with_regrant(&wire).expect("a notice");
            assert_eq!(copy.notice, void_notice_for_wire(&wire).unwrap());
            assert_eq!(copy.regrant, None, "{wire}");
            assert_eq!(copy.regrant_action, None, "{wire}");
        }
        assert!(void_notice_for_wire_with_regrant(&serde_json::json!("x")).is_none());
    }

    /// The re-grant is fresh consent: it goes back through the choices and
    /// agrees to the settings now in force, and it is about new projects.
    #[test]
    fn the_regrant_sentence_says_it_goes_through_the_choices_again() {
        assert!(VOID_GRANT_REGRANT.contains("new projects"));
        assert!(VOID_GRANT_REGRANT.contains("same choices again"));
        assert!(VOID_GRANT_REGRANT.contains("agrees to the new settings"));
        assert!(!VOID_GRANT_REGRANT_ACTION.is_empty());
    }

    /// K12: an offer is described only in a disclosure version this build
    /// knows. A version it does not know gets no words, so a shell cannot
    /// offer that connection rather than describe it wrongly.
    #[test]
    fn a_connection_is_described_only_in_a_known_disclosure_version() {
        let known = trace_commons_protocol::inference_connection::DISCLOSURE_VERSION;
        assert_eq!(
            inference_connection_disclosure(known),
            Some(INFERENCE_DISCLOSURE_V1)
        );
        assert_eq!(
            inference_connection_disclosure("inference-connection-disclosure-v2"),
            None
        );
        assert_eq!(inference_connection_disclosure(""), None);
    }

    /// What connecting inference says before each step, held to the daemon's
    /// contract (`docs/contributor-daemon-ipc-v1_1.md`, "Connecting
    /// inference").
    #[test]
    fn the_inference_connection_copy_says_what_the_daemon_does() {
        let copy = inference_connection_copy();
        // Optional, and one route to a witness, not the door to anything.
        assert!(copy.why.contains("optional"));
        assert!(copy.why.contains("witness"));
        // A selection grants no other consent.
        for word in ["folders", "used", "automatic contributing"] {
            assert!(copy.grants_nothing.contains(word), "{word}");
        }
        // Installing changes the witness, which voids armed grants.
        assert!(copy.install.contains("separate"));
        assert!(copy.install.contains("stops"));
        // One installed device per account.
        assert!(copy.one_device.contains("one device"));
        assert!(copy.other_device.contains("stops using"));
        // A retired revision keeps the installed witness until chosen again.
        assert!(copy.reselect.contains("stays"));
        // A refused install writes nothing.
        assert!(copy.install_failed.contains("nothing was written"));
        assert!(copy.previous_removed.contains("removed"));
        assert!(copy.disconnect.contains("removes"));
        let value = serde_json::to_value(copy).expect("serialises");
        for (key, field) in value.as_object().expect("an object") {
            assert!(field.as_str().is_some_and(|s| !s.is_empty()), "{key}");
        }
    }

    /// Every route that writes a witness has its own origin sentence, now
    /// that connecting inference is one. (This replaced a single hedged
    /// `AUTO_WITNESS_ORIGIN` once the config began recording the origin.)
    #[test]
    fn the_witness_origin_names_connected_inference() {
        use crate::config::{WitnessOrigin, WitnessOriginView};
        let line = witness_origin_line(WitnessOriginView::Recorded(
            WitnessOrigin::ConnectedInference,
        ));
        assert!(line.contains("connected inference"));
    }
}
