//! Desktop notifications, off until asked for.
//
// This is the loudest channel IronWire has, so it is the one held to the
// narrowest filter. `NotificationPolicy` decides what qualifies — mirroring the
// daemon's own `Event::is_user_visible` — and this file only delivers.
//
// Off by default, and authorisation is requested at the moment the user turns
// them on rather than at first launch. An app that asks for notification
// permission before it has told you anything is asking you to trust a promise.

import Foundation
import IronWireKit
import UserNotifications

@MainActor
final class Notifier: ObservableObject {
    private static let defaultsKey = "notificationsEnabled"

    /// Whether the user has opted in. Persisted, and false until they do.
    @Published var enabled: Bool {
        didSet {
            guard enabled != oldValue else { return }
            UserDefaults.standard.set(enabled, forKey: Self.defaultsKey)
            if enabled { requestAuthorisation() }
        }
    }

    /// Whether notifications can be delivered at all.
    ///
    /// `UNUserNotificationCenter` needs a real bundle: a `swift run` of the
    /// executable has none, and touching the centre there traps. Checking once
    /// keeps the toggle honest instead of silently doing nothing.
    private let available: Bool

    init() {
        enabled = UserDefaults.standard.bool(forKey: Self.defaultsKey)
        available = Bundle.main.bundleIdentifier != nil
    }

    /// Show an event, if it is one of the few that warrant it and the user asked
    /// to see them.
    func deliver(_ event: Event) {
        guard enabled, available else { return }
        guard let (title, body) = NotificationPolicy.notification(for: event) else { return }

        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body

        let request = UNNotificationRequest(
            identifier: UUID().uuidString,
            content: content,
            trigger: nil
        )
        // Failure here is not worth telling anyone about: the menu already shows
        // the same state, and this channel is the optional one.
        UNUserNotificationCenter.current().add(request, withCompletionHandler: nil)
    }

    private func requestAuthorisation() {
        guard available else { return }
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound]) { granted, _ in
                guard !granted else { return }
                // The user said no at the system prompt. Reflect that in the
                // menu rather than leaving a toggle on that does nothing.
                Task { @MainActor [weak self] in self?.enabled = false }
            }
    }
}
