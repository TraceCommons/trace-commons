//! Copy for decisions made from the scrubbed contribution preview.

pub const NOTHING_MATCHED: &str = "nothing matched";
pub const REDACTION_CATEGORY_LOCAL_PATH: &str = "File paths from this machine.";
pub const REDACTION_CATEGORY_SECRET: &str =
    "API keys, tokens, private keys, and high-entropy strings found next to credential words.";
pub const REDACTION_CATEGORY_PRIVACY_FILTER: &str =
    "Names, emails, and other personal details found in prose.";
pub const REDACTION_CATEGORY_SENSITIVE_FIELD: &str =
    "Fields whose name marks them sensitive, like password or authorization.";
pub const REDACTION_CATEGORY_TOOL_SENSITIVE_FIELD: &str =
    "Tool-call arguments whose name marks them sensitive.";
pub const REDACTION_CATEGORY_RESIDUAL: &str = "Found, and still in what would be sent. Either a credential inside a correction \
     you wrote, which is kept on purpose, or a field scrubbing does not reach.";
pub const REDACTION_CATEGORY_UNKNOWN: &str =
    "Removed by a pattern this version has no description for.";

pub fn redaction_row_counts(occurrences: u32, distinct: u32) -> String {
    if distinct > 0 && distinct < occurrences {
        format!("{occurrences} ({distinct} distinct)")
    } else {
        format!("{occurrences}")
    }
}

/// A detection that survived scrubbing remains in the outgoing envelope.
/// `count` measures detection sites, not the number of secret values.
pub fn residual_secret_line(count: u32, sites: &[String]) -> String {
    let head = if count == 1 {
        "A secret found here is still in what would be sent".to_string()
    } else {
        format!("Secrets found in {count} places are still in what would be sent")
    };
    if sites.is_empty() {
        return head;
    }
    format!("{head} ({})", sites.join(", "))
}

// ---------------------------------------------------------------------------
// Flow 2 states (#1118 K3): the scrub state, "worth a second look", and the
// per-line unsure hints. Worded from the design's review-sheet mock.
//
// Every sentence in this section was approved 2026-10-06.

/// Approved 2026-10-06. The heading over a session with a
/// `second_look` reason.
pub const SECOND_LOOK_HEADING: &str = "Worth a second look";

/// Approved 2026-10-06. A session no preview has scrubbed yet. Not
/// "0 marks": nobody has counted.
pub const NOT_YET_SCRUBBED: &str = "Not yet scrubbed";

/// Approved 2026-10-06. The row's scrub state: `Scrubbed · 7 marks`,
/// `Scrubbed · 1 mark`, or [`NOT_YET_SCRUBBED`] for `None` (the absent
/// `marks` key). `None` is never rendered as zero.
pub fn scrub_state_line(marks: Option<u32>) -> String {
    match marks {
        None => NOT_YET_SCRUBBED.to_string(),
        Some(1) => "Scrubbed \u{00b7} 1 mark".to_string(),
        Some(n) => format!("Scrubbed \u{00b7} {n} marks"),
    }
}

/// Approved 2026-10-06. Why one `second_look` reason waits, or `None`
/// for a label this build does not know.
pub fn second_look_line(reason: &str) -> Option<&'static str> {
    match reason {
        crate::daemon::second_look::REASON_NOTHING_MATCHED => Some(
            "No personal details matched, only file paths or nothing at all; that is why this one waits.",
        ),
        crate::daemon::second_look::REASON_LOOKS_UNSURE => Some(
            "Something here looks like an email, phone number or key that was not matched; that is why this one waits.",
        ),
        crate::daemon::second_look::REASON_TRIMMED_TO_FIT => Some(
            "Trimmed to fit the upload limit, so part of this session is not in what would be sent. That is why this one waits.",
        ),
        _ => None,
    }
}

/// The line for a `second_look` reason
/// [`second_look_line`] has no sentence for. Only reachable if a reason is
/// added without one, which `every_second_look_reason_has_its_own_line`
/// fails on; it exists so `second_look_lines` stays index-for-index with
/// `second_look` even then, rather than silently dropping a line.
pub const SECOND_LOOK_FALLBACK_LINE: &str = "This one waits for you to look before it goes.";

/// [`second_look_line`], made total: the reason's own sentence, or
/// [`SECOND_LOOK_FALLBACK_LINE`].
#[must_use]
pub fn second_look_line_or_fallback(reason: &str) -> &'static str {
    second_look_line(reason).unwrap_or(SECOND_LOOK_FALLBACK_LINE)
}

