import Foundation
import TCShellCore
import UserNotifications

/// Local notifications, with exactly two actions.
///
/// **No action may upload.** `Review` opens the window on the queue;
/// `Not now` dismisses and does nothing else. Its presence is what makes the
/// notification feel non-coercive, and the absence of any third action is
/// what keeps a misclick from contributing a transcript.
///
/// The app sets `local_notifications: false` in daemon settings and renders
/// these itself, precisely so it -- not the daemon -- controls that action
/// list. The daemon's `digest_due` event is the trigger.
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    static let categoryIdentifier = "trace-commons.digest"
    static let reviewAction = "trace-commons.review"
    static let notNowAction = "trace-commons.not-now"

    /// Set by the app so `Review` can open the window.
    var onReview: (() -> Void)?

    private var available: Bool {
        // UNUserNotificationCenter traps in a process with no bundle
        // identifier (a bare `swift run` binary), so this stays inert there
        // instead of taking the app down.
        Bundle.main.bundleIdentifier != nil
    }

    /// Registers the two-action category. Authorization is requested at the
    /// end of onboarding in the finished product, with an explanation; that
    /// flow is not built yet, so this asks once at launch and carries on
    /// regardless of the answer.
    func configure() {
        guard available else { return }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let review = UNNotificationAction(
            identifier: Self.reviewAction,
            title: "Review",
            options: [.foreground]
        )
        let notNow = UNNotificationAction(
            identifier: Self.notNowAction,
            title: "Not now",
            options: []
        )
        let category = UNNotificationCategory(
            identifier: Self.categoryIdentifier,
            actions: [review, notNow],
            intentIdentifiers: [],
            options: []
        )
        center.setNotificationCategories([category])
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    /// The 4-hour digest. Passive, so Focus and Do Not Disturb hold it.
    ///
    /// Fires for either half: sessions waiting for review, or sessions that
    /// were contributed without being asked about since the last one. It used
    /// to guard on `pendingCount > 0` alone, which meant a contributor whose
    /// projects were all armed -- nothing ever queued, nothing ever waiting --
    /// received no digest at any point. Silence was the reward for trusting
    /// the app most.
    func postDigest(
        pendingCount: Int,
        projects: [String],
        contributedCount: Int = 0,
        contributedProjects: [String] = [],
        creditPending: Double = 0
    ) {
        guard available, pendingCount > 0 || contributedCount > 0 else { return }
        let content = UNMutableNotificationContent()
        content.title = "Trace Commons"
        // Two sentences, either of which may be absent: what is waiting for
        // you, and what went without you. They are about different things and
        // a contributor acts on only one of them, so they are separate lines
        // rather than one merged sentence.
        var lines: [String] = []
        if pendingCount > 0 {
            let noun = pendingCount == 1 ? "session" : "sessions"
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
        content.body = lines.joined(separator: "\n")
        content.categoryIdentifier = Self.categoryIdentifier
        content.interruptionLevel = .passive
        let request = UNNotificationRequest(
            identifier: UUID().uuidString,
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
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
        // Only `Review` does anything, and what it does is open a window.
        if response.actionIdentifier == Self.reviewAction
            || response.actionIdentifier == UNNotificationDefaultActionIdentifier
        {
            DispatchQueue.main.async { self.onReview?() }
        }
        completionHandler()
    }
}
