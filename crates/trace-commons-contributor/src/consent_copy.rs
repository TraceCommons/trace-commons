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
//!
//! # Switch-on notices
//!
//! The two notices the connect-and-forget design puts on the enforcement
//! switch-on list ("When enforcement is switched on", item 5), and the words
//! for the held-count health condition (item 2):
//!
//! - [`arming_reworded_notice_for_wire`] (`tc_arming_reworded_notice`), for
//!   one element of `status.arming_rewordings`: a folder armed under the old
//!   "will be scrubbed" wording whose wording is now patterns-only (K5).
//! - [`gate_held_notice_for_wire`] (`tc_gate_held_notice`), for
//!   `status.automatic_contribution_held`: armed folders whose sessions the
//!   automatic-contribution gate is holding, why, and that they release on
//!   their own.
//!
//! **DRAFT, NEEDS APPROVAL**, every sentence in both: the spec's Open list
//! says the wording is open.
//!
//! # The submit toast and the per-session notification (K9, #1118)
//!
//! Two more WYSIWYG design sentences, matched here so no shell writes its
//! own: [`toast_sent_text`] (`tc_toast_sent_text`), "Sent. N left to decide
//! (middle dot) upload limit X of Y"; and [`session_notification_copy`]
//! (`tc_session_notification_copy`), the per-session notification whose body
//! is [`GATE_STATEMENT`] and whose one action is "Look, then decide". Both
//! **DRAFT, NEEDS APPROVAL**.

/// Core-owned disclosure for configured activity missions with rewards disabled.
pub const ACTIVITY_MISSIONS_DISCLOSURE: &str = "Matching stays on this Mac; no activity profile or match result is sent. Missions change no capture or contribution permissions and send no sessions. Progress uses contributions made through your existing consent. Mission rewards are disabled, and no mission credit is available. Any future mission credit would remain pending and conditional until settlement.";

/// Core-owned disclosure for discovery, separate from daily activity mechanics.
/// Skill awards do not become corpus credit or authorize a contribution.
pub const MISSION_CATALOGUE_DISCLOSURE: &str = "These are published skill-evaluation tasks. Matching stays on this Mac; no activity profile or match result is sent. Viewing or selecting a mission changes no capture or contribution permissions and sends no sessions. Contributions still require your existing consent. Corpus credit remains pending and conditional until settlement. Skill-evaluation awards are separate from corpus credit.";

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

/// The title of the "Automatic" override's
/// void notice (`grant_voids` element of kind `contribution_override`). It
/// names the mode by its one name (`project_copy::CONTRIBUTION_MODE_AUTO_LABEL`).
pub const VOID_OVERRIDE_TITLE: &str = concat!(
    crate::project_copy::folder_mode_auto_label!(),
    " turned off"
);

/// What happened. Held to `sweep_grants`: the
/// override is cleared, so every folder is back on its own setting, and a
/// folder that asks first waits for you again.
pub const VOID_OVERRIDE_BODY: &str = concat!(
    "Settings it was turned on under have since changed, so ",
    crate::project_copy::folder_mode_auto_label!(),
    " is off and each folder is back on its own setting. Sessions from folders that ask first \
     wait for you again."
);