/// Approved 2026-10-06. The hint under an unsure span, or `None` for a
/// label this build does not know.
pub fn unsure_hint_line(label: &str) -> Option<&'static str> {
    match label {
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_EMAIL => {
            Some("Looks like an email. Not matched. Your call.")
        }
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_PHONE => {
            Some("Looks like a phone number. Not matched. Your call.")
        }
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_KEY => {
            Some("Looks like a key. Not matched. Your call.")
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The glass monitor's Traces tab (#1173 R6/R7): the session inspector's row
// labels, the review's actions, the Traces badge's text equivalent, and what
// the tab says when the core does not answer. One table, so the monitor
// writes none of its own.

/// What the Traces tab says when the core does
/// not answer. The tree below it is the last one the core reported.
pub const MONITOR_CORE_UNREACHABLE: &str =
    "The watcher isn't answering. This is what it last reported.";

/// What the Traces tab says when a request the
/// core answered failed (a refused write, or a reply this build could not
/// read).
pub const MONITOR_REQUEST_FAILED: &str = "That didn't go through. Try again.";

/// Every fixed word of the monitor's Traces tab. The row labels are single
/// words; `keep` and `undo_keep` are Customize's (K5) so the two surfaces
/// cannot drift.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorTracesCopy {
    /// A session's pill, which opens its review.
    pub review: &'static str,
    /// The inspector's rows.
    pub tool: &'static str,
    pub folder: &'static str,
    pub started: &'static str,
    pub length: &'static str,
    pub prompts: &'static str,
    pub size: &'static str,
    pub sends: &'static str,
    pub marks: &'static str,
    pub unsure: &'static str,
    /// The inspector's rows for whether the session may be contributed and
    /// what the privacy witness attested; their values are the core's own
    /// state and reason lines.
    pub eligibility: &'static str,
    pub attestation: &'static str,
    /// A row's hold flag in words, so the amber flag is never colour only.
    pub held: &'static str,
    /// The marker a debug build shows over sample data.
    pub sample: &'static str,
    /// The inspector's rows for the full preview's residual-risk label and
    /// the personal-information categories it saw (categories only).
    pub residual_risk: &'static str,
    pub personal_information: &'static str,
    /// Added to the Traces badge's text equivalent when a waiting session
    /// is worth a second look (nothing matched, or trimmed to fit).
    pub second_look_waiting: &'static str,
    /// The review's actions.
    pub contribute: &'static str,
    pub keep: &'static str,
    pub dismiss: &'static str,
    /// Taking back a Contribute while it is still held.
    pub undo_contribute: &'static str,
    /// Taking back a Keep.
    pub undo_keep: &'static str,
    /// The core did not answer; see [`MONITOR_CORE_UNREACHABLE`].
    pub core_unreachable: &'static str,
    /// A request failed; see [`MONITOR_REQUEST_FAILED`].
    pub request_failed: &'static str,
    /// Ron's plain Dismiss (#1146): the undo bar's, the Dismiss-session
    /// confirmation's and the review card's. Apart from [`Self::dismiss`],
    /// native's "Not this one", which this does not replace.
    pub dismiss_action: &'static str,
    /// The arming offer's eyebrow, over the core's arming copy.
    pub optional_automation: &'static str,
    /// The Traces tree's rows and the Dismiss-session confirmation.
    pub tree: MonitorTreeCopy,
    /// Counts the tree, the inspectors and the summary share.
    pub counts: MonitorCountsCopy,
    /// The tool, folder and session inspectors' headings, and Submit-all-as.
    pub inspector: MonitorInspectorCopy,
    /// The summary inspector shown when nothing is selected.
    pub summary_panel: MonitorSummaryCopy,
    /// The session review card (Ron's `WaitingReview`).
    pub session_review: MonitorSessionReviewCopy,
    /// Look inside, read-only (Ron's `PreviewInspector`), with the native
    /// review actions and the witness consent line.
    pub look_inside: MonitorLookInsideCopy,
    /// The undo bar after an approve (Ron's `UndoBar`).
    pub undo: MonitorUndoCopy,
}

// ---------------------------------------------------------------------------
// Ron's #1146 inspector (#1241). His strings verbatim from
// `origin/claude/tc-monitor-frontend-refactor-8ecf22`
// (`tauri-desktop/frontend/src/features/waiting/`), with these departures:
//
// - Each number is a `{name}` hole from [`MONITOR_PLACEHOLDERS`]; the shell
//   fills it and adds nothing else. Where Ron pluralised with a helper, the
//   singular is its own line.
// - A word a table already holds keeps its key: Contribute, Undo, Size,
//   Tool, Folder, Residual risk here; Watching, On, Off, Folders, Waiting,
//   Summary on [`MonitorScreensCopy`].
// - No credit, community or withdrawal words, and none of Ron's routing
//   state labels: those stay on the core tables native already reads
//   (`history_copy`, `routing_copy`). Since the owner's 2026-10-06 ruling
//   (#1146's wording wins), Home's and History's #1146 lines -- his
//   "credit pending" accessory, the filters, the empty states -- are
//   [`MonitorShellCopy`].
// - No "Share automatically": the folder rule is Ask me / Automatic /
//   Never, from `project_copy::FOLDER_MODE_LABELS`.

/// Every placeholder the monitor tables use, each written `{name}`:
/// Ron's, and History's and Inference's (`shown`, `hours`) before them.
/// `uploads` and `megabytes` are the daily limit's two remainders.
pub const MONITOR_PLACEHOLDERS: &[&str] = &[
    "count",
    "total",
    "max",
    "tool",
    "label",
    "seconds",
    "when",
    "size",
    "start",
    "end",
    "shown",
    "hours",
    "uploads",
    "megabytes",
    "amount",
];

/// The Traces tree (`traces-tree.tsx`).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorTreeCopy {
    /// The tree's accessible name.
    pub tree_label: &'static str,
    /// While the queue is first read.
    pub reading_queue: &'static str,
    /// A folder's Submit pill with nothing eligible, with `{count}`
    /// eligible, and while it sends.
    pub submit: &'static str,
    pub submit_count: &'static str,
    pub submitting: &'static str,
    /// The folder Submit pill's tip.
    pub submit_tip: &'static str,
    /// Beside the core's withheld line under a folder.
    pub eligible_count: &'static str,
    /// A session's pill while its review is open, and the pill's tip.
    pub reviewing: &'static str,
    pub review_tip: &'static str,
    /// An ignored folder's sub line.
    pub ignored_folder: &'static str,
    /// The folder row menu's one item.
    pub ignore_folder: &'static str,
    /// The folder and tool switches' accessible names.
    pub watch_folder: &'static str,
    pub watch_tool: &'static str,
    /// The session row menu's one item, and its confirmation.
    pub dismiss_session: &'static str,
    pub dismiss_session_title: &'static str,
    /// `{when}` is the session's start, `{size}` its size.
    pub dismiss_session_body: &'static str,
    pub dismiss_session_keep: &'static str,
    pub dismissing: &'static str,
    pub dismiss_session_failed: &'static str,
}

/// Counted lines the tree, the inspectors and the summary share.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorCountsCopy {
    pub sessions_waiting_one: &'static str,
    pub sessions_waiting: &'static str,
    pub waiting_count: &'static str,
    pub contributed_count: &'static str,
    pub project_count_one: &'static str,
    pub project_count: &'static str,
}

/// The tool, folder and session inspectors (`waiting-page.tsx`) and
/// Submit-all-as (`submit-all-as-control.tsx`).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorInspectorCopy {
    pub decisions: &'static str,
    /// A tool whose watching the core has not been told.
    pub not_set: &'static str,
    pub sessions_folder: &'static str,
    pub project: &'static str,
    /// The folder and session headers' sub lines; `{tool}` is the tool.
    pub project_of: &'static str,
    pub session_of: &'static str,
    pub path: &'static str,
    pub contribution_rule: &'static str,
    pub no_rule: &'static str,
    /// Submit-all-as's sentence, over the core's `outcome` words.
    pub apply_outcome_one: &'static str,
    pub apply_outcome: &'static str,
    pub cancel: &'static str,
    /// The folder inspector's Decisions card (`waiting-project-folder.tsx`):
    /// what is waiting there, one and then counted, and Submit all with the
    /// eligible count.
    pub waiting_sessions_one: &'static str,
    pub waiting_sessions: &'static str,
    pub submit_all_eligible: &'static str,
}

/// The summary inspector (`waiting-page.tsx`'s `SummaryInspector` and
/// `queue-outcome-disclosure.tsx`).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorSummaryCopy {
    pub tools_watched: &'static str,
    pub waiting_for_you: &'static str,
    pub worth_a_second_look: &'static str,
    pub uploads_today: &'static str,
    pub statistics: &'static str,
    pub top_projects: &'static str,
    pub top_tools: &'static str,
    pub no_longer_waiting: &'static str,
    /// What the no-longer-waiting counts cover.
    pub no_longer_waiting_scope: &'static str,
}

