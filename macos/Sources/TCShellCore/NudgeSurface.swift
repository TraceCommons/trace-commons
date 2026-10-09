import Foundation

/// What each re-engagement surface draws from `status.nudge` and
/// `reengage_due`, and what each of their buttons does.
///
/// Every word here is the daemon's (`status.nudge.text`, `mark_text`,
/// `reengage_due`): this type chooses which of those words a surface draws
/// and never writes one. It fails closed throughout: an absent nudge, a
/// `none` or `unknown` state, a lead this build does not know, an armed lead
/// without words and a button whose id this build cannot act on all draw
/// nothing.
public enum NudgeSurface {
    /// The kinds this build acts on. A later daemon may add kinds; a lead or
    /// a notification of a kind not listed here is not drawn.
    public enum Kind: String, Sendable, CaseIterable {
        case idleSessions = "idle_sessions"
        case reviewBacklog = "review_backlog"
        case verdictsLanded = "verdicts_landed"
    }

    /// Where a card is drawn: Traces leads with the asks, History with the
    /// news.
    public enum Place: Sendable {
        case traces, history

        var kinds: Set<Kind> {
            switch self {
            case .traces: [.idleSessions, .reviewBacklog]
            case .history: [.verdictsLanded]
            }
        }
    }

    /// What a button means, decided from its id and the kind it is on.
    public enum Intent: Equatable, Sendable {
        /// Open Traces at the kind's sessions.
        case review(Kind)
        /// Open History; acknowledges the verdict news.
        case seeHistory
        /// The in-app "Not now" for the kind.
        case notNow(Kind)
    }

    /// One button: what it does, and the daemon's label for it.
    public struct Action: Equatable, Sendable {
        public let intent: Intent
        public let label: String

        public init(intent: Intent, label: String) {
            self.intent = intent
            self.label = label
        }
    }

    /// A card's words, in the daemon's order.
    public struct Card: Equatable, Sendable {
        public let kind: Kind
        public let title: String
        /// Nil when the title says everything.
        public let body: String?
        public let actions: [Action]
    }

    /// The menu-bar panel's row for the lead, and what tapping it does.
    public struct PanelRow: Equatable, Sendable {
        public let kind: Kind
        public let text: String
        public let intent: Intent
    }

    /// A standalone re-engagement notification, in the daemon's words.
    public struct Notification: Equatable, Sendable {
        public let kind: Kind
        public let title: String
        public let body: String
        public let actions: [Action]
        /// What a click on the notification itself does: its first action
        /// that is not "Not now"; nil when it has none.
        public let defaultIntent: Intent?
    }

    /// Where an action takes the window.
    public enum Destination: Equatable, Sendable {
        /// Traces, narrowed to the idle sessions when `idleOnly`.
        case traces(idleOnly: Bool)
        case history
    }

    /// The request an action sends.
    public enum Request: Equatable, Sendable {
        case opened(Kind)
        case declined(Kind)
    }

    /// What an action does: one request, then (except for "Not now") a
    /// place to open.
    public struct Effect: Equatable, Sendable {
        public let request: Request
        public let destination: Destination?

        public init(request: Request, destination: Destination?) {
            self.request = request
            self.destination = destination
        }
    }

    /// The menu-bar mark the daemon lit.
    public enum Mark: Equatable, Sendable {
        case none
        /// Something new to look at: a small ring in the badge's slot.
        case news
        /// Some of the decisions owed are idle sessions: a halo around the
        /// badge.
        case ready
    }

    // MARK: - Reading the wire

    /// The lead, when the nudge is armed and names a kind this build knows.
    static func lead(_ nudge: DaemonData.Nudge?) -> Kind? {
        guard let nudge, nudge.state == "armed", let lead = nudge.lead else { return nil }
        return Kind(rawValue: lead)
    }

