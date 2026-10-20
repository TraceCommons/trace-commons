//! Swift mirrors of the control API's view types.
//
// `StatusView` is the contract between the daemon and everything that displays
// it. `ironwire status` and `ironwire statusline` already read it; this app is
// the third reader, and it must show the same facts as the first two. When this
// app needs something new, the field goes into `StatusView` and is computed in
// Rust — never derived here (`docs/DESIGN.md` §6).
//
// The source of truth is `crates/ironwire_proxy/src/control.rs`. Two properties
// are carried over deliberately:
//
// 1. **Unknown tags degrade one field, never the menu.** `HeadroomView` and
//    `UpdateStatus` are `#[serde(tag = "state")]` and `Event` is
//    `#[serde(tag = "type")]`. A Swift enum that throws on an unrecognised tag
//    would blank the entire dropdown the first time the daemon is newer than the
//    app — which is the normal state, since the daemon outlives the app that
//    talks to it.
// 2. **A missing field is a default, not an error.** Several fields carry
//    `#[serde(default)]` on the Rust side precisely so an older daemon still
//    parses. Every decode here is `decodeIfPresent` for the same reason.

import Foundation

// MARK: - Status

/// Full daemon state, as `GET /_ironwire/status` returns it.
public struct StatusView: Decodable, Sendable, Equatable {
    /// IronWire version.
    public let version: String
    /// Port in use.
    public let port: Int
    /// Conversations with a sticky route.
    public let trackedConversations: Int
    /// Active pin, if any. `backend` or `backend:model`.
    public let pin: String?
    /// Every configured backend.
    public let backends: [BackendView]
    /// Every pool, seen as one balance.
    public let balance: BalanceView
    /// What the privacy filter is *doing*. Rendered verbatim or not at all
    /// (`docs/TRUST.md` I7).
    public let privacy: String?
    /// Serial of the signed catalog document in force.
    public let catalogSerial: UInt64
    /// What the last update check concluded. Notification, never action.
    public let update: UpdateStatus
    /// The most recent route this daemon took.
    public let lastRoute: LastRouteView?
    /// IronWire's own traffic over its own window. Never a provider's quota.
    public let usage: UsageView

    private enum CodingKeys: String, CodingKey {
        case version, port, trackedConversations, pin, backends, balance
        case privacy, catalogSerial, update, lastRoute, usage
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(String.self, forKey: .version) ?? "unknown"
        port = try c.decodeIfPresent(Int.self, forKey: .port) ?? Discovery.defaultPort
        trackedConversations = try c.decodeIfPresent(Int.self, forKey: .trackedConversations) ?? 0
        pin = try c.decodeIfPresent(String.self, forKey: .pin)
        backends = try c.decodeIfPresent([BackendView].self, forKey: .backends) ?? []
        balance = try c.decodeIfPresent(BalanceView.self, forKey: .balance) ?? BalanceView()
        privacy = try c.decodeIfPresent(String.self, forKey: .privacy)
        catalogSerial = try c.decodeIfPresent(UInt64.self, forKey: .catalogSerial) ?? 0
        update = try c.decodeIfPresent(UpdateStatus.self, forKey: .update) ?? .unknown
        lastRoute = try c.decodeIfPresent(LastRouteView.self, forKey: .lastRoute)
        usage = try c.decodeIfPresent(UsageView.self, forKey: .usage) ?? UsageView()
    }

    /// Test seam. Nothing in the app constructs one of these — it comes off the
    /// wire or it does not exist.
    public init(
        version: String, port: Int, trackedConversations: Int = 0, pin: String? = nil,
        backends: [BackendView] = [], balance: BalanceView = BalanceView(),
        privacy: String? = nil, catalogSerial: UInt64 = 0, update: UpdateStatus = .upToDate,
        lastRoute: LastRouteView? = nil, usage: UsageView = UsageView()
    ) {
        self.usage = usage
        self.version = version
        self.port = port
        self.trackedConversations = trackedConversations
        self.pin = pin
        self.backends = backends
        self.balance = balance
        self.privacy = privacy
        self.catalogSerial = catalogSerial
        self.update = update
        self.lastRoute = lastRoute
    }
}