/// The session review card (`waiting-review.tsx`). The verdict and
/// correction words are the core's `outcome` table.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorSessionReviewCopy {
    pub eyebrow: &'static str,
    pub heading: &'static str,
    pub enrolled: &'static str,
    pub not_enrolled: &'static str,
    pub no_opening_prompt: &'static str,
    /// `{size}`: the redacted payload's size.
    pub redacted_payload: &'static str,
    pub events: &'static str,
    pub building_preview: &'static str,
    pub cannot_show_title: &'static str,
    pub cannot_show_body: &'static str,
    pub preparing_redactions: &'static str,
    pub redactions_unavailable: &'static str,
    pub removed: &'static str,
    pub nothing_removed: &'static str,
    pub still_present: &'static str,
    pub residual_unavailable: &'static str,
    pub residual_loading: &'static str,
    pub consent_scopes: &'static str,
    pub eligibility_failed: &'static str,
    pub eligibility_checking: &'static str,
    pub outcome_unavailable: &'static str,
    pub outcome_loading: &'static str,
    pub look_inside: &'static str,
    /// Contribute's label when it cannot be pressed.
    pub enroll_to_approve: &'static str,
    pub not_eligible: &'static str,
    pub checking_eligibility: &'static str,
}

/// Look inside (`preview-inspector.tsx`, `native-review-actions.tsx`, and
/// the witness consent line of `witness-review-overlay.tsx`).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorLookInsideCopy {
    pub eyebrow: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub would_send: &'static str,
    pub on_disk: &'static str,
    pub load_transcript: &'static str,
    pub loading_transcript: &'static str,
    pub search_original: &'static str,
    pub turn_index: &'static str,
    pub transcript_caption: &'static str,
    /// `{size}`: what is left to load.
    pub load_more: &'static str,
    pub add_turn_separators: &'static str,
    pub search_caption: &'static str,
    pub search_label: &'static str,
    pub search_placeholder: &'static str,
    pub check_count: &'static str,
    pub original_match_one: &'static str,
    pub original_matches: &'static str,
    pub turns_need_full_read: &'static str,
    pub load_turn_index: &'static str,
    pub turn_index_eyebrow: &'static str,
    /// A turn that names no tool.
    pub turn_event: &'static str,
    /// `{start}` and `{end}`: the turn's byte range.
    pub turn_bytes: &'static str,
    pub close: &'static str,
    pub native_review: &'static str,
    pub native_review_caption: &'static str,
    pub prepare_admission: &'static str,
    pub request_witness_review: &'static str,
    /// The witness consent checkbox's line and accessible name.
    pub witness_confirm_line: &'static str,
    pub witness_confirm_label: &'static str,
    pub witness_reviewing: &'static str,
}

/// The undo bar (`undo-bar.tsx`). Undo itself is
/// [`MonitorTracesCopy::undo_contribute`].
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorUndoCopy {
    pub approval_saved: &'static str,
    /// `{label}`: what was approved.
    pub approved: &'static str,
    pub unavailable: &'static str,
    pub within: &'static str,
    pub may_have_started: &'static str,
    pub undoing: &'static str,
}

/// The queue's safeguards panel (`queue-status-panel.tsx`): the labels
/// `HealthCopy`, `DailyBudgetCopy` and `routing_copy` do not already hold.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorSafeguardsCopy {
    pub eyebrow: &'static str,
    pub heading: &'static str,
    pub daily_limit: &'static str,
    pub inference_routing: &'static str,
    pub daemon_owned: &'static str,
    pub rows_unavailable_one: &'static str,
    pub rows_unavailable: &'static str,
    /// The daily limit row: uploads and megabytes left today. The shell
    /// fills both numbers and adds no unit.
    pub remaining: &'static str,
    /// Approved sessions the spent limit holds back: one, then counted.
    pub held_by_limit_one: &'static str,
    pub held_by_limit: &'static str,
    /// `witness_capacity` was reported but could not be read: never "none
    /// waiting".
    pub capacity_unreadable: &'static str,
}

/// History's refresh and account sign-in controls: the controls Ron's
/// #1146 `history-refresh-control.tsx` and `account-sign-in-control.tsx`
/// draw, in his words (owner ruling, 2026-10-06: #1146's wording wins).
/// The two outcome lines and the three sign-in results #1146 has no
/// sentence for (it shows the raw error) stay native, and name no
/// machinery.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorHistoryActionsCopy {
    /// Asks the daemon to check the server sooner (`refresh_history`).
    pub request_refresh: &'static str,
    pub requesting: &'static str,
    pub refresh_requested: &'static str,
    pub refresh_failed: &'static str,
    /// The account session has not been read yet.
    pub checking_account: &'static str,
    /// In Withdraw's place on a row while no account session is active.
    pub sign_in_to_withdraw: &'static str,
    pub waiting_for_sign_in: &'static str,
    pub complete_sign_in: &'static str,
    /// Sign-in returned, and the re-read session is not active.
    pub sign_in_inactive: &'static str,
    /// Sign-in returned, and the session could not be re-read.
    pub sign_in_unverified: &'static str,
    pub sign_in_failed: &'static str,
}