/// How it is turned back on.
pub const VOID_OVERRIDE_REARM: &str = concat!(
    "You can turn ",
    crate::project_copy::folder_mode_auto_label!(),
    " back on from Contribution mode. Doing so agrees to the new settings."
);

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
        // `policy::OVERRIDE_TERMS_UNRECORDED`:
        // only an "Automatic" override saved by a pre-release build.
        "terms-unrecorded" => {
            "It was turned on before this app recorded the settings it was turned on under."
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
        // The "Automatic" override (#1208): the pill is back on each
        // folder's own setting. No button: turning it back on is the pill's
        // own confirmation, not a one-tap re-arm.
        (Some("contribution_override"), _) => {
            let placed = void_notice(None, &reasons);
            Some(VoidNoticeCopy {
                title: VOID_OVERRIDE_TITLE.to_string(),
                body: VOID_OVERRIDE_BODY,
                rearm: VOID_OVERRIDE_REARM,
                rearm_action: None,
                rearm_failed: None,
                ..placed
            })
        }
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

/// Shown under an armed project when its disclosure (K6,
/// `automatic_gate::project_disclosure`) could not be read.
///
/// Approved by Zaki on #1075. It names no wording as a fallback: a shell
/// that cannot read the core's answer shows neither scrub disclosure,
/// rather than guessing one.
pub const AUTO_PROJECT_DISCLOSURE_UNAVAILABLE: &str =
    "What is removed from this project could not be loaded.";

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

// ---------------------------------------------------------------------------
// The Scrub check (K4 of #1118)
// ---------------------------------------------------------------------------
//
// Every constant in this section is DRAFT, NEEDS APPROVAL. Written for the
// Settings row and the held-session row so that no shell writes its own.
//
// "Trust relaxes what may be sent, never what may be said" (the spec's R1):
// the Automatic check counts what the scrubber removed and notices a trimmed
// session. It is not a quality check, and it never says a model looked at
// anything, because on most routes nothing confirms one did.
// `the_scrub_check_copy_claims_no_model_or_quality_check` holds that.

/// **DRAFT, NEEDS APPROVAL.** The Settings row's heading.
pub const SCRUB_CHECK_TITLE: &str = "Scrub check";

/// **DRAFT, NEEDS APPROVAL.** The Automatic choice (`scrub_check:
/// "automatic"`). The default: a daemon where nothing was chosen reports
/// `"automatic"` and holds as this says, so a shell renders it selected.
pub const SCRUB_CHECK_AUTOMATIC_LABEL: &str = "Automatic";

/// **DRAFT, NEEDS APPROVAL.** What Automatic does. Names both second-look
/// reasons and says what the check is not.
pub const SCRUB_CHECK_AUTOMATIC_HELP: &str = "In folders set to share automatically, a session is sent on its own once it has been scrubbed, unless nothing personal was removed from it, something left in it still looks like personal data, or it was trimmed to fit. Those wait for you. This only counts and looks for patterns; it does not check that the scrubbing was right.";

/// **DRAFT, NEEDS APPROVAL.** The Manual choice (`scrub_check: "manual"`).
pub const SCRUB_CHECK_MANUAL_LABEL: &str = "Manual";

/// **DRAFT, NEEDS APPROVAL.** What Manual does.
pub const SCRUB_CHECK_MANUAL_HELP: &str = "Every session waits for you, including in folders set to share automatically. Nothing is sent until you approve it.";

/// **DRAFT, NEEDS APPROVAL.** On a session held under
/// `second-look-review-required`. The particular reason is the row's own
/// `second_look` sentence; this says only that it did not move and will not.
pub const SCRUB_CHECK_HELD: &str =
    "This session was not sent on its own. It waits until you decide.";

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

/// The sentences a contributor reads on the Flow 1 grant screens, with the
/// choice between them made here: `automatic_gate::disclosure` picks the
/// disclosure (R1) and [`automatic_grant_copy`] carries only the scrub
/// wording that answer allows.
///
/// `disclosure(config)` reads configuration only, so it answers
/// `PatternsOnly`, and that is the right answer for a screen shown before
/// the grant. Configuration is not evidence that a model ran: the model-scrub
/// wording is earned only by `automatic_gate::folder_disclosure`, over the
/// certificates of sessions the witness has already redacted, and before the
/// grant there are none. So a shell never reads the `auto_scrub_*` fields to
/// choose, and the model-scrub sentences never reach this screen.
#[must_use]
pub fn automatic_contribution_copy(
    config: Option<&crate::config::ContributorConfig>,
) -> AutomaticGrantCopy {
    automatic_grant_copy(crate::daemon::automatic_gate::disclosure(config))
}

/// The grant screens' words for a disclosure the daemon already chose and
/// reported by name (`list_projects`' `automatic_disclosure`). `None` for a
/// name this build does not know, so a shell shows nothing rather than
/// guessing which wording is true.
#[must_use]
pub fn automatic_grant_copy_named(disclosure: &str) -> Option<AutomaticGrantCopy> {
    use crate::daemon::automatic_gate::Disclosure;
    match disclosure {
        "patterns_only" => Some(automatic_grant_copy(Disclosure::PatternsOnly)),
        "model_scrubbed" => Some(automatic_grant_copy(Disclosure::ModelScrubbed)),
        _ => None,
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
// Moving a legacy invite identity to a NEAR AI account
// ---------------------------------------------------------------------------
//
// Every constant in this section is DRAFT, NEEDS APPROVAL (copy for Zaki's
// approval): written with the client half of the legacy invite migration so
// that no shell writes its own. The Tauri client renders all of it; macOS,
// Windows and GTK render only the notice after a move
// (`legacy_migration_notice_for_wire`), not the offer. The daemon reports
// the move under `status.legacy_invite_migration` and refuses it with
// `legacy_migration_*` labels (`daemon::legacy_migration::LABELS`).

/// **DRAFT, NEEDS APPROVAL.** Heading of the offer, shown only while
/// `status.legacy_invite_migration.offered` is true.
pub const LEGACY_MIGRATION_OFFER_TITLE: &str = "Move to your NEAR AI account";

/// **DRAFT, NEEDS APPROVAL.** The offer. Says it is optional and that
/// declining changes nothing, because coexistence is the default.
pub const LEGACY_MIGRATION_OFFER_BODY: &str = "You joined with an invite. You can move your contributions to your NEAR AI account instead. Nothing changes unless you choose to, and your invite keeps working if you don't.";

/// **DRAFT, NEEDS APPROVAL.** The button that starts the move.
pub const LEGACY_MIGRATION_OFFER_ACTION: &str = "Move to my NEAR AI account";

/// **DRAFT, NEEDS APPROVAL.** Shown while the move runs.
pub const LEGACY_MIGRATION_WORKING: &str = "Moving to your NEAR AI account...";

/// **DRAFT, NEEDS APPROVAL.** Asked only when neither the device nor the
/// commons can say which invite it joined with
/// (`legacy_migration_invite_needed`).
pub const LEGACY_MIGRATION_INVITE_PROMPT: &str = "Paste the invite link you joined with. It is used only to show which invite is yours, and it is not stored.";

/// **DRAFT, NEEDS APPROVAL.** The notice's heading: the one sentence the
/// consent spec requires every shell to show after the move.
pub const LEGACY_MIGRATION_NOTICE_TITLE: &str =
    "Your contributions now go under your NEAR AI account";

/// **DRAFT, NEEDS APPROVAL.**
pub const LEGACY_MIGRATION_NOTICE_BODY: &str = "From now on, what you contribute is credited to your NEAR AI account instead of your invite. What you contributed before stays recorded under your invite.";

/// **DRAFT, NEEDS APPROVAL.** When at least one folder, or the automatic
/// grant, was carried over.
pub const LEGACY_MIGRATION_NOTICE_ARMED_KEPT: &str = "Folders you set to contribute automatically still do. You were not asked again because only the account they go under changed.";

/// **DRAFT, NEEDS APPROVAL.** When nothing was armed.
pub const LEGACY_MIGRATION_NOTICE_NOTHING_ARMED: &str =
    "You had no folders contributing automatically, so nothing else changed.";

/// **DRAFT, NEEDS APPROVAL.**
pub const LEGACY_MIGRATION_NOTICE_ACKNOWLEDGE: &str = "Got it";

/// Shown when the shell could not reach the core to start the move at all
/// (a transport failure, not a refusal): nothing ran, so nothing changed.
pub const LEGACY_MIGRATION_START_FAILED: &str =
    "The move could not be started. Nothing was changed.";

/// The notice after a move, as a shell renders it.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct LegacyMigrationNoticeCopy {
    pub title: &'static str,
    pub body: &'static str,
    pub folders: &'static str,
    pub acknowledge: &'static str,
}

/// The notice for `status.legacy_invite_migration.notice`, or `None` when
/// there is nothing to show (`null`, or not an object).
#[must_use]
pub fn legacy_migration_notice_for_wire(
    notice: &serde_json::Value,
) -> Option<LegacyMigrationNoticeCopy> {
    let object = notice.as_object()?;
    let folders = object
        .get("folders_kept")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let grant = object
        .get("automatic_grant_kept")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some(LegacyMigrationNoticeCopy {
        title: LEGACY_MIGRATION_NOTICE_TITLE,
        body: LEGACY_MIGRATION_NOTICE_BODY,
        folders: if folders > 0 || grant {
            LEGACY_MIGRATION_NOTICE_ARMED_KEPT
        } else {
            LEGACY_MIGRATION_NOTICE_NOTHING_ARMED
        },
        acknowledge: LEGACY_MIGRATION_NOTICE_ACKNOWLEDGE,
    })
}

/// **DRAFT, NEEDS APPROVAL.** What a refused move says, by the daemon's
/// label. Every refusal leaves the invite identity exactly as it was, and
/// the pooled one says so plainly because it is the common case (a shared
/// event code).
#[must_use]
pub fn legacy_migration_refusal_line(label: &str) -> &'static str {
    match label {
        "legacy_migration_tenant_pooled" => {
            "This invite was shared by many people, so it can't be moved to one account. It keeps working exactly as it does now."
        }
        "legacy_migration_admission_not_ready" => {
            "This commons isn't accepting contributions by account yet. Your invite keeps working; try again later."
        }
        "legacy_migration_no_near_ai_session" => {
            "Sign in to NEAR AI first, then try again. Your invite keeps working."
        }
        "legacy_migration_invite_needed" => {
            "We couldn't tell which invite you joined with. Paste your invite link to continue."
        }
        "legacy_migration_invite_other_commons" => "That invite is for a different commons.",
        "legacy_migration_invite_invalid" => "That doesn't look like an invite link or code.",
        "legacy_migration_tenant_claimed" => {
            "This invite has already been moved to a different account. Nothing changed here."
        }
        // Copy for Zaki's approval (V104): another of this person's devices
        // moved the same invite tenant onto this account under a different
        // invite code, which was never granted to the account.
        "legacy_migration_invite_not_linked" => {
            "This device joined with a different invite than the one already moved to your account, so it can't be moved. It keeps working as it does now."
        }
        "legacy_migration_invite_revoked" => {
            "This invite was revoked, so it can't be moved. Nothing changed here."
        }
        "legacy_migration_device_not_eligible" => {
            "This device can't be moved to an account. It keeps working as it does now."
        }
        "legacy_migration_link_not_enabled" => {
            "This commons doesn't offer moving an invite to an account yet. Your invite keeps working."
        }
        "legacy_migration_verification_failed" => {
            "The commons's answer didn't check out, so nothing was changed."
        }
        "legacy_migration_identity_changed" => {
            "You signed out or changed accounts while this was running, so nothing was moved."
        }
        "legacy_migration_commons_changed" => {
            "This commons's settings changed since you joined, so nothing was moved. Try again later."
        }
        "legacy_migration_already_migrated" => "You've already moved to your NEAR AI account.",
        _ => "Nothing was changed. Your invite keeps working; try again later.",
    }
}

/// The offer's words, in one object, for a shell to render.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct LegacyMigrationOfferCopy {
    pub title: &'static str,
    pub body: &'static str,
    pub action: &'static str,
    pub working: &'static str,
    pub invite_prompt: &'static str,
    pub start_failed: &'static str,
}

