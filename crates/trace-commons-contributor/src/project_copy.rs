//! Shared project controls shown beside the contribution queue.

pub const IGNORE_PROJECT: &str = "Ignore project";
pub const IGNORE_PROJECT_TOOLTIP: &str = "Stops this project being offered and clears what it has waiting. \
     Anything already submitted is unaffected, and you can undo this in Settings.";

pub fn ignore_project_title(project: &str) -> String {
    format!("Ignore {project}?")
}

pub fn ignore_project_body(pending: usize) -> String {
    let tail = "Nothing already submitted is affected. You can undo this in Settings.";
    if pending == 0 {
        return format!("Stops this project being offered. {tail}");
    }
    let noun = if pending == 1 { "trace" } else { "traces" };
    format!("This removes {pending} waiting {noun} and stops this project being offered. {tail}")
}

pub fn ignore_project_reconciled(project: &str, promised: usize, purged: u64) -> Option<String> {
    if purged == promised as u64 {
        return None;
    }
    let clause = if purged == 1 {
        "1 waiting trace was removed".to_string()
    } else {
        format!("{purged} waiting traces were removed")
    };
    Some(format!(
        "Ignored {project}. The queue changed while you were deciding: {clause}, not {promised}."
    ))
}

pub fn arming_offer_evidence(project_label: &str, count: u32) -> String {
    let times = if count == 1 {
        "once".to_string()
    } else {
        format!("{count} times")
    };
    format!("You've contributed from {project_label} {times}.")
}

pub fn arming_offer_question(project_label: &str) -> String {
    format!("Contribute from {project_label} automatically?")
}

pub const ARMING_OFFER_CONFIRM: &str = "Turn on automatic contributing";
pub const ARMING_OFFER_DECLINE: &str = "Not now";
/// The confirmation shown before a project is armed **from now**, which is
/// what `set_project_mode` `auto_upload` does by default (K5): new sessions
/// go without asking, and sessions already on this Mac keep waiting for the
/// contributor to pick them.
///
/// **DRAFT, NEEDS APPROVAL.** The first paragraph changed with the from-now
/// default; the second and third are the agreed text unchanged. It still
/// opens "Sessions from this project will be scrubbed", which
/// `arming_wording::project_arming_claim` depends on.
///
/// Each sentence is held to what the daemon does, because this is the only
/// thing a contributor reads before sessions start leaving without a look.
/// Before the default changed it had to say "including any already waiting",
/// because arming then approved every settled pending entry; that sentence
/// now belongs to [`ARMING_BODY_WITH_BACKLOG`] alone. It also used to say
/// three things that were not true:
///
/// - "Every future session" -- at the time, arming also sent sessions
///   already waiting.
/// - "A session is sent a day after you last work on it, so there is time to
///   change your mind" -- a waiting session already quiet for a day goes out
///   at once, and the settle window is not a control (`ARMED_SETTLE_SECS`).
///   What is true, and stated instead, is that nothing goes before it.
/// - "You can turn this off at any time", with nothing said about what that
///   does -- turning automatic off left every session it had approved still
///   uploading. It now returns them to waiting, and the sentence says so.
pub const ARMING_BODY: &str = "Sessions from this project will be scrubbed and contributed \
     without asking you, from now on. You won't review them first. Sessions already on this Mac \
     keep waiting for you to pick them.\n\nNo session is sent until it has been quiet for a \
     day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting \
     for you, and anything already sent stays sent.";

/// The confirmation for arming **with the backlog**
/// (`set_project_mode` `include_backlog: true`): the agreed wording from
/// before the from-now default, which is still exactly what that call does.
pub const ARMING_BODY_WITH_BACKLOG: &str = "Sessions from this project will be scrubbed and \
     contributed without asking you, including any already waiting. You won't review them \
     first.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off \
     at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already \
     sent stays sent.";