/// The one table of the monitor's Traces words. See [`MonitorTracesCopy`].
#[must_use]
pub fn monitor_traces_copy() -> MonitorTracesCopy {
    let customize = crate::project_copy::customize_copy();
    MonitorTracesCopy {
        review: "Review",
        tool: "Tool",
        folder: "Folder",
        started: "Started",
        length: "Length",
        prompts: "Prompts",
        size: "Size",
        sends: "Sends",
        marks: "Marks",
        unsure: "Unsure",
        eligibility: "Eligibility",
        attestation: "Attestation",
        held: "Held",
        sample: "Sample",
        residual_risk: "Residual risk",
        personal_information: "Personal information",
        second_look_waiting: "some worth a second look",
        contribute: "Contribute",
        keep: customize.keep,
        dismiss: "Not this one",
        undo_contribute: "Undo",
        undo_keep: customize.undo_keep,
        core_unreachable: MONITOR_CORE_UNREACHABLE,
        request_failed: MONITOR_REQUEST_FAILED,
        dismiss_action: "Dismiss",
        optional_automation: "OPTIONAL AUTOMATION",
        tree: MonitorTreeCopy {
            tree_label: "Tools, folders and sessions",
            reading_queue: "Reading local queue\u{2026}",
            submit: "Submit",
            submit_count: "Submit \u{00b7} {count}",
            submitting: "Submitting\u{2026}",
            submit_tip: "Sends every eligible waiting session here.",
            eligible_count: "{count} eligible",
            reviewing: "Reviewing",
            review_tip: "Opens what would be sent. Nothing leaves until you contribute.",
            ignored_folder: "ignored \u{00b7} never queued",
            ignore_folder: "Ignore folder / repo",
            watch_folder: "Watch this folder",
            watch_tool: "Watch {tool}",
            dismiss_session: "Dismiss session",
            dismiss_session_title: "Dismiss this session?",
            dismiss_session_body: "{when} \u{00b7} {size}. Dismissing removes it from the sessions \
                waiting for you, without sending it.",
            dismiss_session_keep: "Keep it",
            dismissing: "Dismissing\u{2026}",
            dismiss_session_failed: "Could not dismiss session.",
        },
        counts: MonitorCountsCopy {
            sessions_waiting_one: "1 session waiting",
            sessions_waiting: "{count} sessions waiting",
            waiting_count: "{count} waiting",
            contributed_count: "{count} contributed",
            project_count_one: "1 project",
            project_count: "{count} projects",
        },
        inspector: MonitorInspectorCopy {
            decisions: "Decisions",
            not_set: "Not set",
            sessions_folder: "Sessions folder",
            project: "Project",
            project_of: "Project \u{00b7} {tool}",
            session_of: "Session \u{00b7} {tool}",
            path: "Path",
            contribution_rule: "Contribution rule",
            no_rule: "This folder has no rule of its own yet.",
            apply_outcome_one: "Apply one outcome to 1 eligible session.",
            apply_outcome: "Apply one outcome to {count} eligible sessions.",
            cancel: "Cancel",
            waiting_sessions_one: "1 waiting session",
            waiting_sessions: "{count} waiting sessions",
            submit_all_eligible: "Submit all eligible ({count})",
        },
        summary_panel: MonitorSummaryCopy {
            tools_watched: "{count} of {total} tools watched",
            waiting_for_you: "{count} waiting for you",
            worth_a_second_look: "{count} worth a second look",
            uploads_today: "{count} of {max} uploads today",
            statistics: "Statistics",
            top_projects: "Top projects",
            top_tools: "Top tools",
            no_longer_waiting: "Sessions no longer waiting ({count})",
            no_longer_waiting_scope: "This covers sessions that reached the queue. \
                Sessions never queued are not counted here.",
        },
        session_review: MonitorSessionReviewCopy {
            eyebrow: "LOCAL PREVIEW",
            heading: "What would leave this computer",
            enrolled: "Enrolled",
            not_enrolled: "Not enrolled",
            no_opening_prompt: "No opening prompt",
            redacted_payload: "{size} redacted payload",
            events: "{count} events",
            building_preview: "Building local privacy preview\u{2026}",
            cannot_show_title: "This one can't be shown.",
            cannot_show_body: "The session file changed while it was being read. Nothing has been \
                sent, and nothing will be until it can be shown to you.",
            preparing_redactions: "Preparing redaction summary\u{2026}",
            redactions_unavailable: "Redaction summary unavailable. Contribution is disabled.",
            removed: "Removed",
            nothing_removed: "Nothing matched for removal.",
            still_present: "Found, and still in what would be sent",
            residual_unavailable: "Residual secret warning unavailable. Contribution is disabled.",
            residual_loading: "Loading residual secret warning\u{2026}",
            consent_scopes: "Consent scopes",
            eligibility_failed: "Contribution eligibility could not be checked. Approval is disabled.",
            eligibility_checking: "Checking contribution eligibility\u{2026}",
            outcome_unavailable: "Outcome and correction disclosure unavailable. \
                Contribution is disabled.",
            outcome_loading: "Loading outcome and correction disclosure\u{2026}",
            look_inside: "Look inside",
            enroll_to_approve: "Enroll to approve",
            not_eligible: "Not eligible",
            checking_eligibility: "Checking eligibility\u{2026}",
        },
        look_inside: MonitorLookInsideCopy {
            eyebrow: "LOOK INSIDE",
            title: "Exactly what would be sent",
            description: "This is the redacted envelope. It stays local while you read it. \
                Original-session search returns only a count; it never returns raw text.",
            would_send: "{size} would send",
            on_disk: "{size} on disk",
            load_transcript: "Load redacted transcript",
            loading_transcript: "Loading bounded transcript\u{2026}",
            search_original: "Search original",
            turn_index: "Turn index",
            transcript_caption: "These are the exact redacted bytes an approval covers. \
                Markers show where local scrubbing fired.",
            load_more: "Load more ({size} remaining)",
            add_turn_separators: "Add turn separators",
            search_caption: "Search checks the original session locally and returns a count \
                only. It never renders original text.",
            search_label: "Search original session",
            search_placeholder: "Client, hostname, token label\u{2026}",
            check_count: "Check count",
            original_match_one: "1 original match",
            original_matches: "{count} original matches",
            turns_need_full_read: "Read transcript fully before loading turn index.",
            load_turn_index: "Load turn index",
            turn_index_eyebrow: "TURN INDEX",
            turn_event: "event",
            turn_bytes: "bytes {start}\u{2013}{end}",
            close: "Close",
            native_review: "NATIVE REVIEW",
            native_review_caption: "Optional daemon-backed checks stay local until you \
                explicitly confirm.",
            prepare_admission: "Prepare admission",
            request_witness_review: "Request witness review",
            witness_confirm_line: "I understand and want to send this session for review.",
            witness_confirm_label: "Confirm sending unredacted session to witness",
            witness_reviewing: "Reviewing\u{2026}",
        },
        undo: MonitorUndoCopy {
            approval_saved: "APPROVAL SAVED",
            approved: "{label} approved",
            unavailable: "Undo unavailable",
            within: "Undo within {seconds}s before upload starts.",
            may_have_started: "Upload may already have started.",
            undoing: "Undoing\u{2026}",
        },
    }
}

/// The Traces badge's text equivalent, for assistive tech: the badge is a
/// bare number, or a dash when the count is unknown.
///
/// `None` (the daemon did not report `decisions_owed`) is "unavailable",
/// never zero. Zero is the empty string: no badge, nothing to say.
#[must_use]
pub fn decisions_owed_text(decisions_owed: Option<u64>) -> String {
    match decisions_owed {
        None => "Decisions owed unavailable".to_string(),
        Some(0) => String::new(),
        Some(1) => "1 decision owed".to_string(),
        Some(n) => format!("{n} decisions owed"),
    }
}

// ---------------------------------------------------------------------------
// The glass monitor's other screens (#1173 R8-R13): the map, the Inference
// tab, Home and History, Missions, and the menu-bar popover. One table, as
// for the Traces tab, so no shell writes these words.