#[must_use]
pub fn legacy_migration_offer() -> LegacyMigrationOfferCopy {
    LegacyMigrationOfferCopy {
        title: LEGACY_MIGRATION_OFFER_TITLE,
        body: LEGACY_MIGRATION_OFFER_BODY,
        action: LEGACY_MIGRATION_OFFER_ACTION,
        working: LEGACY_MIGRATION_WORKING,
        invite_prompt: LEGACY_MIGRATION_INVITE_PROMPT,
        start_failed: LEGACY_MIGRATION_START_FAILED,
    }
}

// ---------------------------------------------------------------------------
// Switch-on notices: the old-wording notice (K5) and the held-folder notice
// ---------------------------------------------------------------------------
//
// DRAFT, NEEDS APPROVAL: every constant below. The spec's Open list
// ("Telling the contributor: the copy and the shells") leaves the wording
// open.

/// Why a folder armed under the old wording is being told anything.
///
/// **DRAFT, NEEDS APPROVAL.** States the change and that the mode did not
/// change -- "already-armed folders stay armed" -- before what the arming now
/// means, which is the patterns-only disclosure word for word.
pub const REWORDED_BODY: &str = "When you turned on automatic contributing here, we said its sessions would be scrubbed. That said more than this app can confirm, so this is what it means now. Nothing about the project has changed: it still contributes automatically.";

/// The heading over the patterns-only sentences in a rewording notice.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const REWORDED_NOW_HEADING: &str = "What happens to its sessions";

/// The button that switches a reworded folder to ask-first. A shell sends it
/// as `set_project_mode` with the element's `project_id` and `notify_only`,
/// which also answers the notice. It names the mode it sets, by the mode's
/// one name (`project_copy::CONTRIBUTION_MODE_ASK_LABEL`).
///
/// **DRAFT, NEEDS APPROVAL.**
pub const ASK_ME_FIRST_ACTION: &str = crate::project_copy::CONTRIBUTION_MODE_ASK_LABEL;

/// Shown when the daemon refuses that switch. The notice stays.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const ASK_ME_FIRST_FAILED: &str =
    "It could not be switched, so it still contributes automatically. Nothing was changed.";

/// The title of a rewording notice this build cannot place: no label.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const REWORDED_UNPLACED_TITLE: &str = "What automatic contributing now means";

/// The title of a rewording notice.
#[must_use]
pub fn arming_reworded_title(project_label: Option<&str>) -> String {
    match project_label {
        Some(label) => format!("What automatic contributing from {label} now means"),
        None => REWORDED_UNPLACED_TITLE.to_string(),
    }
}

/// Everything a shell shows for one element of `status.arming_rewordings`.
///
/// The scope and limit are [`AUTO_PATTERNS_ONLY_SCOPE`] and
/// [`AUTO_PATTERNS_ONLY_LIMIT`], word for word: the spec says a reworded
/// folder "gets the same notice a newly automatic folder gets", and the
/// patterns-only wording is that notice.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ArmingRewordedNoticeCopy {
    pub title: String,
    pub body: &'static str,
    pub now_heading: &'static str,
    pub scope: &'static str,
    pub limit: &'static str,
    pub no_review: &'static str,
    pub acknowledge: &'static str,
    /// Present exactly when the element names a `project_id` to act on.
    pub ask_first_action: Option<&'static str>,
    /// Present exactly when `ask_first_action` is.
    pub ask_first_failed: Option<&'static str>,
}

/// The notice for one element of `status.arming_rewordings`, as it came off
/// the wire. `None` only for a value that is not an object.
///
/// The daemon also uses this persisted/acknowledged channel for the
/// Automatic-default upgrade. Shells pass the whole object through, so
/// both notices reach every shell without shell-authored consent copy.
#[must_use]
pub fn arming_reworded_notice_for_wire(
    value: &serde_json::Value,
) -> Option<ArmingRewordedNoticeCopy> {
    let object = value.as_object()?;
    let label = object
        .get("project_label")
        .and_then(serde_json::Value::as_str)
        .filter(|l| !l.is_empty());
    let has_id = object
        .get("project_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| !id.is_empty());
    if object
        .get("scrub_check_defaulted")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Some(ArmingRewordedNoticeCopy {
            title: label.map_or_else(
                || "The Scrub check is now Automatic".to_string(),
                |label| format!("The Scrub check is now Automatic for {label}"),
            ),
            body: "Your Scrub check was previously unset. This update makes it Automatic. This folder stays set to share automatically, but more sessions may now wait for your review.",
            now_heading: "What happens now",
            scope: SCRUB_CHECK_AUTOMATIC_HELP,
            limit: concat!(
                "Choose ",
                crate::project_copy::folder_mode_ask_label!(),
                " for this folder if you want to review every session from it."
            ),
            no_review: "Held sessions are not sent until you decide.",
            acknowledge: VOID_ACKNOWLEDGE,
            ask_first_action: has_id.then_some(ASK_ME_FIRST_ACTION),
            ask_first_failed: has_id.then_some(ASK_ME_FIRST_FAILED),
        });
    }
    Some(ArmingRewordedNoticeCopy {
        title: arming_reworded_title(label),
        body: REWORDED_BODY,
        now_heading: REWORDED_NOW_HEADING,
        scope: AUTO_PATTERNS_ONLY_SCOPE,
        limit: AUTO_PATTERNS_ONLY_LIMIT,
        no_review: AUTO_NO_REVIEW,
        acknowledge: VOID_ACKNOWLEDGE,
        ask_first_action: has_id.then_some(ASK_ME_FIRST_ACTION),
        ask_first_failed: has_id.then_some(ASK_ME_FIRST_FAILED),
    })
}

/// The title of the held-folder notice.
///
/// **DRAFT, NEEDS APPROVAL.**
pub const GATE_HELD_TITLE: &str = "Automatic contributing is on hold";

/// That nothing is sent while held, that nothing is lost, and that it
/// releases on its own.
///
/// **DRAFT, NEEDS APPROVAL.** "On their own" is what separates this hold from
/// the indefinite one rev 6 rejected: it releases the first full pass that
/// finds the requirement met.
pub const GATE_HELD_RELEASE: &str = "Nothing from these projects is sent while they wait, and nothing has been lost. They go out on their own once this changes.";

/// What the contributor can do meanwhile. Shown beside a folder's
/// [`ASK_ME_FIRST_ACTION`].
///
/// **DRAFT, NEEDS APPROVAL.**
pub const GATE_HELD_ASK_FIRST: &str = concat!(
    "To review a project's sessions yourself instead, switch it to ",
    crate::project_copy::folder_mode_ask_label!(),
    "."
);

/// One of the gate's reason labels (`automatic_gate::REASON_*`), as a
/// sentence. A label this build does not know still gets one.
///
/// **DRAFT, NEEDS APPROVAL.** The R3 sentence is the spec's: "this commons
/// does not yet accept automatic contributions from their account".
#[must_use]
pub fn gate_held_reason_line(label: &str) -> &'static str {
    use crate::daemon::automatic_gate::{
        REASON_ACCOUNT_ALLOWANCE_SPENT, REASON_ADMISSION_PER_SESSION, REASON_NO_SCOPE,
    };
    match label {
        REASON_ADMISSION_PER_SESSION => {
            "This commons does not yet accept automatic contributions from your account."
        }
        REASON_ACCOUNT_ALLOWANCE_SPENT => {
            "Your account has reached what this commons accepts from it for now."
        }
        REASON_NO_SCOPE => "You have not chosen how your traces may be used.",
        _ => "Something automatic contributing needs is not in place yet.",
    }
}