/// IronWire's own traffic over its own window, from the local ledger.
///
/// The distinction this type exists to keep is `AGENTS.md` rule 2's one
/// apparent exception: everything here measures *what IronWire sent*, and never
/// claims to know what a provider has left. That is why `usedPct` is only ever
/// a percentage **of `ceiling`**, and why `ceiling.describes` is rendered
/// verbatim beside it — a bare percentage with no stated basis is the
/// fabrication the whole screen avoids.
public struct UsageView: Decodable, Sendable, Equatable {
    /// Open windows, one per backend with traffic, busiest first.
    public let sessions: [SessionUsageView]
    /// Completed windows the percentile was taken over. Zero means there is no
    /// history yet, which is why a session may carry no ceiling at all.
    public let completedSessions: Int
    /// Length of a window, in hours.
    public let sessionHours: Int

    private enum CodingKeys: String, CodingKey {
        case sessions, completedSessions, sessionHours
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessions = try c.decodeIfPresent([SessionUsageView].self, forKey: .sessions) ?? []
        completedSessions = try c.decodeIfPresent(Int.self, forKey: .completedSessions) ?? 0
        sessionHours = try c.decodeIfPresent(Int.self, forKey: .sessionHours) ?? 5
    }

    public init(
        sessions: [SessionUsageView] = [], completedSessions: Int = 0, sessionHours: Int = 5
    ) {
        self.sessions = sessions
        self.completedSessions = completedSessions
        self.sessionHours = sessionHours
    }
}

/// One backend's open window.
public struct SessionUsageView: Decodable, Sendable, Equatable, Identifiable {
    /// Backend id. Also the identity — one open window per backend.
    public let backend: String
    /// Minutes until the window closes.
    public let remainingMinutes: Double
    /// Exchanges in it.
    public let exchanges: Int
    /// Percent of `ceiling` consumed. `nil` without a ceiling, and rendered as
    /// nothing rather than as zero.
    public let usedPct: Double?
    /// Minutes until the ceiling is reached at the current rate. `nil` when
    /// there is no ceiling, no rate, or nothing left of it.
    public let exhaustsInMinutes: Double?
    /// What the percentage is measured against, when there is one.
    public let ceiling: CeilingView?

    public var id: String { backend }

    private enum CodingKeys: String, CodingKey {
        case backend, remainingMinutes, exchanges, usedPct, exhaustsInMinutes, ceiling
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        backend = try c.decodeIfPresent(String.self, forKey: .backend) ?? ""
        remainingMinutes = try c.decodeIfPresent(Double.self, forKey: .remainingMinutes) ?? 0
        exchanges = try c.decodeIfPresent(Int.self, forKey: .exchanges) ?? 0
        usedPct = try c.decodeIfPresent(Double.self, forKey: .usedPct)
        exhaustsInMinutes = try c.decodeIfPresent(Double.self, forKey: .exhaustsInMinutes)
        ceiling = try c.decodeIfPresent(CeilingView.self, forKey: .ceiling)
    }

    public init(
        backend: String, remainingMinutes: Double = 0, exchanges: Int = 0,
        usedPct: Double? = nil, exhaustsInMinutes: Double? = nil, ceiling: CeilingView? = nil
    ) {
        self.backend = backend
        self.remainingMinutes = remainingMinutes
        self.exchanges = exchanges
        self.usedPct = usedPct
        self.exhaustsInMinutes = exhaustsInMinutes
        self.ceiling = ceiling
    }

    /// Whether the ceiling arrives before the window closes — the one question
    /// this section exists to answer. Mirrors `SessionUsage::exhausts_before_close`.
    public var exhaustsBeforeClose: Bool {
        guard let minutes = exhaustsInMinutes else { return false }
        return minutes < remainingMinutes
    }
}

/// What an open window is being compared against.
public struct CeilingView: Decodable, Sendable, Equatable {
    /// The daemon's own phrase for where this came from, e.g. `your own p90
    /// over 14 sessions`. Rendered verbatim: it is what stops a percentage
    /// reading as a provider's limit.
    public let describes: String
    /// Whether even the source of the figure calls it unverified.
    public let unverified: Bool

