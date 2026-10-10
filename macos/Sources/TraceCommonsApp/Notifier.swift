import Foundation
import TCBridge
import TCShellCore
import UserNotifications

/// Local notifications: the digest, and the re-engagement notifications.
///
/// **No action may upload.** The digest's `Review` opens the window on the
/// queue; `Not now` dismisses and does nothing else. Its presence is what
/// makes the notification feel non-coercive, and the absence of any third
/// action is what keeps a misclick from contributing a transcript. A
/// re-engagement notification's buttons are the daemon's (`review`,
/// `see_history`, `not_now`): `review` and `see_history` open a place, and
/// `not_now` only dismisses the notification. It records nothing, so the
/// in-app suggestion stays where it was.
///
/// The app sets `local_notifications: false` in daemon settings and renders
/// these itself, precisely so it -- not the daemon -- controls that action
/// list. The daemon's `digest_due` and `reengage_due` events are the
/// triggers; every word posted is the daemon's or the core's.
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    static let categoryIdentifier = "trace-commons.digest"
    static let reviewAction = "trace-commons.review"
    static let notNowAction = "trace-commons.not-now"

    /// Set by the app so `Review` can open the window.
    var onReview: (() -> Void)?
    /// Set by the app so a re-engagement button sends its request and
    /// opens its place.
    var onNudge: ((NudgeSurface.Intent) -> Void)?

    /// The opt-in events this app can render.
    static let renderedEvents = ["reengage_due"]

    /// The opt-in events to declare: `renderedEvents` only while a
    /// notification can actually be posted, and none otherwise. The daemon
    /// stamps an announcement against its caps, and retires the idle
    /// sessions it named, only while a subscriber has declared
    /// `reengage_due`, so a declaration made while the system refuses
    /// notifications (denied, never asked, or no notification centre)
    /// would spend that budget on notifications nobody sees.
    static func acceptedEvents(available: Bool, status: UNAuthorizationStatus?) -> [String] {
        available && canPostDigest(status) ? renderedEvents : []
    }

    /// `acceptedEvents` for this process, read fresh.
    func acceptedEvents() async -> [String] {
        Self.acceptedEvents(available: available, status: await authorizationStatus())
    }

    /// Called on the main actor once the system's permission prompt has
    /// been answered, so the app can declare or withdraw `reengage_due`.
    var onAuthorizationAnswered: (() -> Void)?
    /// `userInfo` keys on a re-engagement notification.
    static let kindKey = "trace-commons.nudge.kind"
    static let defaultActionKey = "trace-commons.nudge.default"
    /// A re-engagement button's identifier is this prefix and the daemon's
    /// action id.
    static let nudgeActionPrefix = "trace-commons.nudge."

    /// The core's fixed nudge words: the digest's two button labels.
    static let nudgeCopy = NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON())

    /// One notification button.
    struct Button: Equatable {
        let identifier: String
        let label: String
        /// Brings the app forward: every button that opens a place.
        let opensApp: Bool
    }

    /// What a `reengage_due` posts: the daemon's title and body, unchanged,
    /// its buttons in its order under a category for that set of buttons,
    /// and the kind, so a response can be routed.
    struct NudgePlan: Equatable {
        let title: String
        let body: String
        let categoryIdentifier: String
        let buttons: [Button]
        let userInfo: [String: String]
    }

    /// The plan for `due`, or nil for a kind this build does not know or a
    /// notification without words.
    static func plan(_ due: DaemonData.ReengageDue) -> NudgePlan? {
        guard let note = NudgeSurface.notification(due) else { return nil }
        let set = nudgeCategory(note.actions.map { (id: $0.intent.actionId, label: $0.label) })
        var info = [kindKey: note.kind.rawValue]
        if let fallback = note.defaultIntent { info[defaultActionKey] = fallback.actionId }
        return NudgePlan(
            title: note.title, body: note.body,
            categoryIdentifier: set.identifier,
            buttons: set.buttons, userInfo: info)
    }

    /// One category's identifier and buttons.
    struct CategorySpec: Equatable {
        let identifier: String
        let buttons: [Button]
    }

    /// The category for a set of re-engagement buttons, by action id and
    /// label: one identifier per set of ids, and every button but `not_now`
    /// brings the app forward.
    static func nudgeCategory(_ actions: [(id: String, label: String)]) -> CategorySpec {
        CategorySpec(
            identifier: nudgeActionPrefix + actions.map(\.id).joined(separator: "+"),
            buttons: actions.map {
                Button(identifier: nudgeActionPrefix + $0.id, label: $0.label, opensApp: $0.id != notNowActionId)
            })
    }

    /// The action id that only records an answer and opens nothing.
    private static let notNowActionId = "not_now"

    /// Every category registered at launch: the digest's, then each
    /// re-engagement button set the daemon sends today (idle: review and
    /// not now; verdicts: see history), in the core's words.
    ///
    /// `setNotificationCategories` replaces the whole registered set, and a
    /// notification still in Notification Center after a relaunch keeps its
    /// buttons only while its category is registered, so launch registers
    /// them all rather than the digest's alone. A set whose words are
    /// missing is left out; `postReengage` registers whatever it posts.
    static func launchCategories(_ copy: NudgeCopy?) -> [CategorySpec] {
        var specs = [CategorySpec(identifier: categoryIdentifier, buttons: digestButtons(copy))]
        if let review = copy?[.notifyActionReviewIdle], let notNow = copy?[.notifyActionNotNow] {
            specs.append(nudgeCategory([(id: "review", label: review), (id: notNowActionId, label: notNow)]))
        }
        if let history = copy?[.notifyActionSeeHistory] {
            specs.append(nudgeCategory([(id: "see_history", label: history)]))
        }
        return specs
    }

    /// The intent a response to a re-engagement notification means; nil
    /// for a dismissal, for its own Not now (which dismisses and records
    /// nothing, so the in-app card is not silenced), for a notification
    /// that is not one (the digest), or for an id the kind does not define.
    static func nudgeIntent(actionIdentifier: String, userInfo: [AnyHashable: Any]) -> NudgeSurface.Intent? {
        guard let raw = userInfo[kindKey] as? String, let kind = NudgeSurface.Kind(rawValue: raw) else { return nil }
        let actionId: String?
        if actionIdentifier == UNNotificationDefaultActionIdentifier {
            actionId = userInfo[defaultActionKey] as? String
        } else if actionIdentifier.hasPrefix(nudgeActionPrefix) {
            actionId = String(actionIdentifier.dropFirst(nudgeActionPrefix.count))
        } else {
            actionId = nil
        }
        guard let actionId, actionId != notNowActionId else { return nil }
        return NudgeSurface.intent(actionId: actionId, kind: kind)
    }

    /// The digest's buttons, in the core's words; none without them.
    static func digestButtons(_ copy: NudgeCopy?) -> [Button] {
        guard let review = copy?[.digestActionReview], let notNow = copy?[.digestActionNotNow] else { return [] }
        return [
            Button(identifier: reviewAction, label: review, opensApp: true),
            Button(identifier: notNowAction, label: notNow, opensApp: false),
        ]
    }

    static func category(_ identifier: String, _ buttons: [Button]) -> UNNotificationCategory {
        UNNotificationCategory(
            identifier: identifier,
            actions: buttons.map {
                UNNotificationAction(identifier: $0.identifier, title: $0.label, options: $0.opensApp ? [.foreground] : [])
            },
            intentIdentifiers: [],
            options: [])
    }

    private var available: Bool {
        // UNUserNotificationCenter traps in a process that is not an app
        // bundle: a bare `swift run` binary has no bundle identifier, and
        // the `xctest` runner has one but no app bundle around it. This
        // stays inert there instead of taking the process down.
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    /// Registers the digest's category and every re-engagement one
    /// (`launchCategories`). Deliberately does NOT ask for
    /// authorization: it is asked with a sentence saying what notifications
    /// are for, not sprung at first launch before the app has said what it
    /// is. See `requestAuthorization`, which Settings' `StartupSection`
    /// calls; the first run no longer offers it.
    func configure() {
        guard available else { return }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories(
            Set(Self.launchCategories(Self.nudgeCopy).map { Self.category($0.identifier, $0.buttons) }))
    }

    /// Adds `category` beside the ones already registered, replacing one
    /// with the same identifier. A re-engagement category is registered
    /// when its first notification is posted, since its buttons are the
    /// daemon's.
    private func register(_ category: UNNotificationCategory) async {
        let center = UNUserNotificationCenter.current()
        var categories = await center.notificationCategories()
        categories = categories.filter { $0.identifier != category.identifier }
        categories.insert(category)
        center.setNotificationCategories(categories)
    }

    /// A re-engagement notification, in the daemon's words. Passive, as the
    /// digest is, so Focus and Do Not Disturb hold it.
    func postReengage(_ due: DaemonData.ReengageDue) {
        guard available, let plan = Self.plan(due) else { return }
        let content = UNMutableNotificationContent()
        content.title = plan.title
        content.body = plan.body
        content.categoryIdentifier = plan.categoryIdentifier
        content.userInfo = plan.userInfo
        content.interruptionLevel = .passive
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        let category = Self.category(plan.categoryIdentifier, plan.buttons)
        Task {
            guard Self.canPostDigest(await authorizationStatus()), !Task.isCancelled else { return }
            await register(category)
            try? await UNUserNotificationCenter.current().add(request)
        }
    }

    /// Where the system stands on this app's notifications, or nil where
    /// there is no notification centre to ask (a bare `swift run` binary).
    ///
    /// Read fresh at each call, never cached: the contributor can flip
    /// this in System Settings while the window is open, and a value held
    /// from launch would then claim a state that is no longer true.
    func authorizationStatus() async -> UNAuthorizationStatus? {
        guard available else { return nil }
        return await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// Puts the system's permission prompt up. Answers whether the
    /// contributor allowed it. Called only from a button that sits under a
    /// sentence explaining what the notifications are -- never at launch.
    func requestAuthorization() async -> Bool {
        guard available else { return false }
        let allowed = (try? await UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound])) ?? false
        Task { @MainActor in Self.shared.onAuthorizationAnswered?() }
        return allowed
    }

    /// Puts the system's prompt up only if it was never answered, after a
    /// one-time offer to turn a kind on was accepted under its sentence.
    /// Answers whether the prompt was shown.
    func requestAuthorizationIfNeverAsked() async -> Bool {
        guard await authorizationStatus() == .notDetermined else { return false }
        _ = await requestAuthorization()
        return true
    }

    /// Where the contributor turns notifications back on after saying no.
    /// The pane URL opens macOS notification settings.
    static let systemSettingsURL = URL(
        string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
    )!

    /// The one sentence that says what a notification from this app is.
    /// Shown above the permission button in Settings (`StartupSection`).
    static let copy = TCOnboardingCopy.load()
    static var purpose: String { copy?.notificationPurpose ?? "" }

    static func canPostDigest(_ status: UNAuthorizationStatus?) -> Bool {
        switch status {
        case .authorized?, .provisional?, .ephemeral?: return true
        default: return false
        }
    }

    /// The configured digest. Passive, so Focus and Do Not Disturb hold it.
    ///
    /// Fires for either half: sessions waiting for review, or sessions that
    /// were contributed without being asked about since the last one. It used
    /// to guard on `pendingCount > 0` alone, which meant a contributor whose
    /// projects were all armed -- nothing ever queued, nothing ever waiting --
    /// received no digest at any point. Silence was the reward for trusting
    /// the app most.
    /// `text` is the core's body (`digest_due.text`), which already ends with
    /// a folded re-engagement sentence when one was folded in: it is posted
    /// unchanged. Only a daemon that sent no text gets the body worded from
    /// the counts, as before.
    func postDigest(
        text: String = "",
        pendingCount: Int,
        projects: [String],
        contributedCount: Int = 0,
        contributedProjects: [String] = [],
        creditPending: Double = 0
    ) {
        guard available,
              let body = Self.digestBody(
                text: text, pendingCount: pendingCount, projects: projects, contributedCount: contributedCount,
                contributedProjects: contributedProjects, creditPending: creditPending)
        else { return }
        let content = UNMutableNotificationContent()
        content.title = Self.nudgeCopy?[.digestTitle] ?? "Trace Commons"
        content.body = body
        content.categoryIdentifier = Self.categoryIdentifier
        content.interruptionLevel = .passive
        let request = UNNotificationRequest(
            identifier: UUID().uuidString,
            content: content,
            trigger: nil
        )
        Task {
            guard Self.canPostDigest(await authorizationStatus()), !Task.isCancelled else { return }
            try? await UNUserNotificationCenter.current().add(request)
        }
    }

    /// The digest's body: the core's text, unchanged, whenever it sent
    /// one; otherwise the counts' own lines, as before; nil when there is
    /// nothing to say.
    static func digestBody(
        text: String, pendingCount: Int, projects: [String], contributedCount: Int = 0,
        contributedProjects: [String] = [], creditPending: Double = 0
    ) -> String? {
        if !text.isEmpty { return text }
        guard pendingCount > 0 || contributedCount > 0 else { return nil }
        // Two sentences, either of which may be absent: what is waiting for
        // you, and what went without you. They are about different things and
        // a contributor acts on only one of them, so they are separate lines
        // rather than one merged sentence.
        var lines: [String] = []
        if pendingCount > 0 {
            let noun = pendingCount == 1 ? "trace" : "traces"
            let from = projects.isEmpty ? "" : " from " + Self.joined(projects)
            lines.append("\(pendingCount) \(noun) ready\(from).")
            lines.append("Nothing is sent until you review them.")
        }
        if let contributed = DigestCopy.contributionLine(
            count: contributedCount,
            projects: contributedProjects,
            creditPending: creditPending
        ) {
            lines.append(contributed)
        }
        return lines.joined(separator: "\n")
    }

    private static func joined(_ labels: [String]) -> String {
        switch labels.count {
        case 0: return ""
        case 1: return labels[0]
        case 2: return "\(labels[0]) and \(labels[1])"
        default:
            return labels.dropLast().joined(separator: ", ") + " and " + labels[labels.count - 1]
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        // A re-engagement button: its request and its place, through the
        // app. A dismissal resolves to nothing and does nothing.
        if let intent = Self.nudgeIntent(
            actionIdentifier: response.actionIdentifier, userInfo: response.notification.request.content.userInfo)
        {
            DispatchQueue.main.async { self.onNudge?(intent) }
            completionHandler()
            return
        }
        // The digest: only `Review` does anything, and what it does is open
        // a window.
        if response.actionIdentifier == Self.reviewAction
            || response.actionIdentifier == UNNotificationDefaultActionIdentifier
        {
            DispatchQueue.main.async { self.onReview?() }
        }
        completionHandler()
    }
}