/// How many sessions are held, counted and agreeing in number.
#[must_use]
pub fn gate_held_count_line(held_sessions: u64) -> String {
    match held_sessions {
        0 => "Sessions from projects set to contribute automatically are waiting.".to_string(),
        1 => "1 session from a project set to contribute automatically is waiting.".to_string(),
        n => format!("{n} sessions from projects set to contribute automatically are waiting."),
    }
}

/// One held folder's line.
#[must_use]
pub fn gate_held_project_line(project_label: &str, held_sessions: u64) -> String {
    match held_sessions {
        1 => format!("{project_label}: 1 session waiting"),
        n => format!("{project_label}: {n} sessions waiting"),
    }
}

/// One held folder in [`GateHeldNoticeCopy`].
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct GateHeldProjectCopy {
    /// Passed through from the wire, for the ask-first button's
    /// `set_project_mode`. Never shown.
    pub project_id: Option<String>,
    pub line: String,
    /// Present exactly when `project_id` is.
    pub ask_first_action: Option<&'static str>,
    pub ask_first_failed: Option<&'static str>,
}

/// Everything a shell shows while the gate holds armed folders.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct GateHeldNoticeCopy {
    pub title: &'static str,
    pub body: String,
    /// One sentence per reason label, deduplicated, never empty.
    pub reasons: Vec<&'static str>,
    pub release: &'static str,
    pub ask_first: &'static str,
    pub projects: Vec<GateHeldProjectCopy>,
}

/// The held-folder notice from `status.automatic_contribution_held`, passed
/// through as the object the daemon sent. `None` when nothing is held, or
/// for a value that is not that object: there is nothing to show, and it is
/// never acknowledged -- it goes when the hold does.
#[must_use]
pub fn gate_held_notice_for_wire(value: &serde_json::Value) -> Option<GateHeldNoticeCopy> {
    let held = value.get("held_sessions")?.as_u64()?;
    if held == 0 {
        return None;
    }
    let mut reasons: Vec<&'static str> = Vec::new();
    for label in value
        .get("reasons")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        let line = gate_held_reason_line(label);
        if !reasons.contains(&line) {
            reasons.push(line);
        }
    }
    if reasons.is_empty() {
        reasons.push(gate_held_reason_line(""));
    }
    let projects = value
        .get("projects")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let label = p
                .get("project_label")
                .and_then(serde_json::Value::as_str)
                .filter(|l| !l.is_empty())?;
            let count = p
                .get("held_sessions")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let project_id = p
                .get("project_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string);
            let action = project_id.is_some();
            Some(GateHeldProjectCopy {
                project_id,
                line: gate_held_project_line(label, count),
                ask_first_action: action.then_some(ASK_ME_FIRST_ACTION),
                ask_first_failed: action.then_some(ASK_ME_FIRST_FAILED),
            })
        })
        .collect();
    Some(GateHeldNoticeCopy {
        title: GATE_HELD_TITLE,
        body: gate_held_count_line(held),
        reasons,
        release: GATE_HELD_RELEASE,
        ask_first: GATE_HELD_ASK_FIRST,
        projects,
    })
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

/// The daemon's `route_disclosure` answer, as sent, paired with the words for
/// it: `{"facts": .., "copy": ..}`. The entry point for a shell that words the
/// daemon's answer itself: through the C ABI's `tc_route_disclosure_copy`, or
/// by calling this directly from a Rust shell.
///
/// `None` for anything this build cannot read -- an unknown `route` or
/// `origin` is a newer daemon's answer, and the nearest known value would
/// claim something the daemon did not say. A shell then shows
/// [`disclosure_unreadable_copy`]'s words, never a disclosure of its own.
#[must_use]
pub fn route_disclosure_for_wire(value: &serde_json::Value) -> Option<serde_json::Value> {
    let facts: crate::disclosure::RouteDisclosure = serde_json::from_value(value.clone()).ok()?;
    let copy = route_disclosure_copy(&facts);
    Some(serde_json::json!({ "facts": facts, "copy": copy }))
}

/// What a disclosure surface says when the daemon's answer could not be read
/// -- an older daemon without `route_disclosure`, or a newer route or origin
/// this build cannot word. Said rather than left blank, so a missing panel is
/// never mistaken for nothing to disclose.
///
/// Approved by Zaki with #1102.
pub const DISCLOSURE_UNREADABLE: &str = "Where sessions go could not be read.";
/// The same, on a single session's review.
///
/// Approved by Zaki with #1102.
pub const DISCLOSURE_SESSION_UNREADABLE: &str = "Where this session goes could not be read.";

/// [`DISCLOSURE_UNREADABLE`] and [`DISCLOSURE_SESSION_UNREADABLE`], with the
/// section's [`DISCLOSURE_TITLE`] so an unreadable panel is still named, for
/// a shell that cannot hold them as constants (the C ABI's
/// `tc_route_disclosure_unreadable_copy`, Tauri's
/// `route_disclosure_unreadable_copy`).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct DisclosureUnreadableCopy {
    pub title: &'static str,
    pub panel: &'static str,
    pub session: &'static str,
}