    private enum CodingKeys: String, CodingKey {
        case describes = "description"
        case unverified
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        describes = try c.decodeIfPresent(String.self, forKey: .describes) ?? ""
        unverified = try c.decodeIfPresent(Bool.self, forKey: .unverified) ?? false
    }

    public init(describes: String, unverified: Bool = false) {
        self.describes = describes
        self.unverified = unverified
    }
}

/// One backend, as `ironwire status` renders it.
public struct BackendView: Decodable, Sendable, Equatable, Identifiable {
    /// Stable id — what `/_ironwire/pin` accepts.
    public let id: String
    /// Display name.
    public let name: String
    /// `subscription` / `api_key` / `credits` / `local`.
    public let kind: String
    /// Whether a credential was found.
    public let authenticated: Bool
    /// Whether consent has been recorded, where it is required.
    public let consented: Bool
    /// Why not authenticated, when applicable.
    public let detail: String?
    /// Observed capacity — or unknown. Never a guess.
    public let headroom: HeadroomView
    /// Circuit state, so a backend being skipped says so rather than looking idle.
    public let health: HealthView
    /// Models offered.
    public let models: [String]
    /// What to run to connect this backend, verbatim from the daemon — for a
    /// subscription, the login it reads. Absent from a daemon that predates it.
    public let connectCommand: String?

    private enum CodingKeys: String, CodingKey {
        case id, name, kind, authenticated, consented, detail, headroom, health, models
        case connectCommand
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? id
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? ""
        authenticated = try c.decodeIfPresent(Bool.self, forKey: .authenticated) ?? false
        consented = try c.decodeIfPresent(Bool.self, forKey: .consented) ?? false
        detail = try c.decodeIfPresent(String.self, forKey: .detail)
        headroom = try c.decodeIfPresent(HeadroomView.self, forKey: .headroom) ?? .unknown
        health = try c.decodeIfPresent(HealthView.self, forKey: .health) ?? HealthView()
        models = try c.decodeIfPresent([String].self, forKey: .models) ?? []
        connectCommand = try c.decodeIfPresent(String.self, forKey: .connectCommand)
    }

    public init(
        id: String, name: String, kind: String = "subscription", authenticated: Bool = true,
        consented: Bool = true, detail: String? = nil, headroom: HeadroomView = .unknown,
        health: HealthView = HealthView(), models: [String] = [], connectCommand: String? = nil
    ) {
        self.id = id
        self.name = name
        self.kind = kind
        self.authenticated = authenticated
        self.consented = consented
        self.detail = detail
        self.headroom = headroom
        self.health = health
        self.models = models
        self.connectCommand = connectCommand
    }
}

/// A backend's circuit state, flattened for display.
public struct HealthView: Decodable, Sendable, Equatable {
    /// `closed` / `open` / `half_open`. A string rather than an enum: an
    /// unrecognised state must not throw, and there is nothing to decide from it
    /// here beyond what to draw.
    public let circuit: String
    /// Consecutive failures counted against this backend's health.
    public let consecutiveFailures: Int
    /// Seconds until an open circuit will next allow a probe.
    public let retryInSecs: Int?

    private enum CodingKeys: String, CodingKey {
        case circuit, consecutiveFailures, retryInSecs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        circuit = try c.decodeIfPresent(String.self, forKey: .circuit) ?? "closed"
        consecutiveFailures = try c.decodeIfPresent(Int.self, forKey: .consecutiveFailures) ?? 0
        retryInSecs = try c.decodeIfPresent(Int.self, forKey: .retryInSecs)
    }

    public init(circuit: String = "closed", consecutiveFailures: Int = 0, retryInSecs: Int? = nil) {
        self.circuit = circuit
        self.consecutiveFailures = consecutiveFailures
        self.retryInSecs = retryInSecs
    }

    /// Whether this backend is being skipped entirely.
    public var isOpen: Bool { circuit == "open" }

    /// Whether it is being probed back into service.
    public var isRecovering: Bool { circuit == "half_open" || circuit == "halfopen" }
}