/// Every fixed word of the monitor's screens after Traces. Single words and
/// short labels; the screens' sentences come from their own copy tables.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorScreensCopy {
    /// The map's hub: this computer.
    pub computer: &'static str,
    /// The map's library node.
    pub commons: &'static str,
    /// A count of sessions waiting.
    pub waiting: &'static str,
    /// A count of folders.
    pub folders: &'static str,
    /// A tool that is watched.
    pub watched: &'static str,
    /// Off: a tool, or Private AI.
    pub off: &'static str,
    /// On: watching, or Private AI.
    pub on: &'static str,
    /// A count of tools connected to Private AI.
    pub connected: &'static str,
    /// The map's zoom out.
    pub reduce: &'static str,
    /// The map's zoom in.
    pub enlarge: &'static str,
    /// Model calls.
    pub calls: &'static str,
    /// The per-model summary.
    pub models: &'static str,
    /// What calls were priced at (never billed).
    pub priced: &'static str,
    /// A value the core did not report.
    pub unknown: &'static str,
    /// IronWire's proof label `verified`: the only one that is proof.
    pub proof_verified: &'static str,
    /// `gateway_only`.
    pub proof_gateway_only: &'static str,
    /// `unattested`.
    pub proof_unattested: &'static str,
    /// `pending`.
    pub proof_pending: &'static str,
    /// `unavailable`.
    pub proof_unavailable: &'static str,
    /// `failed`, kept apart from the rest.
    pub proof_failed: &'static str,
    /// `outside`.
    pub proof_outside: &'static str,
    /// `unrecorded`, and any label a later daemon grows.
    pub proof_unrecorded: &'static str,
    /// Home's History.
    pub history: &'static str,
    /// A count of contributions accepted.
    pub contributed: &'static str,
    /// Watching N tools; the Watching pill.
    pub watching: &'static str,
    /// Watching is paused.
    pub paused: &'static str,
    /// The inspector's summary on Home.
    pub summary: &'static str,
    /// This week.
    pub week: &'static str,
    /// This month.
    pub month: &'static str,
    /// All time.
    pub total: &'static str,
    /// Held for privacy review: never rejected.
    pub held: &'static str,
    /// Taken back.
    pub withdrawn: &'static str,
    /// Credit.
    pub credit: &'static str,
    /// Credit settled.
    pub credit_final: &'static str,
    /// Credit not yet settled; shown only with its condition.
    pub pending: &'static str,
    /// Community standing.
    pub community: &'static str,
    /// Rank in the community.
    pub rank: &'static str,
    /// The standing's window.
    pub window: &'static str,
    /// A contribution the person approved.
    pub approved: &'static str,
    /// How a contribution was approved was not recorded; never said as approved.
    pub unrecorded: &'static str,
    /// Home's Missions.
    pub missions: &'static str,
    /// The menu-bar mode pill.
    pub contribution_mode: &'static str,
    /// Folders whose modes differ.
    pub mixed: &'static str,
    /// The graph's contributed series.
    pub shared: &'static str,
    /// The graph's kept series.
    pub kept: &'static str,
    /// The menu-bar popover's recent rows.
    pub recent_activity: &'static str,
    /// Sessions worth a second look.
    pub flagged: &'static str,
    /// Opens the folders' rules.
    pub manage_rules: &'static str,
    /// Opens Settings.
    pub settings: &'static str,
    /// The Settings modal's title (Ron's #1146 `SettingsModal`), over the
    /// Monitor.
    pub settings_title: &'static str,
    /// The Settings modal's subtitle.
    pub settings_subtitle: &'static str,
    /// The name of the Settings modal's section list.
    pub settings_sections: &'static str,
    /// Closes a modal.
    pub close: &'static str,
    /// The Monitor toolbar's View menu (Ron's native shell after #1146's
    /// `monitor-toolbar.tsx`).
    pub view: &'static str,
    /// The toolbar's toggle for the Traces graph.
    pub graph: &'static str,
    /// The View menu item that draws folders set to ignore.
    pub show_ignored_folders: &'static str,
    /// The Traces graph's button that focuses the map on one tool.
    pub focus: &'static str,
    /// The Traces graph's step to the previous period.
    pub previous: &'static str,
    /// The Traces graph's step to the next period.
    pub next: &'static str,
    /// Home's pending-credit count (shown only beside the commons'
    /// statement of what it waits on, D6).
    pub credit_pending: &'static str,
    /// Home's status-card link to the Traces tab.
    pub open_traces: &'static str,
    /// Quits the app.
    pub quit: &'static str,
    /// The core did not answer; see [`MONITOR_CORE_UNREACHABLE`].
    pub core_unreachable: &'static str,
    /// A request failed; see [`MONITOR_REQUEST_FAILED`].
    pub request_failed: &'static str,
    /// Held for privacy review, in full: the inspector's label for the
    /// held count. Never rejected.
    pub held_for_review: &'static str,
    /// What held for privacy review means, beside the held count: the
    /// shipping History's sentence, moved here.
    pub held_explanation: &'static str,
    /// Beside every credit figure, pending included: credit is a record,
    /// not currency. The shipping credit view's sentence, moved here.
    pub credit_not_currency: &'static str,
    /// History reads a page of rows; when the page
    /// is full, how many of the total it shows. `{shown}` and `{total}`
    /// are replaced with numbers.
    pub history_shown_of: &'static str,
    /// As `history_shown_of`, when the total is
    /// not known. `{shown}` is replaced with a number.
    pub history_shown: &'static str,
    /// Home's watching row when the core says
    /// the contributor is not signed in: nothing is contributed.
    pub signed_out: &'static str,
    /// A mission's credit range: projected credit,
    /// labelled as such (owner ruling, 2026-10-02). Never `pending`, which
    /// is submitted credit still being scored: a contribution mission is
    /// apart from the reward ledger (#1174).
    pub projected: &'static str,
    /// Beside projected mission credit: what it is,
    /// and that it is not yet earned.
    pub projected_note: &'static str,
    /// Approved 2026-10-06. The window the Inference tab's counts
    /// cover, from `window_hours` on `inference_calls` and
    /// `tool_destinations`. `{hours}` is replaced with a number.
    pub window_last_hours: &'static str,
    /// History's word for a contribution whose
    /// status is `submitted`: sent, and not yet scored. The shipping
    /// History's sentence, moved here; never "Submitted", which reads as
    /// done.
    pub history_submitted: &'static str,
    /// The queue's safeguards panel (Ron's #1146 `QueueStatusPanel`).
    pub safeguards: MonitorSafeguardsCopy,
    /// History's refresh and account sign-in controls (#1146).
    pub history_actions: MonitorHistoryActionsCopy,
    /// Ron's #1146 shell, Home and History words (owner ruling,
    /// 2026-10-06): the toolbar toggles, the graph's focus tips, Home's
    /// status lines and History's filters and empty states.
    pub shell: MonitorShellCopy,
}

