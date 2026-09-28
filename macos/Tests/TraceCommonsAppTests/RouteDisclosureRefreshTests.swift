import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Answers by method, and can hold `route_disclosure` answers back so a test
/// decides the order they land in. Called from several detached tasks at
/// once, so every field is behind the lock (see CredentialCancelTests for
/// the crash an unlocked recorder causes).
private final class DisclosureDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    private var attestedBodiesValue = false
    /// When set, the next `route_disclosure` call waits on it, answering
    /// with the facts that were current when it was *made*.
    private var holdNextValue: DispatchSemaphore?

    var methods: [String] { lock.lock(); defer { lock.unlock() }; return recorded }
    var disclosureReads: Int { methods.filter { $0 == "route_disclosure" }.count }

    var attestedBodies: Bool {
        get { lock.lock(); defer { lock.unlock() }; return attestedBodiesValue }
        set { lock.lock(); defer { lock.unlock() }; attestedBodiesValue = newValue }
    }

    func holdNextDisclosure() -> DispatchSemaphore {
        let gate = DispatchSemaphore(value: 0)
        lock.lock()
        holdNextValue = gate
        lock.unlock()
        return gate
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        recorded.append(method)
        let bodies = attestedBodiesValue
        var hold: DispatchSemaphore?
        if method == "route_disclosure" {
            hold = holdNextValue
            holdNextValue = nil
        }
        if method == "set_settings",
            let data = paramsJSON.data(using: .utf8),
            let params = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let requested = params["ironwire_attested_bodies"] as? Bool
        {
            attestedBodiesValue = requested
        }
        let settingsBodies = attestedBodiesValue
        lock.unlock()

        switch method {
        case "route_disclosure":
            hold?.wait()
            return #"{"id":1,"result":"# + Self.facts(attestedBodies: bodies) + "}"
        case "set_settings", "get_settings":
            return Self.settings(attestedBodies: settingsBodies)
        case "enroll":
            return #"{"id":1,"result":{"enrolled":true,"tenant_id":"t","device_key_id":"d"}}"#
        default:
            return #"{"id":1,"result":{}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    static func facts(attestedBodies: Bool) -> String {
        """
        {"route":"witness","witness":{"state":"pinned","url":"https://witness.example",\
        "signing_address":"0xab","pinned_measurements":["mrtd=aa"],"origin":"published_at_join"},\
        "local_filter":null,"receipts":{"endpoint_configured":true,"check_attestation":false},\
        "attested_bodies":\(attestedBodies)}
        """
    }

    static func settings(attestedBodies: Bool) -> String {
        """
        {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,\
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,\
        "max_uploads_per_day":100,"near_ai_configured":false,\
        "ironwire_attested_bodies":\(attestedBodies),\
        "claude_root_configured":true,"codex_root_configured":true}}
        """
    }
}

/// The disclosure says what is true now, not what was true when the panel
/// first appeared: every write that can change a fact it states re-reads it.
final class RouteDisclosureRefreshTests: XCTestCase {
    @MainActor
    private func waitUntil(_ condition: @MainActor () -> Bool) async throws {
        for _ in 0..<300 where !condition() {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    @MainActor
    private func connectedModel(_ daemon: DisclosureDaemon) async throws -> AppModel {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshRouteDisclosure()
        try await waitUntil { model.routeDisclosure != nil }
        return model
    }

    /// The Medium: the toggle below the panel changes what the panel says.
    @MainActor
    func testTogglingInferenceEvidenceReReadsTheDisclosure() async throws {
        let daemon = DisclosureDaemon()
        let model = try await connectedModel(daemon)
        XCTAssertNil(model.routeDisclosure?.copy.attestedBodies)
        let readsBefore = daemon.disclosureReads

        await model.setInferenceEvidence(true, disclosureConfirmed: true)
        try await waitUntil { model.routeDisclosure?.copy.attestedBodies != nil }

        XCTAssertGreaterThan(daemon.disclosureReads, readsBefore)
        XCTAssertNotNil(model.routeDisclosure?.copy.attestedBodies,
                        "turning inference evidence on must add the attested-bodies line")

        await model.setInferenceEvidence(false)
        try await waitUntil { model.routeDisclosure?.copy.attestedBodies == nil }
        XCTAssertNil(model.routeDisclosure?.copy.attestedBodies,
                     "turning it off must take the line away")
    }

    /// Enrolling moves the route off `not_enrolled`.
    @MainActor
    func testEnrollingReReadsTheDisclosure() async throws {
        let daemon = DisclosureDaemon()
        let model = try await connectedModel(daemon)
        let readsBefore = daemon.disclosureReads

        guard case .succeeded = await model.enroll(invite: "i") else {
            return XCTFail("the double answers enroll with success")
        }
        try await waitUntil { daemon.disclosureReads > readsBefore }
        XCTAssertGreaterThan(daemon.disclosureReads, readsBefore)
    }

    /// Before the first read the panel is loading, not empty.
    @MainActor
    func testBeforeTheFirstReadThePanelIsLoading() {
        XCTAssertEqual(AppModel().routeDisclosureState, .loading)
    }

    /// With no daemon to ask, the panel says it could not read the route.
    @MainActor
    func testWithNoClientThePanelIsUnreadableNotBlank() {
        let model = AppModel()
        model.refreshRouteDisclosure()
        XCTAssertEqual(model.routeDisclosureState, .unreadable)
    }

    /// An older answer landing after a newer one does not replace it.
    @MainActor
    func testAnOlderReadCannotOverwriteANewerOne() async throws {
        let daemon = DisclosureDaemon()
        let model = try await connectedModel(daemon)

        let gate = daemon.holdNextDisclosure()
        model.refreshRouteDisclosure()  // held; will answer attested_bodies false
        try await waitUntil { daemon.disclosureReads == 2 }

        daemon.attestedBodies = true
        model.refreshRouteDisclosure()  // answers attested_bodies true
        try await waitUntil { model.routeDisclosure?.copy.attestedBodies != nil }
        XCTAssertNotNil(model.routeDisclosure?.copy.attestedBodies)

        gate.signal()
        // Let the stale answer land, then check it was dropped.
        for _ in 0..<30 { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertNotNil(model.routeDisclosure?.copy.attestedBodies,
                        "the held, older read overwrote the newer one")
    }
}