/// Observed capacity, flattened for display.
///
/// `unknown` is a real answer, not a missing one, and the app is required to
/// render it as the word `unknown` with no bar. See `capacityFraction`.
public enum HeadroomView: Decodable, Sendable, Equatable {
    /// The provider reported a number, this long ago.
    case observed(usedPct: Double, observedSecsAgo: Int, resetsInSecs: Int?)
    /// Inside a `retry-after` window.
    case exhausted(retryInSecs: Int)
    /// Stopped by a spend cap the user set, not by the provider.
    case capReached(spentUsd: Double, capUsd: Double, resetsInSecs: Int)
    /// The provider has told us nothing.
    case unknown
    /// A state this build does not recognise, from a newer daemon. Rendered as
    /// absent — the alternative is guessing what it means.
    case unrecognised(String)

    private enum CodingKeys: String, CodingKey {
        case state, usedPct, observedSecsAgo, resetsInSecs, retryInSecs, spentUsd, capUsd
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let state = try c.decodeIfPresent(String.self, forKey: .state) ?? "unknown"
        switch state {
        case "observed":
            self = .observed(
                usedPct: try c.decodeIfPresent(Double.self, forKey: .usedPct) ?? 0,
                observedSecsAgo: try c.decodeIfPresent(Int.self, forKey: .observedSecsAgo) ?? 0,
                resetsInSecs: try c.decodeIfPresent(Int.self, forKey: .resetsInSecs)
            )
        case "exhausted":
            self = .exhausted(retryInSecs: try c.decodeIfPresent(Int.self, forKey: .retryInSecs) ?? 0)
        case "cap_reached":
            self = .capReached(
                spentUsd: try c.decodeIfPresent(Double.self, forKey: .spentUsd) ?? 0,
                capUsd: try c.decodeIfPresent(Double.self, forKey: .capUsd) ?? 0,
                resetsInSecs: try c.decodeIfPresent(Int.self, forKey: .resetsInSecs) ?? 0
            )
        case "unknown":
            self = .unknown
        default:
            self = .unrecognised(state)
        }
    }
}

/// Every pool, seen as one balance.
///
/// A count of pools rather than a merged number, for the reason `BalanceView`
/// gives in `control.rs`: a five-hour window, a weekly window and a dollar
/// balance have no shared unit.
public struct BalanceView: Decodable, Sendable, Equatable {
    /// Pools that could serve a request right now.
    public let available: Int
    /// Available pools whose marginal cost is zero.
    public let freeAvailable: Int
    /// Authenticated and consented, but not yet reporting capacity.
    public let unknown: Int
    /// Exhausted, or with an open circuit.
    public let unavailable: Int
    /// When the first unavailable pool is expected back.
    public let nextAvailableAt: Date?
    /// Spend on *metered* backends today. `nil` means unmeasured, not zero.
    public let spendTodayUsd: Double?
    /// Cache reads as a fraction of every prompt token in the window.
    public let cacheHitRate: Double?
    /// Exchanges summarised for that rate, so the figure carries its basis.
    public let cacheExchanges: Int
    /// The configured daily spend cap and progress against it.
    public let spendCap: SpendCapView?
    /// What each subscription has used of its own window.
    public let subscriptionUsed: [SubscriptionUse]

    private enum CodingKeys: String, CodingKey {
        case available, freeAvailable, unknown, unavailable, nextAvailableAt
        case spendTodayUsd, cacheHitRate, cacheExchanges, spendCap, subscriptionUsed
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        available = try c.decodeIfPresent(Int.self, forKey: .available) ?? 0
        freeAvailable = try c.decodeIfPresent(Int.self, forKey: .freeAvailable) ?? 0
        unknown = try c.decodeIfPresent(Int.self, forKey: .unknown) ?? 0
        unavailable = try c.decodeIfPresent(Int.self, forKey: .unavailable) ?? 0
        nextAvailableAt = try c.decodeIfPresent(Timestamp.self, forKey: .nextAvailableAt)?.date
        spendTodayUsd = try c.decodeIfPresent(Double.self, forKey: .spendTodayUsd)
        cacheHitRate = try c.decodeIfPresent(Double.self, forKey: .cacheHitRate)
        cacheExchanges = try c.decodeIfPresent(Int.self, forKey: .cacheExchanges) ?? 0
        spendCap = try c.decodeIfPresent(SpendCapView.self, forKey: .spendCap)
        subscriptionUsed = try c.decodeIfPresent([SubscriptionUse].self, forKey: .subscriptionUsed) ?? []
    }