/// Customize's words for the from-now rule, the past-session picker, and
/// "Keep on this Mac" (K5), in one table so no shell writes its own.
///
/// **DRAFT, NEEDS APPROVAL**, every sentence. Each is held to what the daemon
/// does: see "Arming from now" and "`keep`: Keep on this Mac" in
/// `docs/contributor-daemon-ipc-v1_1.md`.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CustomizeCopy {
    /// The rule's one-line description beside "Share automatically".
    pub share_automatically_rule: &'static str,
    /// What happens to the folder's backlog under that rule.
    pub backlog_rule: &'static str,
    /// The picker's heading.
    pub picker_heading: &'static str,
    /// The picker's explainer, above the folders.
    pub picker_explainer: &'static str,
    /// The picker's action.
    pub picker_include: &'static str,
    /// The sheet's button.
    pub keep: &'static str,
    /// Shown once a session is kept.
    pub kept_confirmation: &'static str,
    /// The heading over the kept list.
    pub kept_heading: &'static str,
    /// The undo on a kept row.
    pub undo_keep: &'static str,
    /// Shown when an undo is refused because the queue is full.
    pub undo_keep_queue_full: &'static str,
    /// Shown when an undo is refused because the folder is set to Never.
    pub undo_keep_folder_never: &'static str,
}

/// The one table of Customize copy. See [`CustomizeCopy`].
#[must_use]
pub fn customize_copy() -> CustomizeCopy {
    CustomizeCopy {
        share_automatically_rule: "New sessions from this folder are contributed without asking you.",
        backlog_rule: "Sessions already on this Mac keep waiting until you pick them below.",
        picker_heading: "Past sessions, by folder",
        picker_explainer: "Tick the sessions already on this Mac that you want to include. \
             Anything you leave unticked keeps waiting for you.",
        picker_include: "Include selected",
        keep: "Keep on this Mac",
        kept_confirmation: "Kept on this Mac. It won't be sent, and it won't expire. You can \
             offer it again later.",
        kept_heading: "Kept on this Mac",
        undo_keep: "Offer it again",
        undo_keep_queue_full: "Too many sessions are waiting. Decide some of them first, then \
             offer this one again.",
        undo_keep_folder_never: "This folder is set to Never. Change its rule first, then offer \
             this session again.",
    }
}

/// Every word of the ignore-project control and its confirmation, for one
/// project with `pending` sessions waiting. One table so a shell renders it
/// whole rather than assembling the title and body itself.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct IgnoreProjectCopy {
    pub title: String,
    pub body: String,
    pub button: &'static str,
    pub tooltip: &'static str,
}

/// The ignore-project words for `project` with `pending` sessions waiting.
#[must_use]
pub fn ignore_project_copy(project: &str, pending: usize) -> IgnoreProjectCopy {
    IgnoreProjectCopy {
        title: ignore_project_title(project),
        body: ignore_project_body(pending),
        button: IGNORE_PROJECT,
        tooltip: IGNORE_PROJECT_TOOLTIP,
    }
}

/// Every word of the arming offer and the arming confirmation, for one
/// project the contributor has contributed from `count` times.
///
/// `body` is the from-now confirmation and `body_with_backlog` the one for
/// `include_backlog`; `customize` is Customize's table (K5). DRAFT, NEEDS
/// APPROVAL where [`ARMING_BODY`] and [`customize_copy`] say so.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ArmingOfferCopy {
    pub evidence: String,
    pub question: String,
    pub confirm: &'static str,
    pub decline: &'static str,
    pub body: &'static str,
    pub body_with_backlog: &'static str,
    pub customize: CustomizeCopy,
}

/// The arming words for `project_label`, contributed from `count` times.
#[must_use]
pub fn arming_offer_copy(project_label: &str, count: u32) -> ArmingOfferCopy {
    ArmingOfferCopy {
        evidence: arming_offer_evidence(project_label, count),
        question: arming_offer_question(project_label),
        confirm: ARMING_OFFER_CONFIRM,
        decline: ARMING_OFFER_DECLINE,
        body: ARMING_BODY,
        body_with_backlog: ARMING_BODY_WITH_BACKLOG,
        customize: customize_copy(),
    }
}

// ---------------------------------------------------------------------------
// The menu-bar Contribution mode pill and its global override (#1173, R13)
// ---------------------------------------------------------------------------
//
// Every sentence in this section was approved on 2026-10-02. Each is held to
// what the daemon does under `policy::ContributionOverride`; see "The
// contribution override" in `docs/contributor-daemon-ipc-v1_1.md`.

/// The pill's heading.
pub const CONTRIBUTION_MODE_TITLE: &str = "Contribution mode";

/// The pill when folders differ and no override
/// is in force (`status.contribution_mode: "mixed"`).
pub const CONTRIBUTION_MODE_MIXED: &str = "Mixed";

