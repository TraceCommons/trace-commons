import TCBridge
import TCShellCore

// The legacy Settings screen's words. The screen itself is gone: every
// section now draws on glass from `Views/Settings/`. What the old bodies
// wrote is the core's now (`shell_words_copy::settings_words`, through
// `ShellWords`, #1146 parity 2026-10-07); only the notices' dismiss
// fallback below is still this file's.

/// Section headings the Settings list shows too, and the change log's
/// sentences. Held once here so the list and the section say the same
/// words. A namespace now; the view of this name was retired with the
/// legacy screen.
enum SettingsContent {
    static var consentHeading: String { ShellWords.table?.settings.consentHeading ?? "" }
    /// #1146's `sections.ts` word, from the core (owner ruling, 2026-10-06).
    static var auditHeading: String { MonitorWords.table?.shell.changesHeading ?? "" }

    /// Fixed action labels to sentences, the core's. An action this build
    /// does not know still gets a row, because a change that happened and
    /// is not listed is exactly what this log exists to prevent.
    static func auditSentence(_ action: String, project: String?) -> String {
        guard let words = ShellWords.table?.settings else { return "" }
        let sentence = words.auditActions[action] ?? words.auditChanged
        guard let project, !project.isEmpty else { return sentence }
        return sentence + " " + project
    }
}

// MARK: - Words the glass sections read

/// Every sentence the glass Settings sections read that has no other core
/// table, from the core's `settings_words`.
enum SettingsLegacyWords {
    private static var words: ShellWordsCopy.Settings? { ShellWords.table?.settings }
    private static func fill(_ template: String?, _ values: [String: String]) -> String {
        ShellWords.fill(template ?? "", values)
    }

    static var connected: String { words?.connected ?? "" }
    static var notConnected: String { words?.notConnected ?? "" }
    static var queuedNothingSent: String { words?.queuedNothingSent ?? "" }
    static var extraScanConfigured: String { words?.extraScanConfigured ?? "" }
    static func sessionFinishedAfter(_ secs: Int) -> String {
        fill(words?.sessionFinishedAfter, ["seconds": String(secs)])
    }
    static func atMostOneNotification(_ hours: Int) -> String {
        fill(words?.atMostOneNotification, ["hours": String(hours)])
    }
    static func undecidedDropped(_ days: Int) -> String {
        fill(words?.undecidedDropped, ["days": String(days)])
    }
    static func stateLabel(_ title: String, _ value: Bool) -> String {
        fill(value ? words?.stateYes : words?.stateNo, ["title": title])
    }
    static var startAtLogin: String { MonitorWords.table?.shell.startAtLogin ?? "" }
    /// #1146's two-level card heads and the watcher card, from the core's
    /// monitor shell table.
    static var refresh: String { MonitorWords.table?.shell.settingsRefresh ?? "" }
    static var consentEyebrow: String { MonitorWords.table?.shell.consentEyebrow ?? "" }
    static var desktopEyebrow: String { MonitorWords.table?.shell.desktopEyebrow ?? "" }
    static var desktopTitle: String { MonitorWords.table?.shell.desktopTitle ?? "" }
    static var discoveryEyebrow: String { MonitorWords.table?.shell.discoveryEyebrow ?? "" }
    static var discoveryTitle: String { MonitorWords.table?.shell.discoveryTitle ?? "" }
    static var watcherEyebrow: String { MonitorWords.table?.shell.watcherEyebrow ?? "" }
    static var watcherTitle: String { MonitorWords.table?.shell.watcherTitle ?? "" }
    static var watcherWatching: String { MonitorWords.table?.shell.watcherWatching ?? "" }
    static var watcherPaused: String { MonitorWords.table?.shell.watcherPaused ?? "" }
    static var watcherCaption: String { MonitorWords.table?.shell.watcherCaption ?? "" }
    static var connectionReady: String { MonitorWords.table?.shell.connectionReady ?? "" }
    static var connectionLocalOnly: String { MonitorWords.table?.shell.connectionLocalOnly ?? "" }
    static var pauseWatcher: String { MonitorWords.table?.shell.pauseWatcher ?? "" }
    static var resumeWatcher: String { MonitorWords.table?.shell.resumeWatcher ?? "" }
    static var waitingOnApproval: String { words?.waitingOnApproval ?? "" }
    static var turnOnInSystemSettings: String { words?.turnOnInSystemSettings ?? "" }
    static func couldNotTurnOn(_ message: String) -> String { fill(words?.couldNotTurnOn, ["message": message]) }
    static func couldNotTurnOff(_ message: String) -> String { fill(words?.couldNotTurnOff, ["message": message]) }
    static var version: String { words?.version ?? "" }
    static var checkNow: String { words?.checkNow ?? "" }
    static var copy: String { words?.copy ?? "" }
    static var checksDaily: String { words?.checksDaily ?? "" }
    static var checksAutomatically: String { words?.checksAutomatically ?? "" }
    static var managedByHomebrew: String { words?.managedByHomebrew ?? "" }
    static var homebrewReplaces: String { words?.homebrewReplaces ?? "" }
    static var updatesUnavailable: String { words?.updatesUnavailable ?? "" }
    static var notCheckedYet: String { words?.notCheckedYet ?? "" }
    static func lastChecked(_ relative: String) -> String { fill(words?.lastChecked, ["relative": relative]) }
    static var noFeed: String { words?.noFeed ?? "" }
    static var insecureFeed: String { words?.insecureFeed ?? "" }
    static var updatesOff: String { words?.updatesOff ?? "" }
    static var notificationsRenderedHere: String { words?.notificationsRenderedHere ?? "" }
    static var pausedNothingSent: String { words?.pausedNothingSent ?? "" }
    static var consentHeading: String { SettingsContent.consentHeading }
    static var appliesFromNow: String { words?.appliesFromNow ?? "" }
    static var alwaysIncluded: String { words?.alwaysIncluded ?? "" }
    /// #1146's group name ("Optional data use").
    static var optionalEachOne: String { words?.optionalDataUse ?? "" }
    static var credit: String { words?.credit ?? "" }
    /// The always-on scope's tag (#1146's "required").
    static var alwaysOn: String { words?.required ?? "" }
    static var nothingPreselected: String { words?.nothingPreselected ?? "" }
    static var auditHeading: String { SettingsContent.auditHeading }
    static var noProjectsYet: String { MonitorWords.table?.shell.projectsEmpty ?? "" }
    static var nothingChanged: String { words?.nothingChanged ?? "" }
    static func auditSentence(_ action: String, project: String?) -> String {
        SettingsContent.auditSentence(action, project: project)
    }
}

// MARK: - The notices' dismiss word

/// What every glass notice that can be put away names its dismiss control.
/// Moved verbatim from the retired legacy banner (R15), with its counted
/// sentence: the core's word first, this file's when the core's copy does
/// not decode, so no notice is ever left undismissable.
enum ActionNoticeWords {
    /// The dismiss control's name, reachable so a glass notice that cannot
    /// reach the core's word still names its control.
    static let dismissWord = "Dismiss this message"

    /// The core's word for dismissing a notice, for every glass notice that
    /// can be put away; nil when the core's copy does not decode, and the
    /// caller falls back to `dismissWord`.
    static let coreDismissWord = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.dismissAction
}