/// Ron's #1146 words for the monitor's toolbar, the Traces graph's focus
/// button, Home and History (`monitor-toolbar.tsx`, `monitor-shell.tsx`,
/// `home-view.tsx`, `history-filter.tsx`, `history-page.tsx`,
/// `history-row.tsx`). Where Ron pluralised with a helper, the singular is
/// its own line.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorShellCopy {
    /// The toolbar's graph, flow map and inspector toggles, by state: each
    /// names what pressing it does.
    pub show_graph: &'static str,
    pub hide_graph: &'static str,
    pub show_map: &'static str,
    pub hide_map: &'static str,
    pub show_inspector: &'static str,
    pub hide_inspector: &'static str,
    /// The graph's focus button: nothing selected, focused, and `{tool}`
    /// the selected session's tool.
    pub focus_needs_selection: &'static str,
    pub focus_whole_map: &'static str,
    pub focus_tool: &'static str,
    /// Home's status card: watching N tools.
    pub watching_tools_one: &'static str,
    pub watching_tools: &'static str,
    /// Home's status card's second line.
    pub waiting_for_you_one: &'static str,
    pub waiting_for_you: &'static str,
    /// Joined to the line above with a middle dot.
    pub worth_a_second_look: &'static str,
    pub nothing_waiting: &'static str,
    /// Home's History card, before anything was contributed.
    pub nothing_contributed: &'static str,
    /// Home's History card accessory: `{amount}` is the pending credit.
    pub credit_pending_amount: &'static str,
    /// History's filters, in Ron's order. `submitted` is History's status
    /// word for it (`history_copy::WAITING_TO_BE_SCORED`).
    pub filter_all: &'static str,
    pub filter_accepted: &'static str,
    pub filter_submitted: &'static str,
    pub filter_quarantined: &'static str,
    pub filter_withdrawn: &'static str,
    /// History's list, empty, and empty under a filter.
    pub history_empty: &'static str,
    pub history_filter_empty: &'static str,
    /// A History row's way into its details.
    pub open: &'static str,
    /// The menu's pause and resume (#1146 `tray.rs`), and the pause
    /// lengths. "Until tomorrow morning" is native's own: #1146 has no
    /// counterpart.
    pub pause_watcher: &'static str,
    pub resume_watcher: &'static str,
    pub pause_hour: &'static str,
    pub pause_morning: &'static str,
    pub pause_until_resumed: &'static str,
    /// Settings: the login switch, the projects list before discovery, and
    /// the change log's heading (#1146 `platform-panel.tsx`,
    /// `projects-panel.tsx`, `sections.ts`).
    pub start_at_login: &'static str,
    pub projects_empty: &'static str,
    pub changes_heading: &'static str,
    /// The monitor's tabs, the tab strip's accessible name, and the map's
    /// view selector's accessible name: words macOS held in Swift until
    /// 2026-10-06.
    pub tab_home: &'static str,
    pub tab_inference: &'static str,
    pub tab_traces: &'static str,
    pub tabs_label: &'static str,
    pub map_views_label: &'static str,
}