#[must_use]
pub fn disclosure_unreadable_copy() -> DisclosureUnreadableCopy {
    DisclosureUnreadableCopy {
        title: DISCLOSURE_TITLE,
        panel: DISCLOSURE_UNREADABLE,
        session: DISCLOSURE_SESSION_UNREADABLE,
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

// ---------------------------------------------------------------------------
// "Leaves this Mac" (Flow 2 review sheet, #1118 K3)
//
// The sheet in the design says, under "Leaves this Mac", "19 KB · 12 turns ·
// tool, project label, timing, outcome. Never the path." That is a claim
// about what the envelope carries, made at the instant of consent, so it is
// derived here from the envelope's own serialized keys rather than written
// by each shell -- and derived, it does NOT say "project label": the
// envelope has carried no project name, in the clear or hashed, since #207.
//
// The list is COMPLETE by construction: [`leaves_this_mac_fields`] walks every
// key of the serialized envelope and classifies it, and a key it has no
// label for is reported as [`LEAVES_OTHER`] rather than dropped, so a field
// added to the protocol later is shown as "other metadata" until somebody
// names it. `every_envelope_key_has_a_named_label` fails the build of that
// field instead.
//
// Every sentence in this section is DRAFT, NEEDS APPROVAL.

/// Wire labels for [`leaves_this_mac_fields`]. Closed: each has exactly one
/// phrase in [`leaves_this_mac_phrase`], and [`LEAVES_FIELDS`] is the order
/// they are listed in.
pub const LEAVES_CONVERSATION: &str = "conversation";
pub const LEAVES_TOOL: &str = "tool";
pub const LEAVES_TOOL_VERSION: &str = "tool-version";
pub const LEAVES_MODEL: &str = "model";
pub const LEAVES_TIMING: &str = "timing";
pub const LEAVES_USAGE_AND_COST: &str = "usage-and-cost";
pub const LEAVES_ROUTING: &str = "routing";
pub const LEAVES_OUTCOME: &str = "outcome";
pub const LEAVES_CORRECTION: &str = "correction";
pub const LEAVES_USES: &str = "uses";
pub const LEAVES_REDACTION_SUMMARY: &str = "redaction-summary";
pub const LEAVES_SESSION_ID: &str = "session-id";
pub const LEAVES_TRACE_IDS: &str = "trace-ids";
pub const LEAVES_CONTRIBUTOR_ID: &str = "contributor-id";
pub const LEAVES_TENANT: &str = "tenant";
pub const LEAVES_CREDIT_ACCOUNT: &str = "credit-account";
pub const LEAVES_REVOCATION_HANDLE: &str = "revocation-handle";
pub const LEAVES_FOLDER_FINGERPRINT: &str = "folder-fingerprint";
pub const LEAVES_REPLAY: &str = "replay";
pub const LEAVES_SCORES: &str = "scores";
pub const LEAVES_FORMAT_VERSION: &str = "format-version";
/// A key this build has no name for. Reported, never dropped.
pub const LEAVES_OTHER: &str = "other";

/// Every label [`leaves_this_mac_fields`] can return, in list order.
pub const LEAVES_FIELDS: &[&str] = &[
    LEAVES_CONVERSATION,
    LEAVES_TOOL,
    LEAVES_TOOL_VERSION,
    LEAVES_MODEL,
    LEAVES_TIMING,
    LEAVES_USAGE_AND_COST,
    LEAVES_ROUTING,
    LEAVES_OUTCOME,
    LEAVES_CORRECTION,
    LEAVES_USES,
    LEAVES_REDACTION_SUMMARY,
    LEAVES_SESSION_ID,
    LEAVES_TRACE_IDS,
    LEAVES_CONTRIBUTOR_ID,
    LEAVES_TENANT,
    LEAVES_CREDIT_ACCOUNT,
    LEAVES_REVOCATION_HANDLE,
    LEAVES_FOLDER_FINGERPRINT,
    LEAVES_REPLAY,
    LEAVES_SCORES,
    LEAVES_FORMAT_VERSION,
    LEAVES_OTHER,
];

/// **DRAFT, NEEDS APPROVAL.** What the metadata never carries. Scoped to
/// the metadata on purpose: only absolute paths are scrubbed out of the
/// conversation, so a relative path or a sentence can still name the folder
/// there -- see [`LEAVES_FOLDER_IN_CONVERSATION`].
pub const LEAVES_METADATA_NEVER: &str = "The metadata never carries the path or the folder name.";

/// **DRAFT, NEEDS APPROVAL.** Added when the conversation itself names the
/// folder, for instance through `../myproj/src/main.rs`.
pub const LEAVES_FOLDER_IN_CONVERSATION: &str = "The conversation itself names the folder.";

/// **DRAFT, NEEDS APPROVAL.** Replaces [`LEAVES_METADATA_NEVER`] in the
/// (not expected) case that the folder's name is found outside the
/// conversation too: the sentence is only ever said when it is true.
pub const LEAVES_FOLDER_IN_METADATA: &str = "The folder's name appears in what would be sent.";

/// Where a key sits in the envelope: a label that covers everything under
/// it, a node to look inside, or a key with no name.
enum Class {
    Label(&'static str),
    Descend,
    Unknown,
}

/// The label for one serialized key path (array indices are not part of a
/// path). One table, read by both the live list and the completeness test.
fn classify(path: &[&str]) -> Class {
    use Class::*;
    match path {
        [] => Descend,
        ["schema_version"] => Label(LEAVES_FORMAT_VERSION),
        ["trace_id"] | ["submission_id"] => Label(LEAVES_TRACE_IDS),
        ["created_at"] => Label(LEAVES_TIMING),
        ["ironclaw"] | ["ironclaw", "feature_flags"] => Descend,
        ["ironclaw", "version"] | ["ironclaw", "engine_version"] => Label(LEAVES_TOOL_VERSION),
        ["ironclaw", "channel"] => Label(LEAVES_TOOL),
        ["ironclaw", "model_name"] => Label(LEAVES_MODEL),
        ["ironclaw", "feature_flags", "agent"] => Label(LEAVES_TOOL),
        ["ironclaw", "feature_flags", "agent_version"] => Label(LEAVES_TOOL_VERSION),
        ["ironclaw", "feature_flags", "cwd_hash"] => Label(LEAVES_FOLDER_FINGERPRINT),
        ["consent", ..] | ["trace_card", ..] => Label(LEAVES_USES),
        ["contributor"] => Descend,
        ["contributor", "pseudonymous_contributor_id"] => Label(LEAVES_CONTRIBUTOR_ID),
        ["contributor", "tenant_scope_ref"] => Label(LEAVES_TENANT),
        ["contributor", "credit_account_ref"] => Label(LEAVES_CREDIT_ACCOUNT),
        ["contributor", "revocation_handle"] => Label(LEAVES_REVOCATION_HANDLE),
        ["privacy", ..] => Label(LEAVES_REDACTION_SUMMARY),
        ["events"] => Descend,
        ["events", "timestamp"] | ["events", "latency_ms"] => Label(LEAVES_TIMING),
        ["events", "token_counts"] | ["events", "cost_usd"] => Label(LEAVES_USAGE_AND_COST),
        ["events", "tool_name"] | ["events", "tool_category"] => Label(LEAVES_TOOL),
        [
            "events",
            "event_id" | "parent_event_id" | "event_type" | "redacted_content"
            | "structured_payload" | "tool_call_id" | "success" | "failure_modes" | "side_effect",
        ] => Label(LEAVES_CONVERSATION),
        ["outcome"] => Descend,
        ["outcome", "human_correction"] => Label(LEAVES_CORRECTION),
        [
            "outcome",
            "user_feedback" | "task_success" | "error_taxonomy" | "failure_modes",
        ] => Label(LEAVES_OUTCOME),
        ["replay", ..] => Label(LEAVES_REPLAY),
        ["conversation_id"] | ["source_session", ..] => Label(LEAVES_SESSION_ID),
        ["value", ..]
        | ["value_card", ..]
        | ["embedding_analysis", ..]
        | ["hindsight", ..]
        | ["training_dynamics", ..]
        | ["process_evaluation", ..] => Label(LEAVES_SCORES),
        _ => Unknown,
    }
}

fn walk(
    value: &serde_json::Value,
    path: &mut Vec<String>,
    found: &mut std::collections::BTreeSet<&'static str>,
    unknown: &mut Vec<String>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, path, found, unknown);
            }
        }
        serde_json::Value::Object(map) => {
            // A routing row is an event like any other, and it is also the
            // one place routing leaves: name it.
            if path.len() == 1
                && path[0] == "events"
                && map.get("event_type").and_then(|t| t.as_str()) == Some("routing_decision")
            {
                found.insert(LEAVES_ROUTING);
            }
            for (key, child) in map {
                if child.is_null() {
                    continue;
                }
                path.push(key.clone());
                let refs: Vec<&str> = path.iter().map(String::as_str).collect();
                match classify(&refs) {
                    Class::Label(label) => {
                        found.insert(label);
                    }
                    Class::Descend => walk(child, path, found, unknown),
                    Class::Unknown => {
                        found.insert(LEAVES_OTHER);
                        unknown.push(refs.join("."));
                    }
                }
                path.pop();
            }
        }
        _ => {}
    }
}

/// Every labelled key path of `envelope`'s serialized form, plus the paths
/// no label covers. The second list is empty for every envelope this build
/// produces; `every_envelope_key_has_a_named_label` holds it to that.
fn classify_envelope(
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
) -> (Vec<&'static str>, Vec<String>) {
    let value = serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null);
    let mut found = std::collections::BTreeSet::new();
    let mut unknown = Vec::new();
    walk(&value, &mut Vec::new(), &mut found, &mut unknown);
    // A serialization failure must not read as "nothing leaves".
    if value.is_null() {
        found.insert(LEAVES_OTHER);
    }
    let ordered = LEAVES_FIELDS
        .iter()
        .copied()
        .filter(|label| found.contains(label))
        .collect();
    (ordered, unknown)
}

