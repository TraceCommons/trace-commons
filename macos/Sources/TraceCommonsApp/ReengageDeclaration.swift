import Foundation
import TCShellCore

/// Holds this app's `reengage_due` declaration exactly while it can post
/// the notification the daemon would announce.
///
/// The arbiter publishes a standalone re-engagement notification, stamps it
/// against its caps, and retires the idle sessions it named, only while a
/// subscriber has declared `reengage_due` (contributor-daemon-ipc-v1_1.md,
/// "Opt-in events"). So the declaration follows `Notifier.acceptedEvents`:
/// made when the system lets the app post, withdrawn when it does not.
///
/// It is a second subscription beside the app's plain one. Replacing the
/// plain one to change what it declares would drop or repeat ordinary
/// frames in between; this one hands on `reengage_due` and ignores the
/// rest, which the plain subscription already delivers.
@MainActor
final class ReengageDeclaration<Token> {
    typealias Subscribe = (_ accepts: [String], _ handler: @escaping (String) -> Void) -> Token?
    /// Answers whether the subscription was ended; a refusal keeps it.
    typealias Unsubscribe = (Token) -> Bool

    private let subscribe: Subscribe
    private let unsubscribe: Unsubscribe
    private let deliver: (DaemonData.ReengageDue) -> Void
    private var token: Token?

    /// `deliver` runs on whatever thread the subscription's callback does.
    init(
        subscribe: @escaping Subscribe,
        unsubscribe: @escaping Unsubscribe,
        deliver: @escaping (DaemonData.ReengageDue) -> Void
    ) {
        self.subscribe = subscribe
        self.unsubscribe = unsubscribe
        self.deliver = deliver
    }

    var isDeclared: Bool { token != nil }

    /// Declares when `accepts` names anything, and withdraws when it names
    /// nothing. Asking again with the same answer changes nothing.
    func update(accepts: [String]) {
        if accepts.isEmpty {
            guard let token else { return }
            if unsubscribe(token) { self.token = nil }
        } else if token == nil {
            token = subscribe(accepts, Self.handler(deliver))
        }
    }

    /// Withdraws the declaration, for teardown.
    func withdraw() {
        update(accepts: [])
    }

    /// The subscription's callback, made outside the main actor: it runs on
    /// the Rust thread the subscription delivers on.
    private nonisolated static func handler(
        _ deliver: @escaping (DaemonData.ReengageDue) -> Void
    ) -> (String) -> Void {
        { json in
            if let due = reengageDue(json) { deliver(due) }
        }
    }

    /// The `reengage_due` a frame carries, or nil for any other frame.
    nonisolated static func reengageDue(_ json: String) -> DaemonData.ReengageDue? {
        guard case .reengageDue(let due) = DaemonEventParser.parse(json) else { return nil }
        return due
    }
}