/// The one table of the monitor screens' words. See [`MonitorScreensCopy`].
#[must_use]
pub fn monitor_screens_copy() -> MonitorScreensCopy {
    MonitorScreensCopy {
        computer: "This computer",
        commons: "Library \u{00b7} commons",
        waiting: "Waiting",
        folders: "Folders",
        watched: "Watched",
        off: "Off",
        on: "On",
        connected: "Connected",
        reduce: "Zoom map out",
        enlarge: "Zoom map in",
        calls: "Calls",
        models: "Models",
        priced: "Priced",
        unknown: "Unknown",
        proof_verified: "Verified",
        proof_gateway_only: "Gateway",
        proof_unattested: "Unattested",
        proof_pending: "Pending",
        proof_unavailable: "Unavailable",
        proof_failed: "Failed",
        proof_outside: "Outside",
        proof_unrecorded: "Unrecorded",
        history: "History",
        contributed: "Contributed",
        watching: "Watching",
        paused: "Paused",
        summary: "Summary",
        week: "Week",
        month: "Month",
        total: "Total",
        held: "Held",
        withdrawn: "Withdrawn",
        credit: "Credit",
        credit_final: "Final",
        pending: "Pending",
        community: "Community",
        rank: "Rank",
        window: "Window",
        approved: "Approved",
        unrecorded: "Unrecorded",
        missions: "Missions",
        contribution_mode: "Contribution mode",
        mixed: "Mixed",
        shared: "shared",
        kept: "kept",
        recent_activity: "Recent activity",
        flagged: SECOND_LOOK_HEADING,
        manage_rules: "Manage rules…",
        settings: "Trace Commons Settings…",
        settings_title: "Settings",
        settings_subtitle: "What this machine watches, and what your traces are allowed to do.",
        settings_sections: "Settings sections",
        close: "Close",
        view: "View options",
        graph: "Graph",
        show_ignored_folders: "Show ignored folders",
        focus: "Focus",
        previous: "Previous period",
        next: "Next period",
        credit_pending: "Credit pending",
        open_traces: "Open Traces",
        quit: "Quit Trace Commons",
        core_unreachable: MONITOR_CORE_UNREACHABLE,
        request_failed: MONITOR_REQUEST_FAILED,
        held_for_review: crate::history_copy::HELD_FOR_PRIVACY_REVIEW,
        held_explanation: crate::history_copy::HELD_ROW_BODY,
        credit_not_currency: "A credit is a signed record that a contribution was accepted. It is not currency.",
        history_shown_of: "Showing the newest {shown} of {total}",
        history_shown: "Showing the newest {shown}",
        signed_out: "Not signed in",
        projected: "Projected",
        projected_note: "Projected credit is an estimate for a contribution that matches a mission. \
            It is not earned until a contribution is accepted and scored.",
        window_last_hours: "Last {hours} hours",
        history_submitted: crate::history_copy::WAITING_TO_BE_SCORED,
        safeguards: MonitorSafeguardsCopy {
            eyebrow: "RUNTIME",
            heading: "Contribution safeguards",
            daily_limit: "Daily limit",
            inference_routing: "Inference routing",
            daemon_owned: "daemon-owned",
            rows_unavailable_one: "1 row unavailable",
            rows_unavailable: "{count} rows unavailable",
            remaining: "{uploads} uploads left \u{00b7} {megabytes} MB left",
            held_by_limit_one: "1 queued session held by limit",
            held_by_limit: "{count} queued sessions held by limit",
            capacity_unreadable: "Some approved sessions may be waiting and have not been sent, \
                but this build could not read how many or why.",
        },
        history_actions: MonitorHistoryActionsCopy {
            request_refresh: "Request server refresh",
            requesting: "Requesting\u{2026}",
            refresh_requested: "Asked Trace Commons for the latest results. Changes show here when they arrive.",
            refresh_failed: "Could not ask for updates. Nothing changed; try again.",
            checking_account: "Checking account session\u{2026}",
            sign_in_to_withdraw: "Sign in to withdraw",
            waiting_for_sign_in: "Waiting for sign-in\u{2026}",
            complete_sign_in: "Complete sign-in in your browser. This may take up to five minutes.",
            sign_in_inactive: "Sign-in finished, but this device is still signed out of your \
                Trace Commons account. Sign in again to withdraw.",
            sign_in_unverified: "Sign-in finished, but your account could not be checked. \
                Sign in again before withdrawing.",
            sign_in_failed: "Sign-in did not finish. Nothing was withdrawn; sign in again to withdraw.",
        },
        shell: MonitorShellCopy {
            show_graph: "Show the graph",
            hide_graph: "Hide the graph",
            show_map: "Show the flow map",
            hide_map: "Hide the flow map",
            show_inspector: "Show the inspector",
            hide_inspector: "Hide the inspector",
            focus_needs_selection: "Select a tool, project or session first",
            focus_whole_map: "Back to the whole map",
            focus_tool: "Show {tool} in the map",
            watching_tools_one: "Watching 1 tool",
            watching_tools: "Watching {count} tools",
            waiting_for_you_one: "1 session waiting for you",
            waiting_for_you: "{count} sessions waiting for you",
            worth_a_second_look: "{count} worth a second look",
            nothing_waiting: "Nothing waiting for you",
            nothing_contributed: "Nothing contributed from this machine yet.",
            credit_pending_amount: "{amount} credit pending",
            filter_all: "All",
            filter_accepted: "In commons",
            filter_submitted: crate::history_copy::WAITING_TO_BE_SCORED,
            filter_quarantined: "Privacy review",
            filter_withdrawn: "Withdrawn",
            history_empty: "No submissions recorded on this device yet.",
            history_filter_empty: "No submissions match this filter.",
            open: "Open",
            pause_watcher: "Pause watcher",
            resume_watcher: "Resume watcher",
            pause_hour: "For 1 hour",
            pause_morning: "Until tomorrow morning",
            pause_until_resumed: "Until I turn it back on",
            start_at_login: "Start Trace Commons at login",
            projects_empty: "No projects seen yet. Sessions appear here after discovery.",
            changes_heading: "Changes on this machine",
            tab_home: "Home",
            tab_inference: "Inference",
            tab_traces: "Traces",
            tabs_label: "Monitor",
            map_views_label: "Map",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string leaf of a copy table, by its path.
    fn leaves(value: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
        match value {
            serde_json::Value::String(word) => out.push((path.to_owned(), word.clone())),
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    leaves(value, &format!("{path}/{key}"), out);
                }
            }
            other => panic!("{path} is neither a word nor a table: {other}"),
        }
    }

    fn words_of<T: serde::Serialize>(copy: &T) -> Vec<(String, String)> {
        let mut out = Vec::new();
        leaves(&serde_json::to_value(copy).unwrap(), "", &mut out);
        out
    }

    #[test]
    fn the_monitor_screens_copy_is_whole_and_shares_the_traces_lines() {
        let copy = monitor_screens_copy();
        for (key, word) in words_of(&copy) {
            assert!(!word.is_empty(), "{key} is empty");
        }
        let traces = monitor_traces_copy();
        assert_eq!(copy.core_unreachable, traces.core_unreachable);
        assert_eq!(copy.request_failed, traces.request_failed);
        // Not recorded is never said as approved.
        assert_ne!(copy.unrecorded, copy.approved);
        // History's cap lines carry their numbers' places.
        assert!(
            copy.history_shown_of.contains("{shown}") && copy.history_shown_of.contains("{total}")
        );
        assert!(copy.history_shown.contains("{shown}") && !copy.history_shown.contains("{total}"));
        // The Inference tab's window carries its number's place.
        assert!(copy.window_last_hours.contains("{hours}"));
        // A submission is said as waiting, never as done.
        assert_ne!(copy.history_submitted, "Submitted");
        // One held-for-review explanation (owner ruling, 2026-10-02): the
        // monitor and History read the same sentence.
        assert_eq!(copy.held_explanation, crate::history_copy::HELD_ROW_BODY);
        // Projected mission credit is never said as pending.
        assert_ne!(copy.projected, copy.pending);
        assert!(copy.projected_note.contains("not earned"));
        // Held is said in full, and never as rejected.
        assert!(copy.held_explanation.contains("not been rejected"));
        // Ron's #1146 settings modal (#1241 Task 10): its title is not the
        // menu item that opens it, and its subtitle is a sentence.
        assert_eq!(copy.settings_title, "Settings");
        assert_ne!(copy.settings_title, copy.settings);
        assert!(copy.settings_subtitle.ends_with('.'));
        assert!(!copy.settings_sections.is_empty() && !copy.close.is_empty());
        // Ron's native shell words: the toolbar, the Traces graph and Home.
        for word in [
            copy.view,
            copy.graph,
            copy.show_ignored_folders,
            copy.focus,
            copy.previous,
            copy.next,
            copy.credit_pending,
            copy.open_traces,
        ] {
            assert!(!word.is_empty() && !word.ends_with('.'), "{word}");
        }
    }

    #[test]
    fn the_monitor_traces_copy_is_whole_and_shares_customize_words() {
        let copy = monitor_traces_copy();
        for (key, word) in words_of(&copy) {
            assert!(!word.is_empty(), "{key} is empty");
        }
        let customize = crate::project_copy::customize_copy();
        assert_eq!(copy.keep, customize.keep);
        assert_eq!(copy.undo_keep, customize.undo_keep);
        assert_eq!(copy.core_unreachable, MONITOR_CORE_UNREACHABLE);
    }

    /// Ron's #1146 inspector words (#1241): every one is there, and none of
    /// them offers "Share automatically" -- the folder rule is Ask me /
    /// Automatic / Never (`project_copy::contribution_mode_copy`).
    #[test]
    fn rons_inspector_words_are_whole_and_never_say_share_automatically() {
        let mut words = words_of(&monitor_traces_copy());
        words.extend(words_of(&monitor_screens_copy()));
        for (key, word) in &words {
            assert!(!word.trim().is_empty(), "{key} is empty");
            assert!(!word.contains("Share automatically"), "{key}: {word}");
        }
        for table in [
            "/tree/",
            "/counts/",
            "/inspector/",
            "/summary_panel/",
            "/session_review/",
            "/look_inside/",
            "/undo/",
            "/safeguards/",
            "/history_actions/",
            "/shell/",
        ] {
            assert!(
                words.iter().any(|(key, _)| key.starts_with(table)),
                "{table} is missing"
            );
        }
    }

    /// Ron's lines verbatim, with his numbers as `{name}` holes.
    #[test]
    fn rons_counted_lines_hold_their_numbers() {
        let traces = monitor_traces_copy();
        assert_eq!(traces.tree.submit_count, "Submit \u{00b7} {count}");
        // `waiting-project-folder.tsx`'s Decisions card.
        assert_eq!(
            traces.inspector.submit_all_eligible,
            "Submit all eligible ({count})"
        );
        assert_eq!(
            traces.inspector.waiting_sessions,
            "{count} waiting sessions"
        );
        assert_eq!(traces.inspector.waiting_sessions_one, "1 waiting session");
        // `queue-status-panel.tsx`'s daily limit row and its held line.
        assert_eq!(
            monitor_screens_copy().safeguards.remaining,
            "{uploads} uploads left \u{00b7} {megabytes} MB left"
        );
        assert_eq!(
            monitor_screens_copy().safeguards.held_by_limit,
            "{count} queued sessions held by limit"
        );
        assert_eq!(traces.optional_automation, "OPTIONAL AUTOMATION");
        assert_eq!(
            traces.inspector.apply_outcome,
            "Apply one outcome to {count} eligible sessions."
        );
        assert_eq!(
            traces.inspector.no_rule,
            "This folder has no rule of its own yet."
        );
        assert_eq!(
            traces.undo.within,
            "Undo within {seconds}s before upload starts."
        );
        assert_eq!(traces.undo.approved, "{label} approved");
        assert_eq!(
            traces.look_inside.witness_confirm_line,
            "I understand and want to send this session for review."
        );
        assert!(traces.summary_panel.no_longer_waiting.contains("{count}"));
        assert_eq!(
            traces.summary_panel.no_longer_waiting_scope,
            "This covers sessions that reached the queue. Sessions never queued are not counted here."
        );
        // A singular is its own line, never a plural with a 1 in it.
        assert!(!traces.counts.sessions_waiting_one.contains('{'));
        assert!(!traces.counts.project_count_one.contains('{'));
    }

    /// History's refresh and sign-in controls are Ron's #1146 words (owner
    /// ruling, 2026-10-06). The lines #1146 has no sentence for -- the
    /// refresh outcome and the sign-in results, where it shows raw error
    /// text -- stay native and name no machinery.
    #[test]
    fn history_actions_speak_in_rons_words() {
        let copy = monitor_screens_copy().history_actions;
        assert_eq!(words_of(&copy).len(), 11, "every history action is read");
        assert_eq!(copy.request_refresh, "Request server refresh");
        assert_eq!(copy.requesting, "Requesting\u{2026}");
        assert_eq!(copy.checking_account, "Checking account session\u{2026}");
        assert_eq!(copy.sign_in_to_withdraw, "Sign in to withdraw");
        assert_eq!(copy.waiting_for_sign_in, "Waiting for sign-in\u{2026}");
        assert_eq!(
            copy.complete_sign_in,
            "Complete sign-in in your browser. This may take up to five minutes."
        );
        for (key, word) in [
            ("refresh_requested", copy.refresh_requested),
            ("refresh_failed", copy.refresh_failed),
            ("sign_in_inactive", copy.sign_in_inactive),
            ("sign_in_unverified", copy.sign_in_unverified),
            ("sign_in_failed", copy.sign_in_failed),
        ] {
            assert!(!word.is_empty(), "{key} is empty");
            let lower = word.to_lowercase();
            for machinery in ["daemon", "core", "server", "asynchronous"] {
                assert!(!lower.contains(machinery), "{key} says {machinery}: {word}");
            }
        }
    }

    /// Ron's #1146 shell words, verbatim, with his numbers as holes and a
    /// singular of its own wherever he pluralised.
    #[test]
    fn the_shell_words_are_rons() {
        let screens = monitor_screens_copy();
        let shell = &screens.shell;
        for (key, word) in words_of(shell) {
            assert!(!word.is_empty(), "{key} is empty");
        }
        assert_eq!(screens.view, "View options");
        assert_eq!(screens.previous, "Previous period");
        assert_eq!(screens.next, "Next period");
        assert_eq!(screens.computer, "This computer");
        assert_eq!(shell.show_map, "Show the flow map");
        assert!(shell.focus_tool.contains("{tool}"));
        assert!(shell.watching_tools.contains("{count}"));
        assert!(!shell.watching_tools_one.contains('{'));
        assert!(!shell.waiting_for_you_one.contains('{'));
        assert!(shell.credit_pending_amount.contains("{amount}"));
        // The submitted filter is History's own status word.
        assert_eq!(
            shell.filter_submitted,
            crate::history_copy::WAITING_TO_BE_SCORED
        );
        // Withdrawn is a filter word, never "by you": it also gathers
        // withdrawals this machine did not make.
        assert!(!shell.filter_withdrawn.contains("by you"));
    }

    /// Every `{...}` in the monitor tables is one [`MONITOR_PLACEHOLDERS`]
    /// lists, and every one listed is used, so a shell knows each hole it
    /// fills.
    #[test]
    fn every_monitor_placeholder_is_a_documented_one() {
        let mut words = words_of(&monitor_traces_copy());
        words.extend(words_of(&monitor_screens_copy()));
        let mut unknown = Vec::new();
        for (key, word) in &words {
            let mut rest = word.as_str();
            while let Some(open) = rest.find('{') {
                rest = &rest[open + 1..];
                let Some(close) = rest.find('}') else { break };
                let name = &rest[..close];
                if !MONITOR_PLACEHOLDERS.contains(&name) {
                    unknown.push(format!("{key}: {{{name}}}"));
                }
            }
        }
        assert!(unknown.is_empty(), "undocumented placeholders: {unknown:?}");
        for name in MONITOR_PLACEHOLDERS {
            assert!(
                words
                    .iter()
                    .any(|(_, word)| word.contains(&format!("{{{name}}}"))),
                "{{{name}}} is documented but no string carries it"
            );
        }
    }

    #[test]
    fn an_unknown_decision_count_is_never_zero() {
        assert_eq!(decisions_owed_text(None), "Decisions owed unavailable");
        assert_eq!(decisions_owed_text(Some(0)), "");
        assert_eq!(decisions_owed_text(Some(1)), "1 decision owed");
        assert_eq!(decisions_owed_text(Some(120)), "120 decisions owed");
        assert_ne!(decisions_owed_text(None), decisions_owed_text(Some(0)));
    }

    #[test]
    fn every_flow2_label_has_words_and_unscrubbed_is_not_zero() {
        for reason in crate::daemon::second_look::SECOND_LOOK_REASONS {
            assert!(second_look_line(reason).is_some(), "{reason}");
        }
        for label in crate::daemon::unsure_spans::UNSURE_LABELS {
            assert!(unsure_hint_line(label).is_some(), "{label}");
        }
        assert_eq!(scrub_state_line(Some(7)), "Scrubbed \u{00b7} 7 marks");
        assert_eq!(scrub_state_line(Some(1)), "Scrubbed \u{00b7} 1 mark");
        assert_eq!(scrub_state_line(None), NOT_YET_SCRUBBED);
        assert_ne!(scrub_state_line(None), scrub_state_line(Some(0)));
    }
}