/// The `notify_only` choice's label.
pub const CONTRIBUTION_MODE_ASK_LABEL: &str = "Ask me";
/// The `auto_upload` choice's label.
pub const CONTRIBUTION_MODE_AUTO_LABEL: &str = "Auto contribute";
/// The `ignore` choice's label.
pub const CONTRIBUTION_MODE_NEVER_LABEL: &str = "Never";

/// The `notify_only` sub-list line, from the
/// menu-bar handoff. True under an "Ask me" override: no folder sends
/// unattended, and a folder set to Never stays off.
pub const CONTRIBUTION_MODE_ASK_LINE: &str = "Every finished session waits for you.";

/// The `auto_upload` sub-list line, from the
/// handoff. It is a summary, not the disclosure: the confirmation
/// ([`contribution_override_confirm_copy`]) carries that, including that
/// sessions already on this Mac keep waiting and Never folders stay off.
pub const CONTRIBUTION_MODE_AUTO_LINE: &str = "Scrubbed sessions go; the digest tells you.";

/// The `ignore` sub-list line, from the handoff.
pub const CONTRIBUTION_MODE_NEVER_LINE: &str = "Nothing is queued or sent.";

/// Under the pill's `auto_upload` label when
/// `status.contribution_mode_partial` is true (#1208): a folder set to Never,
/// or sessions from a folder the app could not identify, do not upload.
pub const CONTRIBUTION_MODE_AUTO_PARTIAL: &str = "Except folders set to Never. Sessions from a folder the app can't identify still wait for you.";

/// Shown under the pill while an override is in
/// force (`status.contribution_override` not null), so it reads "override"
/// rather than a folder roll-up.
pub const CONTRIBUTION_OVERRIDE_ACTIVE: &str =
    "This setting is overriding each folder's own setting.";

/// The action that clears the override
/// (`clear_contribution_override`).
pub const CONTRIBUTION_OVERRIDE_CLEAR: &str = "Use each folder's own setting";

/// The "Ask me" override's confirmation.
pub const CONTRIBUTION_OVERRIDE_ASK_TITLE: &str = "Ask before contributing from every folder?";
/// Held to `set_contribution_override`
/// `notify_only`: every folder resolves to ask-first, what was approved
/// without you and not yet sent goes back to waiting, Never folders stay off,
/// and clearing restores each folder's own mode.
pub const CONTRIBUTION_OVERRIDE_ASK_BODY: &str = "Every folder asks you first until you turn \
     this off, including folders set to contribute automatically. Nothing more is sent on its \
     own, and anything approved without you that hasn't been sent goes back to waiting for \
     you. Folders set to Never stay off.\n\nTurning this off puts each folder back on its own \
     setting.";
pub const CONTRIBUTION_OVERRIDE_ASK_CONFIRM: &str = "Ask me everywhere";

/// The "Never" override's confirmation.
pub const CONTRIBUTION_OVERRIDE_NEVER_TITLE: &str = "Stop contributing from every folder?";
/// Held to `set_contribution_override` `ignore`:
/// nothing is queued or sent -- `drain_approved` holds even what the
/// contributor approved, and `approve` is refused
/// (`contribution-override-never`) -- nothing waiting is refused, a held
/// approval goes once the override clears, and a session finished meanwhile
/// is offered then by that folder's own setting -- so a folder set to
/// contribute automatically sends it.
pub const CONTRIBUTION_OVERRIDE_NEVER_BODY: &str = "Nothing is queued or sent from any folder \
     until you turn this off, including sessions you already approved.\n\nTurning this off \
     puts each folder back on its own setting, and sessions you approved are sent. Sessions you \
     finish in the meantime are then treated by that setting, so a folder set to contribute \
     automatically sends them without asking.";
pub const CONTRIBUTION_OVERRIDE_NEVER_CONFIRM: &str = "Stop everywhere";

/// The "Auto contribute" override's confirmation.
pub const CONTRIBUTION_OVERRIDE_AUTO_TITLE: &str = "Contribute automatically from every folder?";
/// The arming body ([`ARMING_BODY`]) for every
/// folder at once, held to `set_contribution_override` `auto_upload`: from
/// now (nothing already on disk is sent unattended), Never folders stay off,
/// the settle window, and what turning it off does. Its second paragraph is
/// [`ARMING_BODY`]'s, unchanged.
pub const CONTRIBUTION_OVERRIDE_AUTO_BODY: &str = "Sessions from every folder will be scrubbed \
     and contributed without asking you, from now on, until you turn this off. You won't review \
     them first. Sessions already on this Mac keep waiting for you to pick them, and folders set \
     to Never stay off.\n\nNo session is sent until it has been quiet for a day.\n\nTurning this \
     off puts each folder back on its own setting. Anything it hasn't sent yet from a folder \
     that asks first goes back to waiting for you, and anything already sent stays sent.";
