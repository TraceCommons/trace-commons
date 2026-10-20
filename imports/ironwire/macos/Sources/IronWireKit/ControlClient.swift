//! The only thing in this app that talks to the daemon.
//
// Three calls, one of which writes: `GET /_ironwire/status`,
// `GET /_ironwire/events`, and `POST /_ironwire/pin`. Everything the menu shows
// comes out of the first; the second exists so the first happens sooner.
//
// Two properties are carried over from `ironwire statusline`
// (`src/commands/statusline.rs`), which is the closest existing client:
//
// - **A daemon that is not running is a normal state.** It is not an error, it
//   does not deserve a dialog, and it must not fill anyone's console. It
//   recovers on its own when the daemon comes back.
// - **Nothing is invented on failure.** A poll that fails drops what it knew
//   rather than redrawing it as though it were current — see `forget(_:)`.

import Combine
import Foundation

@MainActor
public final class ControlClient: ObservableObject {
    /// Whether the daemon is answering, and if not, why.
    public enum Connection: Sendable, Equatable {
        /// Nothing has been tried yet.
        case connecting
        /// The last poll succeeded.
        case connected
        /// Nothing answered. The ordinary "IronWire is not running" case.
        case unreachable
        /// It answered, and rejected our token — after a re-read and one retry.
        case unauthorised
    }

    /// While the menu is closed. `/_ironwire/status` calls
    /// `BackendRegistry::statuses()`, which does a credential check per backend
    /// — a Keychain read for each Claude login — so a background poll is not free.
    public static let idleInterval: Duration = .seconds(5)
    /// While the menu is open and someone is watching numbers change.
    public static let activeInterval: Duration = .seconds(1)

    @Published public private(set) var status: StatusView?
    @Published public private(set) var connection: Connection = .connecting

    /// What can be changed, once it has been asked for.
    ///
    /// Fetched when the settings pane opens rather than on every poll: it costs
    /// another `statuses()` sweep on the daemon, and nothing in it changes
    /// unless this app changed it.
    @Published public private(set) var settings: SettingsView? {
        didSet {
            servicesById = Dictionary((settings?.services ?? []).map { ($0.id, $0) },
                                      uniquingKeysWith: { first, _ in first })
        }
    }
    /// Settings and status arrive separately; index the join once per settings
    /// snapshot instead of scanning every login for every visible row.
    public private(set) var servicesById: [String: ServiceView] = [:]

    /// Raised while the dropdown is on screen, which is the only time the fast
    /// poll is worth its Keychain reads.
    @Published public var menuIsOpen = false {
        didSet { if menuIsOpen != oldValue { pollTick?.cancel() } }
    }

    /// Called for every event the stream delivers, on the main actor. The app
    /// decides what to do with it; `NotificationPolicy` decides which ones are
    /// worth showing.
    public var onEvent: (@MainActor (Event) -> Void)?

    private var token: String?
    private var port: Int
    private let home: URL

    private var pollTask: Task<Void, Never>?
    private var streamTask: Task<Void, Never>?
    private var pollTick: Task<Void, Never>?

    private let pollSession: URLSession
    private let streamSession: URLSession
    private let decoder = controlDecoder()

    public init(home: URL = Discovery.home()) {
        self.home = home
        self.port = Discovery.port(home: home)
        self.token = Discovery.token(home: home)

        let poll = URLSessionConfiguration.ephemeral
        // Short: a status call that has not answered in this long is not going
        // to, and the next tick is seconds away.
        poll.timeoutIntervalForRequest = 4
        poll.waitsForConnectivity = false
        pollSession = URLSession(configuration: poll)

        let stream = URLSessionConfiguration.ephemeral
        // The event stream is *meant* to sit idle. A quiet system is the normal
        // one, and a timeout here would tear down a working connection every
        // time nobody was routing anything.
        stream.timeoutIntervalForRequest = TimeInterval(Int32.max)
        stream.timeoutIntervalForResource = TimeInterval(Int32.max)
        stream.waitsForConnectivity = false
        streamSession = URLSession(configuration: stream)
    }

