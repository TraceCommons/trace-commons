import CTraceCommons
import Foundation
import TCLocalization

/// The copy tables the contributor core assembles for the queue, the
/// preview, Settings, onboarding and withdrawal, read across the C ABI
/// (K1.1, #1173) rather than written here.
///
/// Handle-free for the same reason `TCConsentCopy` is: it describes the
/// build, not a running daemon. The quit prompt is the exception, because
/// its sentence depends on whether this process hosts the watcher, and lives
/// on `TCDaemon`.
///
/// Fixed copy tables are localized by namespace and wire-field key before
/// decoding. Dynamic sentences remain the core's unless it exports a keyed
/// message; arbitrary daemon data is never translated.
public enum TCCoreCopy {
    /// Takes ownership of an ABI string, frees it, and returns it.
    private static func take(_ raw: UnsafeMutablePointer<CChar>?) -> String? {
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// A JSON value for a Swift collection, or nil if it cannot be encoded.
    private static func json<T: Encodable>(_ value: T) -> String? {
        guard let data = try? JSONEncoder().encode(value) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// The core chooses the message; Foundation chooses its plural form.
    /// Detection sites stay out of the text, as in the legacy English ABI.
    public static func residualSecretLine(count: Int, sites: [String]) -> String? {
        let clamped = UInt32(clamping: max(count, 0))
        guard let sitesJSON = json(sites) else {
            return Localization.current.messageJSON(take(tc_residual_secret_message_json(clamped, nil)))
        }
        return Localization.current.messageJSON(take(sitesJSON.withCString { tc_residual_secret_message_json(clamped, $0) }))
    }

    /// `tc_redaction_summary_json`: the preview's removed-summary panel.
    /// Decoded by `TCShellCore.RedactionSummary`.
    public static func redactionSummaryJSON(
        occurrences: [String: Int],
        distinct: [String: Int]
    ) -> String? {
        LocalizedPresentation.json("redaction_summary_json", parameters: [
            "occurrences": occurrences, "distinct": distinct
        ])
    }

    /// `tc_project_ignore_copy_json`: the ignore-project control and its
    /// confirmation. Decoded by `TCShellCore.ProjectIgnoreCopy`.
    public static func projectIgnoreCopyJSON(project: String, pending: Int) -> String? {
        LocalizedPresentation.json("project_ignore_copy_json", parameters: ["project": project, "pending": Int64(pending)])
    }

    /// `tc_project_ignore_reconciled_text`: what is said after an ignore when
    /// the daemon's `purged` differs from the count the confirmation named.
    /// Nil when the two agree, as well as on a NULL from the ABI.
    public static func projectIgnoreReconciled(
        project: String,
        promised: Int,
        purged: Int
    ) -> String? {
        let line = LocalizedPresentation.line("project_ignore_reconciled_text", parameters: [
            "project": project, "promised": Int64(promised), "purged": Int64(purged)
        ])
        guard let line, !line.isEmpty else { return nil }
        return line
    }

    /// `tc_arming_offer_copy_json`: the arming offer and the arming
    /// confirmation. Decoded by `TCShellCore.ProjectArmingCopy`.
    public static func armingOfferCopyJSON(project: String, count: Int) -> String? {
        let clamped = UInt32(clamping: max(count, 0))
        return LocalizedPresentation.json("arming_offer_copy_json", parameters: ["project": project, "count": clamped])
    }

    /// `tc_legacy_migration_offer_json`: the offer to move a legacy invite
    /// identity to a NEAR AI account.
    public static func legacyMigrationOfferJSON() -> String? {
        Localization.current.copyJSON(take(tc_legacy_migration_offer_json()), namespace: "legacy_migration_offer")
    }

    /// `tc_legacy_migration_refusal_text`: the sentence for a refused
    /// `legacy_invite_migrate`, from the IPC error's label.
    public static func legacyMigrationRefusalLine(label: String) -> String? {
        LocalizedPresentation.line("legacy_migration_refusal_text", parameters: ["label": label])
    }

    /// `tc_inference_connection_copy_json`: the connecting-inference step.
    public static func inferenceConnectionCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_inference_connection_copy_json()), namespace: "inference_connection_copy")
    }

    /// `tc_automatic_contribution_copy_json`: the Flow 1 grant screens' words
    /// for the configuration in `configDir`, the disclosure chosen by the
    /// core.
    public static func automaticContributionCopyJSON(configDir: String) -> String? {
        LocalizedPresentation.json("automatic_contribution_copy_json", parameters: ["config_dir": configDir])
    }

    /// `tc_withdrawal_confirmation_prompt_text`: the withdrawal confirmation
    /// for a trace whose reach this machine cannot know.
    public static func withdrawalConfirmationPrompt() -> String? {
        LocalizedPresentation.line("withdrawal_confirmation_prompt_text")
    }

    /// `tc_privacy_scan_copy_json`: the extra privacy scan's words. Decoded
    /// by `TCShellCore.PrivacyScanCopy`.
    public static func privacyScanCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_privacy_scan_copy_json()), namespace: "privacy_scan_copy")
    }

    /// `tc_health_copy_json`: the health banner's words. `reachable` is this
    /// shell's own liveness fact; a nil or empty `label` on a reachable
    /// daemon is nil (nothing to show). A nil `maxQueueEntries` is passed as
    /// -1 (unknown). Decoded by `TCShellCore.HealthLineCopy`.
    public static func healthCopyJSON(reachable: Bool, label: String?, maxQueueEntries: Int?) -> String? {
        let limit = Int64(maxQueueEntries ?? -1)
        return LocalizedPresentation.json("health_copy_json", parameters: [
            "reachable": reachable, "label": label as Any? ?? NSNull(), "max_queue_entries": limit
        ])
    }

    /// `tc_monitor_traces_copy_json`: the monitor's Traces words. Decoded by
    /// `TCShellCore.MonitorTracesCopy`.
    public static func monitorTracesCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_monitor_traces_copy_json()), namespace: "monitor_traces_copy")
    }

    /// `tc_nudge_copy_json`: every fixed re-engagement nudge string, key to
    /// text. Decoded by `TCShellCore.NudgeCopy`. Sentences with counts in
    /// them arrive composed on `status.nudge`, never from here.
    public static func nudgeCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_nudge_copy_json()), namespace: "nudge_copy")
    }

    /// `tc_nudge_entry_tags_json`: a Traces row's tags, worded by the core
    /// from the row's `mission_fit` and `credit_estimate`
    /// (`NudgeEntryTags.input(for:)`). Decoded by
    /// `TCShellCore.NudgeEntryTags`; `{}` for nil or unreadable input.
    public static func nudgeEntryTagsJSON(entryJSON: String?) -> String? {
        LocalizedPresentation.json("nudge_entry_tags_json", inputJSON: entryJSON ?? "{}")
    }

    /// `tc_nudge_digest_help_json`: the digest switch's Settings help,
    /// composed by the core from the schedule and interval
    /// (`NudgeSettings.digestHelpInput(_:)`). Decoded by
    /// `TCShellCore.NudgeSettings.digestHelp(fromJSON:)`; `{}` for nil or
    /// unreadable input.
    public static func nudgeDigestHelpJSON(settingsJSON: String?) -> String? {
        LocalizedPresentation.json("nudge_digest_help_json", inputJSON: settingsJSON ?? "{}")
    }

    /// `tc_first_run_copy_json`: the #1030 first run's words, grouped by
    /// screen. Decoded by `TCShellCore.FirstRunCopy`.
    public static func firstRunCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_first_run_copy_json()), namespace: "first_run_copy")
    }

    /// `tc_first_run_estimate_line`: Traces' one estimate line, worded,
    /// multiplied and formatted by the core from `requestJSON` (`traces`,
    /// `usd_micros_per_accepted_trace`, `credit_range`). Nil draws no line.
    public static func firstRunEstimateLine(requestJSON: String) -> String? {
        LocalizedPresentation.line("first_run_estimate_line", inputJSON: requestJSON)
    }

    /// `tc_install_guides_json`: each coding tool's setup guide, keyed by
    /// discovery slug. Decoded by `TCShellCore.InstallGuides`.
    public static func installGuidesJSON() -> String? {
        take(tc_install_guides_json())
    }

    /// `tc_contribution_mode_copy_json`: the menu-bar Contribution mode
    /// pill's words (#1208). Decoded by `TCShellCore.ContributionModeCopy`.
    public static func contributionModeCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_contribution_mode_copy_json()), namespace: "contribution_mode_copy")
    }

    /// `tc_contribution_override_confirm_json`: one override's confirmation
    /// (`mode` as `set_contribution_override` takes it). For `auto_upload` it
    /// carries the arming disclosure for the configuration in `configDir`,
    /// and is nil without a readable one. Decoded by
    /// `TCShellCore.ContributionOverrideConfirmCopy`.
    public static func contributionOverrideConfirmJSON(mode: String, configDir: String?) -> String? {
        LocalizedPresentation.json("contribution_override_confirm_json", parameters: [
            "mode": mode, "config_dir": configDir as Any? ?? NSNull()
        ])
    }

    /// `tc_contribution_override_refusal_text`: the sentence for a refused
    /// override write, from the IPC error's label.
    public static func contributionOverrideRefusalLine(label: String) -> String? {
        LocalizedPresentation.line("contribution_override_refusal_text", parameters: ["label": label])
    }

    /// `tc_contributor_disclosure_copy_json`: the disclosure bundle. The
    /// monitor decodes its verdict, History and folder-mode tables with
    /// `TCShellCore.ContributorDisclosureCopy`.
    public static func contributorDisclosureCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_contributor_disclosure_copy_json()), namespace: "contributor_disclosure_copy")
    }

    /// `tc_monitor_screens_copy_json`: the monitor's other screens' words.
    /// Decoded by `TCShellCore.MonitorScreensCopy`.
    public static func monitorScreensCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_monitor_screens_copy_json()), namespace: "monitor_screens_copy")
    }

    /// The compiled default for the remotely editable campaign card.
    public static func privateAICampaignJSON() -> String? {
        take(tc_private_ai_campaign_json())
    }

    /// `tc_shell_words_copy_json`: the words this shell used to write in
    /// Swift (withdrawal, the public profile, the legacy queue and History
    /// words, the scrubbing caveat, Settings). Decoded by
    /// `TCShellCore.ShellWordsCopy`.
    public static func shellWordsCopyJSON() -> String? {
        Localization.current.copyJSON(take(tc_shell_words_copy_json()), namespace: "shell_words_copy")
    }

    /// `tc_automatic_grant_copy_json`: the words for the disclosure an armed
    /// folder's `list_projects` row names (`automatic_disclosure`). Decoded
    /// by `TCShellCore.AutomaticGrantCopy`. Nil for a name the core does
    /// not know.
    public static func automaticGrantCopyJSON(disclosure: String) -> String? {
        LocalizedPresentation.json("automatic_grant_copy_json", parameters: ["disclosure": disclosure])
    }

    /// `tc_decisions_owed_text`: the Traces badge's text equivalent. Nil
    /// `decisionsOwed` is an unknown count, which the core never words as
    /// zero; the empty string is zero (no badge).
    public static func decisionsOwedText(_ decisionsOwed: Int?) -> String? {
        LocalizedPresentation.line("decisions_owed_text", parameters: ["count": decisionsOwed.map(Int64.init) ?? -1])
    }

    /// `tc_quit_prompt_json` with no handle: the prompt for a process with no
    /// watcher to stop. `TCDaemon.quitPromptJSON()` is the one to use while
    /// a daemon handle exists.
    public static func quitPromptWithoutWatcherJSON() -> String? {
        LocalizedPresentation.renderOwned(tc_localized_quit_prompt_json(nil))
    }
}