/// The confirm button: the arming offer's own ([`ARMING_OFFER_CONFIRM`]).
pub const CONTRIBUTION_OVERRIDE_AUTO_CONFIRM: &str = ARMING_OFFER_CONFIRM;

/// Every confirmation's cancel.
pub const CONTRIBUTION_OVERRIDE_CANCEL: &str = "Cancel";

/// One choice in the pill's sub-list.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ContributionModeChoice {
    /// The `mode` `set_contribution_override` takes for it.
    pub mode: &'static str,
    pub label: &'static str,
    pub line: &'static str,
}

/// Every word of the Contribution mode pill (#1173). Approved 2026-10-02.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ContributionModeCopy {
    pub title: &'static str,
    pub mixed: &'static str,
    /// Ask me, Auto contribute, Never, in that order.
    pub choices: Vec<ContributionModeChoice>,
    pub override_active: &'static str,
    pub clear: &'static str,
    /// Shown under the `auto_upload` label exactly when
    /// `status.contribution_mode_partial` is true.
    pub auto_partial: &'static str,
}

/// The pill's table. See [`ContributionModeCopy`].
#[must_use]
pub fn contribution_mode_copy() -> ContributionModeCopy {
    ContributionModeCopy {
        title: CONTRIBUTION_MODE_TITLE,
        mixed: CONTRIBUTION_MODE_MIXED,
        choices: vec![
            ContributionModeChoice {
                mode: "notify_only",
                label: CONTRIBUTION_MODE_ASK_LABEL,
                line: CONTRIBUTION_MODE_ASK_LINE,
            },
            ContributionModeChoice {
                mode: "auto_upload",
                label: CONTRIBUTION_MODE_AUTO_LABEL,
                line: CONTRIBUTION_MODE_AUTO_LINE,
            },
            ContributionModeChoice {
                mode: "ignore",
                label: CONTRIBUTION_MODE_NEVER_LABEL,
                line: CONTRIBUTION_MODE_NEVER_LINE,
            },
        ],
        override_active: CONTRIBUTION_OVERRIDE_ACTIVE,
        clear: CONTRIBUTION_OVERRIDE_CLEAR,
        auto_partial: CONTRIBUTION_MODE_AUTO_PARTIAL,
    }
}

/// The confirmation for one override, every word of it.
///
/// `arming` is present for `auto_upload` only: the arming disclosure, the
/// Flow 1 grant screens' table (`consent_copy::automatic_contribution_copy`)
/// with the disclosure the core chose -- patterns only, since an override
/// is shown before any of the folders it arms has earned more. A shell
/// renders it whole beside `body`, as the grant screens do, and sends
/// `confirm: true` only from this dialog's confirm.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ContributionOverrideConfirmCopy {
    pub mode: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
    pub arming: Option<crate::consent_copy::AutomaticGrantCopy>,
}

/// The confirmation for the override to `mode`, for the contributor
/// configuration `config`. See [`ContributionOverrideConfirmCopy`].
#[must_use]
pub fn contribution_override_confirm_copy(
    mode: crate::daemon::policy::ProjectMode,
    config: Option<&crate::config::ContributorConfig>,
) -> ContributionOverrideConfirmCopy {
    use crate::daemon::policy::ProjectMode;
    match mode {
        ProjectMode::NotifyOnly => ContributionOverrideConfirmCopy {
            mode: "notify_only",
            title: CONTRIBUTION_OVERRIDE_ASK_TITLE,
            body: CONTRIBUTION_OVERRIDE_ASK_BODY,
            confirm: CONTRIBUTION_OVERRIDE_ASK_CONFIRM,
            cancel: CONTRIBUTION_OVERRIDE_CANCEL,
            arming: None,
        },
        ProjectMode::Ignore => ContributionOverrideConfirmCopy {
            mode: "ignore",
            title: CONTRIBUTION_OVERRIDE_NEVER_TITLE,
            body: CONTRIBUTION_OVERRIDE_NEVER_BODY,
            confirm: CONTRIBUTION_OVERRIDE_NEVER_CONFIRM,
            cancel: CONTRIBUTION_OVERRIDE_CANCEL,
            arming: None,
        },
        ProjectMode::AutoUpload => ContributionOverrideConfirmCopy {
            mode: "auto_upload",
            title: CONTRIBUTION_OVERRIDE_AUTO_TITLE,
            body: CONTRIBUTION_OVERRIDE_AUTO_BODY,
            confirm: CONTRIBUTION_OVERRIDE_AUTO_CONFIRM,
            cancel: CONTRIBUTION_OVERRIDE_CANCEL,
            arming: Some(crate::consent_copy::automatic_contribution_copy(config)),
        },
    }
}