    public init(
        available: Int = 0, freeAvailable: Int = 0, unknown: Int = 0, unavailable: Int = 0,
        nextAvailableAt: Date? = nil, spendTodayUsd: Double? = nil, cacheHitRate: Double? = nil,
        cacheExchanges: Int = 0, spendCap: SpendCapView? = nil,
        subscriptionUsed: [SubscriptionUse] = []
    ) {
        self.available = available
        self.freeAvailable = freeAvailable
        self.unknown = unknown
        self.unavailable = unavailable
        self.nextAvailableAt = nextAvailableAt
        self.spendTodayUsd = spendTodayUsd
        self.cacheHitRate = cacheHitRate
        self.cacheExchanges = cacheExchanges
        self.spendCap = spendCap
        self.subscriptionUsed = subscriptionUsed
    }
}

/// A configured spend cap, and progress against it.
public struct SpendCapView: Decodable, Sendable, Equatable {
    /// Spent against the cap in this window.
    public let spentUsd: Double
    /// The cap.
    public let capUsd: Double

    public init(spentUsd: Double, capUsd: Double) {
        self.spentUsd = spentUsd
        self.capUsd = capUsd
    }
}

/// One subscription's consumption of its own window.
public struct SubscriptionUse: Decodable, Sendable, Equatable, Identifiable {
    /// Backend id, e.g. `claude-work`. Empty from a daemon that predates
    /// several logins of one product.
    public let backend: String
    /// Display name, e.g. `Claude subscription`.
    public let name: String
    /// Percent of the window consumed, as the provider reported it. `nil` means
    /// the provider has not said — never a guess.
    public let usedPct: Double?
    /// Exchanges served in the last 24 hours, from the local ledger.
    public let exchanges: Int

    /// The backend id, because a display name need not be unique once one
    /// product has two logins, and `ForEach` gives undefined results for a
    /// repeated identity. The name only stands in for a daemon that does not
    /// send the id, which had one login per product.
    public var id: String { backend.isEmpty ? name : backend }

    private enum CodingKeys: String, CodingKey {
        case backend, name, usedPct, exchanges
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        backend = try c.decodeIfPresent(String.self, forKey: .backend) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? backend
        usedPct = try c.decodeIfPresent(Double.self, forKey: .usedPct)
        exchanges = try c.decodeIfPresent(Int.self, forKey: .exchanges) ?? 0
    }

    public init(backend: String = "", name: String, usedPct: Double?, exchanges: Int) {
        self.backend = backend
        self.name = name
        self.usedPct = usedPct
        self.exchanges = exchanges
    }
}

/// The most recent routing decision.
public struct LastRouteView: Decodable, Sendable, Equatable {
    /// Backend chosen.
    public let backend: String
    /// Model sent upstream, when policy named one.
    public let model: String?
    /// Backend the conversation was on before, when this was a change.
    public let from: String?
    /// How far down the ladder this route sits.
    ///
    /// Read rather than inferred. Working "is this degraded" out from backend
    /// names would be a second implementation of a routing question, in a
    /// language that cannot see the policy.
    public let rung: Rung
    /// When it happened.
    public let at: Date

    private enum CodingKeys: String, CodingKey {
        case backend, model, from, rung, at
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        backend = try c.decodeIfPresent(String.self, forKey: .backend) ?? ""
        model = try c.decodeIfPresent(String.self, forKey: .model)
        from = try c.decodeIfPresent(String.self, forKey: .from)
        // Matches `Rung::default()`, so a daemon from before the field existed
        // is read as the undegraded case rather than as a fallback that never
        // happened.
        rung = try c.decodeIfPresent(Rung.self, forKey: .rung) ?? .preferred
        at = try c.decodeIfPresent(Timestamp.self, forKey: .at)?.date ?? Date(timeIntervalSince1970: 0)
    }

