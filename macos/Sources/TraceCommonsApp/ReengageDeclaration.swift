import Foundation

/// The app's one subscription, and what it declares: `reengage_due` exactly
/// while this app can post the notification the daemon would announce.
///
/// The arbiter publishes a standalone re-engagement notification, stamps it
/// against its caps, and retires the idle sessions it named, only while a
/// subscriber has declared `reengage_due` (contributor-daemon-ipc-v1_1.md,
/// "Opt-in events"). So the declaration follows `Notifier.acceptedEvents`:
/// made when the system lets the app post, withdrawn when it does not.
///
/// It is changed on the one subscription (`TCDaemon.redeclare`), never by
/// adding a second: on an attached handle there is one event sink per
/// connection, and a second subscription would take the first one's frames.
@MainActor
final class ReengageDeclaration<Token> {
    /// Re-registers the subscription with a new declaration; nil when the
    /// ABI refused, and the old one still stands.
    typealias Redeclare = (_ token: Token, _ accepts: [String]) -> Token?

    private let redeclare: Redeclare
    private(set) var token: Token
    private(set) var accepts: [String]

    /// `token` is a subscription that declares nothing.
    init(token: Token, redeclare: @escaping Redeclare) {
        self.token = token
        self.accepts = []
        self.redeclare = redeclare
    }

    var isDeclared: Bool { !accepts.isEmpty }

    /// Declares `accepts`, or withdraws with none. The same answer again
    /// changes nothing, so it can be asked as often as the app comes
    /// forward.
    func update(accepts wanted: [String]) {
        guard wanted != accepts, let replaced = redeclare(token, wanted) else { return }
        token = replaced
        accepts = wanted
    }
}