#[cfg(test)]
mod contribution_override_copy_tests {
    use super::*;
    use crate::daemon::policy::ProjectMode;

    #[test]
    fn the_pill_offers_the_three_overrides_with_the_handoff_lines() {
        let c = contribution_mode_copy();
        let modes: Vec<&str> = c.choices.iter().map(|c| c.mode).collect();
        assert_eq!(modes, ["notify_only", "auto_upload", "ignore"]);
        assert_eq!(c.choices[0].line, "Every finished session waits for you.");
        assert_eq!(
            c.choices[1].line,
            "Scrubbed sessions go; the digest tells you."
        );
        assert_eq!(c.choices[2].line, "Nothing is queued or sent.");
        // Each choice's mode is one the override method takes.
        for choice in &c.choices {
            serde_json::from_value::<ProjectMode>(serde_json::json!(choice.mode)).unwrap();
        }
    }

    /// Only the arming override carries the arming disclosure, and it is the
    /// patterns-only one: nothing an override is shown before has earned the
    /// model-scrub wording (R1).
    #[test]
    fn only_the_auto_override_carries_the_arming_disclosure() {
        for mode in [ProjectMode::NotifyOnly, ProjectMode::Ignore] {
            assert!(
                contribution_override_confirm_copy(mode, None)
                    .arming
                    .is_none()
            );
        }
        let auto = contribution_override_confirm_copy(ProjectMode::AutoUpload, None);
        let arming = auto.arming.expect("the arming disclosure");
        assert_eq!(arming.disclosure, "patterns_only");
        assert!(arming.model_scrubbed.is_none());
        assert_eq!(arming.no_review, crate::consent_copy::AUTO_NO_REVIEW);
        assert_eq!(auto.confirm, ARMING_OFFER_CONFIRM);
    }

    /// The bodies say what the daemon does: from now, Never stays off, and
    /// clearing restores each folder.
    #[test]
    fn each_override_body_says_what_the_daemon_does() {
        let auto = CONTRIBUTION_OVERRIDE_AUTO_BODY;
        assert!(auto.contains("from now on"));
        assert!(auto.contains("keep waiting for you"));
        assert!(!auto.contains("already waiting"), "the backlog does not go");
        assert!(auto.contains("Never stay off"));
        assert!(auto.contains("No session is sent until it has been quiet for a day."));
        assert!(CONTRIBUTION_OVERRIDE_ASK_BODY.contains("Never stay off"));
        for body in [
            CONTRIBUTION_OVERRIDE_AUTO_BODY,
            CONTRIBUTION_OVERRIDE_ASK_BODY,
            CONTRIBUTION_OVERRIDE_NEVER_BODY,
        ] {
            assert!(body.contains("back on its own setting"), "{body}");
        }
    }

    #[test]
    fn the_tables_serialize_the_keys_the_shells_read() {
        let pill = serde_json::to_value(contribution_mode_copy()).unwrap();
        for key in ["title", "mixed", "override_active", "clear"] {
            assert!(pill[key].is_string(), "{key}");
        }
        assert_eq!(pill["choices"].as_array().unwrap().len(), 3);
        let confirm = serde_json::to_value(contribution_override_confirm_copy(
            ProjectMode::Ignore,
            None,
        ))
        .unwrap();
        for key in ["mode", "title", "body", "confirm", "cancel"] {
            assert!(confirm[key].is_string(), "{key}");
        }
        assert!(confirm["arming"].is_null());
    }
}

#[cfg(test)]
mod copy_table_tests {
    use super::*;