/// What leaves this machine in `envelope`, as labels from [`LEAVES_FIELDS`],
/// in that order: every serialized key, classified. A key with no name is
/// [`LEAVES_OTHER`], so the list can over-describe but never omit.
pub fn leaves_this_mac_fields(
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
) -> Vec<&'static str> {
    classify_envelope(envelope).0
}

/// Whether `text` names `folder`: a case-insensitive match that stands alone
/// (not glued to a letter or digit on either side), so `api` is found in
/// `../api/src` but not in `rapid`. An empty folder name names nothing.
pub fn names_folder(text: &str, folder: &str) -> bool {
    let folder = folder.trim().to_lowercase();
    if folder.is_empty() {
        return false;
    }
    let text = text.to_lowercase();
    let bytes = text.as_bytes();
    let glued = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    text.match_indices(&folder).any(|(at, m)| {
        !glued(at.checked_sub(1).and_then(|i| bytes.get(i))) && !glued(bytes.get(at + m.len()))
    })
}

/// Where the folder's name appears in `envelope`, for any of `folders`
/// (the project's basenames): `(in_metadata, in_conversation)`, where the
/// conversation is `events` and the metadata is everything else.
pub fn folder_named_in(
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
    folders: &[&str],
) -> (bool, bool) {
    let conversation = serde_json::to_string(&envelope.events).unwrap_or_default();
    let mut metadata_only = envelope.clone();
    metadata_only.events.clear();
    let metadata = serde_json::to_string(&metadata_only).unwrap_or_default();
    let any = |text: &str| folders.iter().any(|f| names_folder(text, f));
    (any(&metadata), any(&conversation))
}

/// **DRAFT, NEEDS APPROVAL.** The phrase for one [`LEAVES_FIELDS`] label,
/// or `None` for a label this build does not know.
pub fn leaves_this_mac_phrase(label: &str) -> Option<&'static str> {
    Some(match label {
        LEAVES_CONVERSATION => "the scrubbed conversation",
        LEAVES_TOOL => "tool",
        LEAVES_TOOL_VERSION => "tool version",
        LEAVES_MODEL => "model",
        LEAVES_TIMING => "timing",
        LEAVES_USAGE_AND_COST => "token counts and cost",
        LEAVES_ROUTING => "which route each call took",
        LEAVES_OUTCOME => "outcome",
        LEAVES_CORRECTION => "your correction",
        LEAVES_USES => "the uses you allowed",
        LEAVES_REDACTION_SUMMARY => "what scrubbing removed, as counts",
        LEAVES_SESSION_ID => "the tool's own session id",
        LEAVES_TRACE_IDS => "random ids for this trace",
        LEAVES_CONTRIBUTOR_ID => "a pseudonymous contributor id",
        LEAVES_TENANT => "the commons you joined",
        LEAVES_CREDIT_ACCOUNT => "your credit account reference",
        LEAVES_REVOCATION_HANDLE => "a handle for taking it back",
        LEAVES_FOLDER_FINGERPRINT => "a one-way fingerprint of the folder",
        LEAVES_REPLAY => "the tools a replay would need",
        LEAVES_SCORES => "a value estimate",
        LEAVES_FORMAT_VERSION => "the format version",
        LEAVES_OTHER => "other metadata this version has no name for",
        _ => return None,
    })
}

/// A byte count as the sheet prints it: `812 bytes`, `19 KB`, `1.5 MB`.
pub fn leaves_this_mac_size(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{} KB", (bytes + KB / 2) / KB)
    } else if bytes == 1 {
        "1 byte".to_string()
    } else {
        format!("{bytes} bytes")
    }
}

/// **DRAFT, NEEDS APPROVAL.** The whole "Leaves this Mac" line, for
/// example `19 KB · 12 turns · tool, model, timing, …. The metadata never
/// carries the path or the folder name.`
///
/// `would_send_bytes` is the envelope's size (`preview`'s figure, the one
/// that governs consent), `turn_count` is `preview_turns`' count, `fields`
/// is [`leaves_this_mac_fields`], and `folder_named` is [`folder_named_in`].
/// The conversation label is carried by the turn count rather than repeated,
/// and an unknown label is skipped rather than printed raw.
pub fn leaves_this_mac_line(
    would_send_bytes: usize,
    turn_count: usize,
    fields: &[&str],
    folder_named: (bool, bool),
) -> String {
    let turns = if turn_count == 1 {
        "1 turn".to_string()
    } else {
        format!("{turn_count} turns")
    };
    let phrases: Vec<&str> = fields
        .iter()
        .filter(|f| **f != LEAVES_CONVERSATION)
        .filter_map(|f| leaves_this_mac_phrase(f))
        .collect();
    let (in_metadata, in_conversation) = folder_named;
    let mut close = vec![if in_metadata {
        LEAVES_FOLDER_IN_METADATA
    } else {
        LEAVES_METADATA_NEVER
    }];
    if in_conversation && !in_metadata {
        close.push(LEAVES_FOLDER_IN_CONVERSATION);
    }
    format!(
        "{} \u{00b7} {turns} \u{00b7} {}. {}",
        leaves_this_mac_size(would_send_bytes),
        phrases.join(", "),
        close.join(" ")
    )
}

/// Test access to the unknown-key list, for the completeness test that lives
/// beside a real envelope build.
#[cfg(test)]
pub(crate) fn unlabelled_envelope_keys(
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
) -> Vec<String> {
    classify_envelope(envelope).1
}

// ---------------------------------------------------------------------------
// The submit toast (K9, #1118): "Sent. N left to decide - upload limit X of Y"
// ---------------------------------------------------------------------------
//
// Not the same sentence as the GTK/macOS/Windows "one-click submit" toast
// (`crate::daemon::queue`'s redaction-count toast, e.g. "Approved. 4
// redactions applied. 1 flagged."). This is a second, later line from the
// WYSIWYG design's Flow 2 and Flow 3: after a submit, how many decisions are
// still owed and where today's upload cap stands. It lives here, not in a
// shell, for the reason the module doc gives -- one Rust sentence rather
// than three native copies of the same arithmetic.

/// **DRAFT, NEEDS APPROVAL.** The toast after a submit, matching the design's
/// worked example exactly: "Sent. 1 left to decide - upload limit 7 of 20".
///
/// `uploads_today` and `max_uploads_per_day` are `status.daily_budget`'s
/// fields of the same names. `decisions_owed` is `status.decisions_owed`
/// (K6, a separate branch): this function only formats the count it is
/// given and never reads the queue itself, so it has nothing to say about
/// what counts as "owed" -- that is K6's decision, not this one's.
#[must_use]
pub fn toast_sent_text(
    uploads_today: u64,
    max_uploads_per_day: u64,
    decisions_owed: u64,
) -> String {
    // `\u{b7}` is the design's middle dot (·), written as an escape so the
    // source stays plain ASCII like the rest of this file's comments.
    format!(
        "Sent. {decisions_owed} left to decide \u{b7} upload limit {uploads_today} of {max_uploads_per_day}"
    )
}

// ---------------------------------------------------------------------------
// The per-session notification (K9, #1118)
// ---------------------------------------------------------------------------

/// **DRAFT, NEEDS APPROVAL.** The one action on a per-session notification
/// (the design's Flow 2, "Notification. Body is the consent sentence; one
/// action, 'Look, then decide'"). Named for what it is, not what it does:
/// the notification offers no way to decide without looking, because the
/// review sheet is where [`GATE_STATEMENT`] and the verdict live, and a
/// second action here would be a second, undisclosed way to approve.
pub const NOTIFICATION_LOOK_THEN_DECIDE_ACTION: &str = "Look, then decide";