    public init(backend: String, model: String? = nil, from: String? = nil,
                rung: Rung = .preferred, at: Date = Date()) {
        self.backend = backend
        self.model = model
        self.from = from
        self.rung = rung
        self.at = at
    }
}

/// How far down the fidelity ladder a route sits (`docs/DESIGN.md` §3).
public enum Rung: Decodable, Sendable, Equatable {
    /// Preferred backend, preferred model.
    case preferred
    /// Same account, smaller model.
    case smallerModel
    /// Same wire format, different credential.
    case alternateCredential
    /// Different API family: cache cold, reasoning dropped, translation required.
    case crossFamily
    /// A rung this build does not know about, from a newer daemon.
    case unrecognised(String)

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        switch raw {
        case "preferred": self = .preferred
        case "smaller_model": self = .smallerModel
        case "alternate_credential": self = .alternateCredential
        case "cross_family": self = .crossFamily
        default: self = .unrecognised(raw)
        }
    }

    /// Whether this route preserved everything.
    ///
    /// An unrecognised rung counts as degraded. `preferred` is the only
    /// undegraded value and it is known, so anything else — including a rung
    /// added after this build shipped — is a descent. Reporting "fine" is the
    /// one answer that would definitely be wrong.
    public var isDegraded: Bool { self != .preferred }

    /// Whether this is the descent IronWire is obliged to announce.
    ///
    /// Deliberately narrow, and deliberately the same test as
    /// `Rung::is_user_visible` in `policy.rs`: rungs 0–2 change nothing the user
    /// can observe, and announcing them trains people to ignore the channel.
    public var isUserVisible: Bool { self == .crossFamily }

    /// How to say it in a menu.
    public var label: String {
        switch self {
        case .preferred: return "preferred"
        case .smallerModel: return "smaller model"
        case .alternateCredential: return "alternate credential"
        case .crossFamily: return "different model family"
        case .unrecognised(let raw): return raw
        }
    }
}

/// What the user should be told about releases, if anything.
///
/// Notify-only in every case. IronWire never applies an update to itself: it is
/// a daemon in the critical path holding streamed responses and the user's
/// credentials (`docs/UPDATES.md` §1). There is deliberately no case here that
/// an install button could hang off.
public enum UpdateStatus: Decodable, Sendable, Equatable {
    /// Running the latest release, or newer.
    case upToDate
    /// A newer release exists.
    case available(latest: String, summary: String?, upgradeCommand: String?)
    /// Older than the supported floor: likely broken, not merely old.
    case unsupported(latest: String, minimumSupported: String, upgradeCommand: String?)
    /// No check has succeeded yet. Not an error worth showing.
    case unknown
    /// A state this build does not recognise. Rendered as absent.
    case unrecognised(String)

    private enum CodingKeys: String, CodingKey {
        case state, latest, summary, upgradeCommand, minimumSupported
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let state = try c.decodeIfPresent(String.self, forKey: .state) ?? "unknown"
        switch state {
        case "up_to_date":
            self = .upToDate
        case "available":
            self = .available(
                latest: try c.decodeIfPresent(String.self, forKey: .latest) ?? "",
                summary: try c.decodeIfPresent(String.self, forKey: .summary),
                upgradeCommand: try c.decodeIfPresent(String.self, forKey: .upgradeCommand)
            )
        case "unsupported":
            self = .unsupported(
                latest: try c.decodeIfPresent(String.self, forKey: .latest) ?? "",
                minimumSupported: try c.decodeIfPresent(String.self, forKey: .minimumSupported) ?? "",
                upgradeCommand: try c.decodeIfPresent(String.self, forKey: .upgradeCommand)
            )
        case "unknown":
            self = .unknown
        default:
            self = .unrecognised(state)
        }
    }

    /// Whether this is worth putting in front of the user. Mirrors
    /// `UpdateStatus::is_actionable`.
    public var isActionable: Bool {
        switch self {
        case .available, .unsupported: return true
        case .upToDate, .unknown, .unrecognised: return false
        }
    }
}

// MARK: - Events

