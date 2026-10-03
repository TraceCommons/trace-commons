// The legacy Settings screen's words, at the path the wording ratchet
// (`ShellWordingTests`) records them under.
//
// The screen itself is gone: every section now draws on glass from
// `Views/Settings/`, and those files author no sentence. What the old
// bodies wrote is held here, verbatim, until the core exports it -- a
// sentence leaves this file only for the core, and the count shrinks then.

/// Section headings the Settings list shows too, and the change log's
/// sentences. Held once here so the list and the section say the same
/// words. A namespace now; the view of this name was retired with the
/// legacy screen.
enum SettingsContent {
    static let consentHeading = "How may your traces be used?"
    static let auditHeading = "What has been changed on this machine"

    /// Fixed action labels to sentences. The wording is the Linux shell's
    /// `audit_sentence`, verbatim, including its catch-all: an action this
    /// build does not know still gets a row, because a change that happened
    /// and is not listed is exactly what this log exists to prevent.
    static func auditSentence(_ action: String, project: String?) -> String {
        let sentence: String
        switch action {
        case "armed-auto-upload": sentence = "Automatic contributing turned on for"
        case "disarmed-auto-upload": sentence = "Automatic contributing turned off for"
        case "queue-bulk-approved": sentence = "The whole queue was approved"
        case "consent-scopes-changed": sentence = "Permissions changed"
        case "near-ai-notice-acknowledged": sentence = "The extra privacy scan was confirmed"
        default: sentence = "Changed"
        }
        guard let project, !project.isEmpty else { return sentence }
        return "\(sentence) \(project)"
    }
}

// MARK: - Words the glass sections read

/// Every sentence this file authors, in one place, so the glass sections
/// can read them without authoring any of their own. The ratchet
/// (`ShellWordingTests`) keys on this file's path: these sentences stay here
/// until the core exports them, and then this table shrinks.
enum SettingsLegacyWords {
    static let connected = "Connected"
    static let notConnected = "Not connected"
    static let queuedNothingSent = "Sessions are being queued, but nothing can be sent."
    static let extraScanConfigured = "Extra privacy scan configured"
    static func sessionFinishedAfter(_ secs: Int) -> String {
        "A session counts as finished after \(secs) seconds of quiet."
    }
    static func atMostOneNotification(_ hours: Int) -> String {
        "At most one notification every \(hours) hours, and none when nothing is waiting."
    }
    static func undecidedDropped(_ days: Int) -> String {
        "Undecided sessions are dropped after \(days) days. Dropped means never sent."
    }
    static func stateLabel(_ title: String, _ value: Bool) -> String {
        "\(title): \(value ? "yes" : "no")"
    }
    static let startAtLogin = "Start Trace Commons when you log in"
    static let waitingOnApproval = "Waiting on approval in System Settings."
    static let turnOnInSystemSettings = """
        Turn it on in System Settings -> General -> Login Items to let \
        Trace Commons start automatically.
        """
    static func couldNotTurnOn(_ message: String) -> String { "Couldn't turn this on: \(message)" }
    static func couldNotTurnOff(_ message: String) -> String { "Couldn't turn this off: \(message)" }
    static let version = "Version"
    static let checkNow = "Check Now"
    static let copy = "Copy"
    static let checksDaily = "Checks daily"
    static let checksAutomatically = """
        Trace Commons checks for updates automatically and asks before installing.
        """
    static let managedByHomebrew = "Updates managed by Homebrew"
    static let homebrewReplaces = """
        Homebrew installed this copy, so Homebrew replaces it. Run \
        this in a terminal:
        """
    static let updatesUnavailable = "Updates unavailable"
    static let notCheckedYet = "Not checked yet on this machine."
    static func lastChecked(_ relative: String) -> String { "Last checked \(relative)." }
    static let noFeed = """
        This build has no update feed configured, so it will not look \
        for new versions. Development builds are like this. Install \
        from a release DMG to receive updates.
        """
    static let insecureFeed = """
        This build's update feed is not HTTPS, so it has been refused. \
        Reinstall from a release DMG.
        """
    static let updatesOff = "Updates are turned off for this build."
    static let notificationsRenderedHere = "Notifications rendered by this app"
    static let pausedNothingSent = "Paused. Nothing is being queued or sent."
    static let consentHeading = SettingsContent.consentHeading
    static let appliesFromNow = "Applies to traces you send from now on."
    static let alwaysIncluded = "Always included"
    static let optionalEachOne = "Optional — each one lets your traces do more"
    static let credit = "Credit"
    static let alwaysOn = "always on"
    static let nothingPreselected = "Nothing here is pre-selected on your behalf."
    static let publishedLines = [
        "Your handle — real handles only, no pseudonyms.",
        "Aggregate counts: accepted, novelty credit, accept rate.",
        "The date you went public.",
        "Your bio, if you write one."
    ]
    static let neverLines = [
        "Your traces or anything in them.",
        "Per-trace data of any kind.",
        "Anything about sessions you didn't send."
    ]
    static let doNotTrustProfileWording = "Do not trust the public-profile wording on this screen."
    static let auditHeading = SettingsContent.auditHeading
    static let noProjectsYet = "No projects seen yet."
    static let nothingChanged = "Nothing has been changed."
    static func auditSentence(_ action: String, project: String?) -> String {
        SettingsContent.auditSentence(action, project: project)
    }
}