/// A per-session notification's words: the body and its one action.
///
/// The body is [`GATE_STATEMENT`] itself, not a paraphrase of it -- the
/// design calls for "the consent sentence", and that is the one this crate
/// already ships above an irreversible Submit. A notification that said
/// something adjacent would be a second, drifting copy of the one sentence
/// this module exists to keep singular.
#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
pub struct SessionNotificationCopy {
    pub body: &'static str,
    pub action: &'static str,
}

/// The per-session notification's copy. Takes no argument: like
/// [`consent_copy`], it describes the build rather than a running daemon, so
/// there is nothing to look up.
#[must_use]
pub fn session_notification_copy() -> SessionNotificationCopy {
    SessionNotificationCopy {
        body: GATE_STATEMENT,
        action: NOTIFICATION_LOOK_THEN_DECIDE_ACTION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leaves_this_mac_line_is_assembled_from_labels() {
        let line = leaves_this_mac_line(
            19 * 1024,
            12,
            &[
                LEAVES_CONVERSATION,
                LEAVES_TOOL,
                LEAVES_TIMING,
                LEAVES_OUTCOME,
                "not-a-field",
            ],
            (false, false),
        );
        assert_eq!(
            line,
            "19 KB \u{00b7} 12 turns \u{00b7} tool, timing, outcome. The metadata never carries the path or the folder name."
        );
        assert_eq!(leaves_this_mac_size(812), "812 bytes");
        assert_eq!(leaves_this_mac_size(3 * 1024 * 1024 / 2), "1.5 MB");
        assert!(leaves_this_mac_line(10, 1, &[], (false, false)).contains("1 turn \u{00b7}"));
        for label in LEAVES_FIELDS {
            assert!(leaves_this_mac_phrase(label).is_some(), "{label}");
        }
    }

    #[test]
    fn a_folder_named_in_the_conversation_is_said_and_never_denied() {
        let named = leaves_this_mac_line(10, 1, &[LEAVES_TOOL], (false, true));
        assert!(named.ends_with(LEAVES_FOLDER_IN_CONVERSATION), "{named}");
        let leaked = leaves_this_mac_line(10, 1, &[LEAVES_TOOL], (true, true));
        assert!(!leaked.contains(LEAVES_METADATA_NEVER), "{leaked}");
        assert!(leaked.ends_with(LEAVES_FOLDER_IN_METADATA), "{leaked}");
    }

    #[test]
    fn a_folder_name_is_matched_standing_alone_and_case_insensitively() {
        assert!(names_folder("cat ../myproj/src/main.rs", "myproj"));
        assert!(names_folder("cd MyProj && ls", "myproj"));
        assert!(names_folder("in api/", "api"));
        assert!(!names_folder("rapid progress", "api"));
        assert!(!names_folder("anything", ""));
    }

    /// K5: the rewording notice says the folder is still armed and then
    /// says exactly what a patterns-only folder is told, with no model-scrub
    /// sentence anywhere.
    #[test]
    fn the_rewording_notice_is_the_patterns_only_disclosure() {
        let wire = serde_json::json!({
            "id": 3, "project_id": "p-1", "project_label": "api",
            "was": "model_scrubbed", "now": "patterns_only",
        });
        let n = arming_reworded_notice_for_wire(&wire).expect("a notice");
        assert_eq!(n.title, "What automatic contributing from api now means");
        assert!(n.body.contains("still contributes automatically"));
        assert_eq!(n.scope, AUTO_PATTERNS_ONLY_SCOPE);
        assert_eq!(n.limit, AUTO_PATTERNS_ONLY_LIMIT);
        assert_eq!(n.no_review, AUTO_NO_REVIEW);
        assert_eq!(n.ask_first_action, Some(ASK_ME_FIRST_ACTION));
        assert_eq!(n.ask_first_failed, Some(ASK_ME_FIRST_FAILED));
        let json = serde_json::to_string(&n).unwrap();
        assert!(!json.contains("when a model recognises it"));
        assert!(!json.contains("The model is not reliable"));
    }

    /// No label: an unplaced title, not a shell-written one. No id: no
    /// button. Not an object: nothing.
    #[test]
    fn a_rewording_the_shell_cannot_place_still_gets_words() {
        let n = arming_reworded_notice_for_wire(&serde_json::json!({ "id": 1 })).unwrap();
        assert_eq!(n.title, REWORDED_UNPLACED_TITLE);
        assert!(n.ask_first_action.is_none() && n.ask_first_failed.is_none());
        assert!(arming_reworded_notice_for_wire(&serde_json::json!("x")).is_none());
    }

    /// #1208: the "Automatic" override's void gets its own words --
    /// not the Flow 1 grant's, not the unplaced fallback -- and no button,
    /// since turning it back on is the pill's own confirmation.
    #[test]
    fn an_override_void_gets_its_own_notice() {
        let n = void_notice_for_wire(&serde_json::json!({
            "id": 3, "kind": "contribution_override", "project_id": null,
            "project_label": null, "reasons": ["scopes-widened"]
        }))
        .unwrap();
        assert_eq!(n.title, VOID_OVERRIDE_TITLE);
        assert_eq!(n.body, VOID_OVERRIDE_BODY);
        assert_eq!(n.rearm, VOID_OVERRIDE_REARM);
        assert_eq!(n.reasons, vec![void_reason_line("scopes-widened")]);
        assert!(n.rearm_action.is_none() && n.rearm_failed.is_none());
        assert_ne!(void_reason_line("terms-unrecorded"), VOID_REASON_UNKNOWN);
    }

    #[test]
    fn an_automatic_default_upgrade_explains_the_new_hold() {
        let copy = arming_reworded_notice_for_wire(&serde_json::json!({
            "id": 1, "project_id": "p-1", "project_label": "api",
            "scrub_check_defaulted": true
        }))
        .unwrap();
        assert_eq!(copy.title, "The Scrub check is now Automatic for api");
        assert!(copy.body.contains("previously unset"));
        assert_eq!(copy.scope, SCRUB_CHECK_AUTOMATIC_HELP);
        assert_eq!(
            copy.no_review,
            "Held sessions are not sent until you decide."
        );
        assert!(copy.ask_first_action.is_some());
    }

    /// The held notice: counted, a sentence per reason, the release line,
    /// and a line and button per folder. Nothing held, nothing shown.
    #[test]
    fn the_held_notice_names_the_folders_and_says_it_releases() {
        use crate::daemon::automatic_gate::{REASON_ADMISSION_PER_SESSION, REASON_NO_SCOPE};
        let wire = serde_json::json!({
            "held_sessions": 3,
            "reasons": [REASON_ADMISSION_PER_SESSION, REASON_ADMISSION_PER_SESSION, REASON_NO_SCOPE],
            "projects": [
                { "project_id": "p-1", "project_label": "api", "held_sessions": 2 },
                { "project_id": "p-2", "project_label": "web", "held_sessions": 1 },
            ],
        });
        let n = gate_held_notice_for_wire(&wire).expect("a notice");
        assert_eq!(n.title, GATE_HELD_TITLE);
        assert_eq!(
            n.body,
            "3 sessions from projects set to contribute automatically are waiting."
        );
        assert_eq!(
            n.reasons,
            vec![
                "This commons does not yet accept automatic contributions from your account.",
                "You have not chosen how your traces may be used.",
            ]
        );
        assert_eq!(n.release, GATE_HELD_RELEASE);
        assert_eq!(n.projects.len(), 2);
        assert_eq!(n.projects[0].line, "api: 2 sessions waiting");
        assert_eq!(n.projects[1].line, "web: 1 session waiting");
        assert_eq!(n.projects[0].project_id.as_deref(), Some("p-1"));
        assert_eq!(n.projects[0].ask_first_action, Some(ASK_ME_FIRST_ACTION));

        let one = gate_held_notice_for_wire(&serde_json::json!({
            "held_sessions": 1, "reasons": ["some-future-reason"], "projects": [],
        }))
        .unwrap();
        assert_eq!(
            one.body,
            "1 session from a project set to contribute automatically is waiting."
        );
        assert_eq!(
            one.reasons,
            vec!["Something automatic contributing needs is not in place yet."]
        );

        for nothing in [
            serde_json::json!({ "held_sessions": 0, "reasons": [], "projects": [] }),
            serde_json::json!({}),
            serde_json::json!("held"),
        ] {
            assert!(gate_held_notice_for_wire(&nothing).is_none(), "{nothing}");
        }
    }

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

    /// The native shells hand the daemon's answer through unchanged and get
    /// back the facts, canonicalised, beside the words for them.
    #[test]
    fn route_disclosure_for_wire_pairs_the_facts_with_their_words() {
        use crate::disclosure::Route;
        let f = facts(Route::Witness);
        let wire = serde_json::to_value(&f).unwrap();
        let out = route_disclosure_for_wire(&wire).expect("readable");
        assert_eq!(out["facts"], wire);
        assert_eq!(
            out["copy"],
            serde_json::to_value(route_disclosure_copy(&f)).unwrap()
        );
        // An unknown route or origin is a newer daemon's answer: refused,
        // never rendered as the nearest one this build knows.
        let mut unknown = wire.clone();
        unknown["route"] = serde_json::json!("somewhere_new");
        assert!(route_disclosure_for_wire(&unknown).is_none());
        let mut origin = wire;
        origin["witness"]["origin"] = serde_json::json!("an_operator");
        assert!(route_disclosure_for_wire(&origin).is_none());
        assert!(route_disclosure_for_wire(&serde_json::json!("witness")).is_none());
    }

    /// An unreadable disclosure keeps its title: a shell that could not read
    /// the route still names the section, from the core rather than a
    /// literal of its own.
    #[test]
    fn the_unreadable_copy_carries_the_disclosure_title() {
        let copy = disclosure_unreadable_copy();
        assert_eq!(copy.title, DISCLOSURE_TITLE);
        assert_eq!(copy.panel, DISCLOSURE_UNREADABLE);
        assert_eq!(copy.session, DISCLOSURE_SESSION_UNREADABLE);
    }

    #[test]
    fn the_certificate_detail_copy_says_what_was_checked_and_when() {
        let copy = certificate_detail_copy();
        assert!(copy.verified_at_review.contains("when"));
        assert!(!copy.heading.is_empty());
        assert!(!copy.measurement_label.is_empty());
        assert!(!copy.signer_label.is_empty());
    }

    /// A name the daemon reported reads as exactly that disclosure's words;
    /// an unknown name reads as nothing.
    #[test]
    fn a_named_disclosure_reads_as_the_one_the_daemon_chose() {
        use crate::daemon::automatic_gate::Disclosure;
        assert_eq!(
            automatic_grant_copy_named("patterns_only"),
            Some(automatic_grant_copy(Disclosure::PatternsOnly))
        );
        assert_eq!(
            automatic_grant_copy_named("model_scrubbed"),
            Some(automatic_grant_copy(Disclosure::ModelScrubbed))
        );
        assert_eq!(automatic_grant_copy_named("scrubbed"), None);
        assert_eq!(automatic_grant_copy_named(""), None);
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

    /// Every refusal the daemon can send has its own sentence, except the
    /// two generic ones, and each says what happened to the invite.
    #[test]
    fn every_migration_refusal_is_worded() {
        let generic = legacy_migration_refusal_line("no-such-label");
        for label in crate::daemon::legacy_migration::LABELS {
            let line = legacy_migration_refusal_line(label);
            if matches!(
                *label,
                "legacy_migration_unavailable"
                    | "legacy_migration_not_enrolled"
                    | "legacy_migration_not_applicable"
                    | "legacy_migration_device_key_missing"
                    | "legacy_migration_pending"
                    | "legacy_migration_account_unavailable"
                    | "legacy_migration_link_refused"
            ) {
                continue;
            }
            assert_ne!(line, generic, "{label} has no sentence of its own");
        }
        assert!(
            legacy_migration_refusal_line("legacy_migration_tenant_pooled")
                .contains("keeps working"),
            "a pooled invite is told it keeps working"
        );
    }

    #[test]
    fn the_migration_notice_says_whether_armed_folders_were_kept() {
        let kept = legacy_migration_notice_for_wire(
            &serde_json::json!({"folders_kept": 2, "automatic_grant_kept": false}),
        )
        .unwrap();
        assert_eq!(kept.title, LEGACY_MIGRATION_NOTICE_TITLE);
        assert!(kept.title.contains("NEAR AI account"));
        assert_eq!(kept.folders, LEGACY_MIGRATION_NOTICE_ARMED_KEPT);
        let grant_only = legacy_migration_notice_for_wire(
            &serde_json::json!({"folders_kept": 0, "automatic_grant_kept": true}),
        )
        .unwrap();
        assert_eq!(grant_only.folders, LEGACY_MIGRATION_NOTICE_ARMED_KEPT);
        let none = legacy_migration_notice_for_wire(
            &serde_json::json!({"folders_kept": 0, "automatic_grant_kept": false}),
        )
        .unwrap();
        assert_eq!(none.folders, LEGACY_MIGRATION_NOTICE_NOTHING_ARMED);
        assert!(legacy_migration_notice_for_wire(&serde_json::Value::Null).is_none());
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

    /// R1: the Scrub check's words never claim a model looked at a session
    /// or that anything was quality checked. The Automatic check is a count
    /// of what the scrubber removed, and must read as one.
    #[test]
    fn the_scrub_check_copy_claims_no_model_or_quality_check() {
        for sentence in [
            SCRUB_CHECK_TITLE,
            SCRUB_CHECK_AUTOMATIC_LABEL,
            SCRUB_CHECK_AUTOMATIC_HELP,
            SCRUB_CHECK_MANUAL_LABEL,
            SCRUB_CHECK_MANUAL_HELP,
            SCRUB_CHECK_HELD,
        ] {
            let lower = sentence.to_lowercase();
            for claim in ["model", "quality", "verified", "certified", "safe"] {
                assert!(
                    !lower.contains(claim),
                    "{claim:?} in Scrub check copy: {sentence}"
                );
            }
        }
        assert!(
            SCRUB_CHECK_AUTOMATIC_HELP.contains("does not check that the scrubbing was right"),
            "Automatic says what it is not"
        );
    }

    /// The design's own worked example (Flow 2), verbatim.
    #[test]
    fn toast_sent_text_matches_the_design_example() {
        assert_eq!(
            toast_sent_text(7, 20, 1),
            "Sent. 1 left to decide \u{b7} upload limit 7 of 20"
        );
    }

    #[test]
    fn toast_sent_text_states_both_counts_whatever_they_are() {
        let text = toast_sent_text(0, 50, 0);
        assert!(text.starts_with("Sent."), "{text}");
        assert!(text.contains("0 left to decide"), "{text}");
        assert!(text.contains("upload limit 0 of 50"), "{text}");
    }

    /// The per-session notification's body is the consent sentence itself,
    /// not a paraphrase -- the same string a shell shows above Submit.
    #[test]
    fn session_notification_copy_uses_the_consent_sentence_and_look_then_decide() {
        let copy = session_notification_copy();
        assert_eq!(copy.body, GATE_STATEMENT);
        assert_eq!(copy.action, "Look, then decide");
        let value = serde_json::to_value(copy).expect("serialises");
        assert_eq!(value["body"], GATE_STATEMENT);
        assert_eq!(value["action"], "Look, then decide");
    }
}
