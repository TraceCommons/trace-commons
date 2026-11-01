import Foundation

/// Owns one window's notification registrations. This object has no actor
/// state: removing its immutable tokens is safe on the thread releasing it,
/// including when AppKit releases the owning view away from the main actor.
final class WindowNotificationObservers {
    private let center: NotificationCenter
    private let tokens: [NSObjectProtocol]

    init(center: NotificationCenter, tokens: [NSObjectProtocol]) {
        self.center = center
        self.tokens = tokens
    }

    deinit {
        tokens.forEach(center.removeObserver)
    }
}