    /// The intent of a button with this id on this kind; nil for an id this
    /// build cannot act on, or one the daemon does not define for the kind
    /// (verdict news has no "Not now" and is not reviewed in Traces).
    public static func intent(actionId: String, kind: Kind) -> Intent? {
        switch (actionId, kind) {
        case ("review", .idleSessions), ("review", .reviewBacklog): .review(kind)
        case ("not_now", .idleSessions), ("not_now", .reviewBacklog): .notNow(kind)
        case ("see_history", .verdictsLanded): .seeHistory
        default: nil
        }
    }

    static func actions(_ wire: [DaemonData.NudgeAction], kind: Kind) -> [Action] {
        wire.compactMap { action in
            intent(actionId: action.id, kind: kind).map { Action(intent: $0, label: action.label) }
        }
    }

    // MARK: - Surfaces

    /// The card `place` draws, or nil.
    public static func card(_ nudge: DaemonData.Nudge?, on place: Place) -> Card? {
        guard let kind = lead(nudge), place.kinds.contains(kind), let text = nudge?.text,
              !text.title.isEmpty
        else { return nil }
        return Card(
            kind: kind, title: text.title, body: text.body.isEmpty ? nil : text.body,
            actions: actions(text.actions, kind: kind))
    }

    /// The panel row for the lead, or nil. Tapping it does what the card's
    /// own first action does; a row whose card has no such action is not
    /// drawn.
    public static func panelRow(_ nudge: DaemonData.Nudge?) -> PanelRow? {
        guard let kind = lead(nudge), let text = nudge?.text, !text.panelRow.isEmpty,
              let primary = actions(text.actions, kind: kind).first(where: { !$0.isNotNow })
        else { return nil }
        return PanelRow(kind: kind, text: text.panelRow, intent: primary.intent)
    }

    /// The mark to draw. Only `news` and `ready` are drawn, and neither
    /// while the strip cannot vouch for what it shows (`available` false:
    /// the core is down or its data stale).
    public static func mark(_ nudge: DaemonData.Nudge?, available: Bool) -> Mark {
        guard available else { return .none }
        switch nudge?.mark {
        case "news": return .news
        case "ready": return .ready
        default: return .none
        }
    }

    /// The lit mark's accessibility sentence and tooltip, or nil while
    /// nothing is lit.
    public static func markText(_ nudge: DaemonData.Nudge?, available: Bool) -> DaemonData.NudgeMarkText? {
        guard mark(nudge, available: available) != .none else { return nil }
        return nudge?.markText
    }

    /// The notification to post for a `reengage_due`, or nil for a kind
    /// this build does not know or one without words.
    public static func notification(_ due: DaemonData.ReengageDue) -> Notification? {
        guard let kind = Kind(rawValue: due.kind), !due.title.isEmpty, !due.body.isEmpty else { return nil }
        let drawn = actions(due.actions, kind: kind)
        return Notification(
            kind: kind, title: due.title, body: due.body, actions: drawn,
            defaultIntent: drawn.first { !$0.isNotNow }?.intent)
    }

    /// What `intent` sends and where it goes.
    public static func effect(_ intent: Intent) -> Effect {
        switch intent {
        case .review(let kind):
            Effect(request: .opened(kind), destination: .traces(idleOnly: kind == .idleSessions))
        case .seeHistory:
            Effect(request: .opened(.verdictsLanded), destination: .history)
        case .notNow(let kind):
            Effect(request: .declined(kind), destination: nil)
        }
    }

    /// Sends `effect`'s request through `client`.
    public static func send(_ effect: Effect, through client: any DaemonDataClient) async throws {
        switch effect.request {
        case .opened(let kind): try await client.nudgeOpened(kind)
        case .declined(let kind): try await client.nudgeDecline(kind)
        }
    }
}

extension NudgeSurface.Intent {
    /// The wire id of the button that means this intent.
    public var actionId: String {
        switch self {
        case .review: "review"
        case .seeHistory: "see_history"
        case .notNow: "not_now"
        }
    }
}

extension NudgeSurface.Action {
    var isNotNow: Bool {
        if case .notNow = intent { return true }
        return false
    }
}