/// Something worth telling the user about, from `GET /_ironwire/events`.
///
/// The bus drops on lag by construction (`events.rs`), so these are hints that
/// state changed and never a complete history. `/_ironwire/status` is the source
/// of truth; an event's job is to make the next poll happen sooner.
public enum Event: Decodable, Sendable, Equatable {
    /// A conversation moved to a different backend.
    case routed(at: Date, conversation: String, from: String?, to: String,
                rung: Rung, translated: Bool, reason: String)
    /// A backend's circuit opened or closed.
    case health(at: Date, backend: String, circuit: String)
    /// A request could not be served at all.
    case failed(at: Date, conversation: String, detail: String)
    /// A spend cap the user set has been reached.
    case capReached(at: Date, backend: String, spentUsd: Double, capUsd: Double)
    /// An event type this build does not recognise. Ignored, not fatal.
    case unrecognised(String)

    private enum CodingKeys: String, CodingKey {
        case type, at, conversation, from, to, rung, translated, reason
        case backend, circuit, detail, spentUsd, capUsd
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decodeIfPresent(String.self, forKey: .type) ?? ""
        let at = try c.decodeIfPresent(Timestamp.self, forKey: .at)?.date ?? Date()
        switch type {
        case "routed":
            self = .routed(
                at: at,
                conversation: try c.decodeIfPresent(String.self, forKey: .conversation) ?? "",
                from: try c.decodeIfPresent(String.self, forKey: .from),
                to: try c.decodeIfPresent(String.self, forKey: .to) ?? "",
                rung: try c.decodeIfPresent(Rung.self, forKey: .rung) ?? .preferred,
                translated: try c.decodeIfPresent(Bool.self, forKey: .translated) ?? false,
                reason: try c.decodeIfPresent(String.self, forKey: .reason) ?? ""
            )
        case "health":
            self = .health(
                at: at,
                backend: try c.decodeIfPresent(String.self, forKey: .backend) ?? "",
                circuit: try c.decodeIfPresent(String.self, forKey: .circuit) ?? ""
            )
        case "failed":
            self = .failed(
                at: at,
                conversation: try c.decodeIfPresent(String.self, forKey: .conversation) ?? "",
                detail: try c.decodeIfPresent(String.self, forKey: .detail) ?? ""
            )
        case "cap_reached":
            self = .capReached(
                at: at,
                backend: try c.decodeIfPresent(String.self, forKey: .backend) ?? "",
                spentUsd: try c.decodeIfPresent(Double.self, forKey: .spentUsd) ?? 0,
                capUsd: try c.decodeIfPresent(Double.self, forKey: .capUsd) ?? 0
            )
        default:
            self = .unrecognised(type)
        }
    }
}

// MARK: - Decoding

/// A `chrono::DateTime<Utc>` off the wire.
///
/// Wrapped rather than handled with `JSONDecoder.dateDecodingStrategy` because
/// chrono emits however many fractional digits the value needs — none, three,
/// six or nine — and both `.iso8601` and a single fixed formatter reject some of
/// those. A timestamp we cannot parse costs a relative time; it must not cost
/// the status.
struct Timestamp: Decodable {
    let date: Date

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        guard let parsed = Timestamp.parse(raw) else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "unparseable timestamp: \(raw)")
            )
        }
        date = parsed
    }

    static func parse(_ raw: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: raw) { return date }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        if let date = plain.date(from: raw) { return date }

        // More fractional digits than Foundation accepts (chrono will emit nine
        // for a nanosecond-precision clock). Truncate to milliseconds rather
        // than lose the timestamp.
        if let dot = raw.firstIndex(of: "."),
           let tzStart = raw[dot...].firstIndex(where: { $0 == "Z" || $0 == "+" || $0 == "-" }) {
            let digits = raw[raw.index(after: dot)..<tzStart]
            if digits.count > 3 {
                let clipped = raw[..<dot] + "." + digits.prefix(3) + raw[tzStart...]
                return fractional.date(from: String(clipped))
            }
        }
        return nil
    }
}

/// The decoder every control-API response goes through.
///
/// `convertFromSnakeCase` matches serde's default on the Rust side; the tagged
/// enums' discriminants (`state`, `type`) are single words and pass through
/// unchanged.
public func controlDecoder() -> JSONDecoder {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
}