    #[test]
    fn the_ignore_table_carries_the_project_and_the_count() {
        let copy = ignore_project_copy("api", 3);
        assert_eq!(copy.title, "Ignore api?");
        assert_eq!(copy.body, ignore_project_body(3));
        assert!(copy.body.contains("3 waiting traces"));
        assert_eq!(copy.button, IGNORE_PROJECT);
        assert_eq!(copy.tooltip, IGNORE_PROJECT_TOOLTIP);
        assert!(
            ignore_project_copy("api", 1)
                .body
                .contains("1 waiting trace ")
        );
        assert!(!ignore_project_copy("api", 0).body.contains("waiting trace"));
    }

    #[test]
    fn the_ignore_table_serializes_the_keys_the_shells_read() {
        let value = serde_json::to_value(ignore_project_copy("api", 2)).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 4);
        for key in ["title", "body", "button", "tooltip"] {
            assert!(value[key].is_string(), "{key}");
        }
    }

    #[test]
    fn the_arming_table_names_the_project_and_its_evidence() {
        let copy = arming_offer_copy("api", 5);
        assert_eq!(copy.evidence, "You've contributed from api 5 times.");
        assert_eq!(
            arming_offer_copy("api", 1).evidence,
            "You've contributed from api once."
        );
        assert_eq!(copy.question, "Contribute from api automatically?");
        assert_eq!(copy.confirm, ARMING_OFFER_CONFIRM);
        assert_eq!(copy.decline, ARMING_OFFER_DECLINE);
        assert_eq!(copy.body, ARMING_BODY);
        assert_eq!(copy.body_with_backlog, ARMING_BODY_WITH_BACKLOG);
        assert_eq!(copy.customize, customize_copy());
    }

    #[test]
    fn the_arming_table_serializes_the_keys_the_shells_read() {
        let value = serde_json::to_value(arming_offer_copy("api", 2)).unwrap();
        for key in [
            "evidence",
            "question",
            "confirm",
            "decline",
            "body",
            "body_with_backlog",
        ] {
            assert!(value[key].is_string(), "{key}");
        }
        assert!(value["customize"]["keep"].is_string());
    }
}

#[cfg(test)]
mod arming_body_tests {
    use super::{ARMING_BODY, ARMING_BODY_WITH_BACKLOG, customize_copy};

    /// Verbatim, because the macOS shell holds its own copy and pins it to
    /// this one; a change here is a change there.
    #[test]
    fn the_arming_body_is_exactly_what_was_agreed() {
        assert_eq!(
            ARMING_BODY,
            "Sessions from this project will be scrubbed and contributed without asking you, from now on. You won't review them first. Sessions already on this Mac keep waiting for you to pick them.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays sent."
        );
        assert_eq!(
            ARMING_BODY_WITH_BACKLOG,
            "Sessions from this project will be scrubbed and contributed without asking you, including any already waiting. You won't review them first.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays sent."
        );
    }

    /// The from-now default must not claim the backlog goes, and the
    /// backlog variant must say it does (K5).
    #[test]
    fn each_arming_body_says_what_happens_to_the_backlog() {
        assert!(!ARMING_BODY.contains("already waiting"));
        assert!(ARMING_BODY.contains("keep waiting for you"));
        assert!(ARMING_BODY_WITH_BACKLOG.contains("including any already waiting"));
    }

    /// The Customize table says what the daemon does: a kept session is
    /// neither sent nor expired, and the backlog waits under the rule.
    #[test]
    fn the_customize_copy_matches_the_keep_and_from_now_semantics() {
        let c = customize_copy();
        assert!(c.kept_confirmation.contains("won't be sent"));
        assert!(c.kept_confirmation.contains("won't expire"));
        assert!(c.backlog_rule.contains("keep waiting"));
        assert!(!c.share_automatically_rule.contains("already"));
    }

    /// The three claims review found untrue may not come back.
    #[test]
    fn the_arming_body_makes_none_of_the_retracted_claims() {
        for claim in [
            "Every future session",
            "change your mind",
            "a day after you last work",
        ] {
            assert!(!ARMING_BODY.contains(claim), "{claim:?} is not true");
        }
        assert!(ARMING_BODY_WITH_BACKLOG.contains("including any already waiting"));
        assert!(ARMING_BODY.contains("goes back to waiting"));
        assert!(ARMING_BODY_WITH_BACKLOG.contains("goes back to waiting"));
    }
}