    /// A client holding a fixed status and talking to nothing.
    ///
    /// The seam the dropdown is tested through. Several states the menu has to
    /// render correctly — an open circuit, a cross-family route, an unobserved
    /// backend — are ones a real daemon will not produce on demand, and the one
    /// that matters most (no bar for an unknown headroom) is the one the issue
    /// warns is most likely to be quietly wrong.
    public static func fixture(
        status: StatusView?, connection: Connection = .connected, settings: SettingsView? = nil
    ) -> ControlClient {
        let client = ControlClient(home: URL(fileURLWithPath: "/nonexistent"))
        client.status = status
        client.connection = connection
        client.settings = settings
        return client
    }

    // MARK: - Lifecycle

    /// Begin polling and listening. Idempotent.
    public func start() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in await self?.pollLoop() }
        streamTask = Task { [weak self] in await self?.streamLoop() }
    }

    /// Stop everything. Quitting must not leave a task polling a daemon on
    /// somebody's behalf.
    public func stop() {
        pollTask?.cancel()
        streamTask?.cancel()
        pollTick?.cancel()
        pollTask = nil
        streamTask = nil
        pollTick = nil
    }

    deinit {
        pollTask?.cancel()
        streamTask?.cancel()
        pollTick?.cancel()
    }

    // MARK: - Polling

    private func pollLoop() async {
        while !Task.isCancelled {
            await refresh()
            let interval = menuIsOpen ? Self.activeInterval : Self.idleInterval
            // Held in a field so opening the menu can cut the current wait
            // short instead of leaving the user watching stale numbers for the
            // rest of a five-second tick.
            let tick = Task<Void, Never> { try? await Task.sleep(for: interval) }
            pollTick = tick
            await tick.value
        }
    }

    /// Fetch the status now.
    public func refresh() async {
        guard let request = authorised(path: "/status") else {
            // No token file at all: the daemon has never run here. This app does
            // not mint one — a GUI creating the credential for a daemon that
            // may not exist is not its business.
            //
            // Re-read first, because the ordinary way to reach this state is an
            // app that was launched before `ironwire serve` ever ran: the token
            // is written when the daemon first starts, and a poll loop that only
            // looked once would never notice it appear.
            reloadCredentials()
            guard let retried = authorised(path: "/status") else {
                forget(.unreachable)
                return
            }
            await deliver(await send(retried, retryingUnauthorised: true))
            return
        }

        await deliver(await send(request, retryingUnauthorised: true))
    }

    /// Turn one call's outcome into what the menu shows.
    private func deliver(_ outcome: Outcome) async {
        switch outcome {
        case .success(let data):
            guard let decoded = try? decoder.decode(StatusView.self, from: data) else {
                // A body we cannot read tells us nothing about the daemon's
                // current state, so we stop claiming to know it.
                forget(.unreachable)
                return
            }
            status = decoded
            connection = .connected
            // The daemon is the authority on its own port; if it moved, later
            // polls follow it without the app being restarted.
            port = decoded.port
        case .unauthorised:
            forget(.unauthorised)
        case .failure:
            forget(.unreachable)
        }
    }

    /// Drop the last known status, because it is no longer known.
    ///
    /// Keeping it would be the more forgiving-looking choice and the wrong one.
    /// Every number on that screen is an *observation with an age*, and the ages
    /// are computed by the daemon: with nothing answering, a menu built from the
    /// last reply goes on saying "observed 12s ago" indefinitely, and the icon
    /// goes on reporting a route that may have been superseded. That is a
    /// fabricated present tense, which is the one thing this whole surface
    /// exists not to do (`docs/CRITIQUE.md` §4).
    ///
    /// The cost is a menu that briefly says "not running" for a poll that merely
    /// timed out. That is the honest reading of not having heard back, and the
    /// next tick is five seconds away.
    private func forget(_ reason: Connection) {
        status = nil
        connection = reason
    }

    // MARK: - Events

    private func streamLoop() async {
        // Backoff so a daemon that is down does not get hammered, capped so a
        // daemon that comes back is picked up promptly.
        var backoff: Duration = .seconds(1)
        let ceiling: Duration = .seconds(30)

        while !Task.isCancelled {
            let connected = await consumeEvents()
            if Task.isCancelled { return }
            if connected {
                // It worked at least once, so the next outage starts over.
                backoff = .seconds(1)
            }
            try? await Task.sleep(for: backoff)
            backoff = min(backoff * 2, ceiling)
            // Re-read both in case the daemon was reconfigured while it was
            // down. Restarting the app to find a new port would be a poor
            // answer for something we can simply check.
            reloadCredentials()
        }
    }

    /// Hold the stream open. Returns whether it ever connected.
    private func consumeEvents() async -> Bool {
        guard let request = authorised(path: "/events") else { return false }
        var decoder = SSEDecoder()
        do {
            let (bytes, response) = try await streamSession.bytes(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                if (response as? HTTPURLResponse)?.statusCode == 401 { reloadCredentials() }
                return false
            }
            for try await line in bytes.lines {
                if Task.isCancelled { return true }
                switch decoder.feed(line) {
                case .event(let payload):
                    guard let data = payload.data(using: .utf8),
                          let event = try? self.decoder.decode(Event.self, from: data)
                    else { continue }
                    onEvent?(event)
                    // An event means the status just changed. Fetching it now is
                    // what makes the icon move when the route does, rather than
                    // up to five seconds later.
                    await refresh()
                case .comment, .pending:
                    // `: connected` and `: lagged N`. Both are framing, and the
                    // lag one is a reminder that this stream is not a history —
                    // which is fine, because the poll above is the truth.
                    continue
                }
            }
            return true
        } catch {
            // A dropped stream is the normal end of a daemon restart.
            return false
        }
    }

    // MARK: - Settings

    /// Fetch what can be changed.
    @discardableResult
    public func refreshSettings() async -> Bool {
        guard let request = authorised(path: "/settings") else { return false }
        guard case .success(let data) = await send(request, retryingUnauthorised: true),
              let decoded = try? decoder.decode(SettingsView.self, from: data)
        else { return false }
        settings = decoded
        return true
    }

    /// What a settings write reported back.
    public struct WriteOutcome: Sendable, Equatable {
        /// A warning the daemon attached to an otherwise successful change —
        /// the privacy mode applied but could not be saved, say. Not an error:
        /// the change is in force, and the user is told what did not happen.
        public let warning: String?
    }

    /// Change the privacy mode.
    ///
    /// The app does not decide whether the mode is allowed — it offers what the
    /// daemon marked selectable, and the daemon refuses anything else with a
    /// sentence worth showing.
    public func setPrivacyMode(_ mode: String) async -> Result<WriteOutcome, PinError> {
        guard var request = authorised(path: "/privacy") else {
            return .failure(PinError(message: "IronWire is not running"))
        }
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["mode": mode])

        switch await send(request, retryingUnauthorised: true) {
        case .success(let data):
            let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            await refreshSettings()
            await refresh()
            return .success(WriteOutcome(warning: body?["warning"] as? String))
        case .unauthorised:
            connection = .unauthorised
            return .failure(PinError(message: "the control token was rejected"))
        case .failure(let message):
            return .failure(PinError(message: message ?? "could not reach the IronWire daemon"))
        }
    }

    /// Record or withdraw consent for a subscription backend.
    ///
    /// `promptVersion` is the version of the question the user was actually
    /// shown, and the daemon checks it. This app must never send a version it
    /// merely knows about — the whole point is that an answer belongs to the
    /// wording it answered.
    public func setConsent(
        backend: String, granted: Bool, promptVersion: Int, binding: String? = nil
    ) async -> Result<Void, PinError> {
        guard var request = authorised(path: "/consent") else {
            return .failure(PinError(message: "IronWire is not running"))
        }
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        var body: [String: Any] = ["backend": backend, "granted": granted,
                                    "prompt_version": promptVersion]
        if let binding { body["binding"] = binding }
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)

        switch await send(request, retryingUnauthorised: true) {
        case .success:
            await refreshSettings()
            await refresh()
            return .success(())
        case .unauthorised:
            connection = .unauthorised
            return .failure(PinError(message: "the control token was rejected"))
        case .failure(let message):
            return .failure(PinError(message: message ?? "could not reach the IronWire daemon"))
        }
    }

    // MARK: - Tools

    /// What wiring a tool did *not* do.
    ///
    /// The rest of the daemon's report — the file, the keys it set, where it
    /// put the backup — is the file the menu already names in the tool's
    /// tooltip, said again. This is the part that is not on screen anywhere: a
    /// slot the user was already using is left alone, so a tool can come back
    /// checked with part of its config still pointing somewhere else, and
    /// nothing but a sentence can say that.
    public struct ToolOutcome: Sendable, Equatable {
        /// Slots left alone because the user was already using them.
        public let occupied: [String]
    }

    /// Point a coding agent at IronWire, or take it back off.
    public func setTool(id: String, connect: Bool) async -> Result<ToolOutcome, PinError> {
        guard var request = authorised(path: "/tools") else {
            return .failure(PinError(message: "IronWire is not running"))
        }
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: [
            "id": id,
            "connect": connect,
        ])

        switch await send(request, retryingUnauthorised: true) {
        case .success(let data):
            let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            await refreshSettings()
            return .success(
                ToolOutcome(
                    occupied: ((body?["occupied"] as? [[String: Any]]) ?? []).map { entry in
                        let slot = entry["slot"] as? String ?? "a setting"
                        let current = entry["current"] as? String ?? "a value of your own"
                        return "\(slot) is already `\(current)`, so IronWire left it"
                    }))
        case .unauthorised:
            connection = .unauthorised
            return .failure(PinError(message: "the control token was rejected"))
        case .failure(let message):
            return .failure(PinError(message: message ?? "could not reach the IronWire daemon"))
        }
    }

    // MARK: - Pin

    /// What went wrong with a pin, in words a menu can show.
    public struct PinError: Error, Sendable, Equatable {
        public let message: String
    }

    /// Force all traffic onto a backend, or clear the force by passing `nil`.
    ///
    /// The only write this app makes. It does not decide *which* backend is
    /// right — it offers the ones the daemon reported and sends back what the
    /// user picked. Validation lives in `pin()` in `control.rs`, where the
    /// backend list actually is.
    public func pin(backend: String?) async -> Result<Void, PinError> {
        guard var request = authorised(path: "/pin") else {
            return .failure(PinError(message: "IronWire is not running"))
        }
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try? JSONSerialization.data(
            withJSONObject: ["backend": backend as Any, "model": NSNull()]
        )

        switch await send(request, retryingUnauthorised: true) {
        case .success(let data):
            // Reflect the result rather than assume it: the next poll is what
            // the menu believes, and `status.pin` is what it shows.
            await refresh()
            _ = data
            return .success(())
        case .unauthorised:
            connection = .unauthorised
            return .failure(PinError(message: "the control token was rejected"))
        case .failure(let message):
            // The daemon knows which backends exist and says so in the body;
            // passing that through beats "400".
            return .failure(PinError(message: message ?? "could not reach the IronWire daemon"))
        }
    }

    // MARK: - Transport

    private enum Outcome {
        case success(Data)
        case unauthorised
        case failure(String?)
    }

    private func send(_ request: URLRequest, retryingUnauthorised: Bool) async -> Outcome {
        do {
            let (data, response) = try await pollSession.data(for: request)
            guard let http = response as? HTTPURLResponse else { return .failure(nil) }
            switch http.statusCode {
            case 200..<300:
                return .success(data)
            case 401:
                // The token is rotated by whoever restarts the daemon, not by
                // us, so a 401 usually means the file on disk moved on. Re-read
                // it and try exactly once more before saying anything.
                guard retryingUnauthorised else { return .unauthorised }
                reloadCredentials()
                guard let retried = reauthorised(request) else { return .unauthorised }
                return await send(retried, retryingUnauthorised: false)
            default:
                let detail = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                return .failure(detail?["error"] as? String)
            }
        } catch {
            return .failure(nil)
        }
    }

    private func authorised(path: String) -> URLRequest? {
        guard let token else { return nil }
        var request = URLRequest(url: Discovery.controlURL(port: port, path: path))
        request.setValue("Bearer \(token)", forHTTPHeaderField: "authorization")
        return request
    }

    private func reauthorised(_ original: URLRequest) -> URLRequest? {
        guard let token else { return nil }
        var request = original
        request.setValue("Bearer \(token)", forHTTPHeaderField: "authorization")
        return request
    }

    private func reloadCredentials() {
        token = Discovery.token(home: home)
        port = Discovery.port(home: home)
    }
}
