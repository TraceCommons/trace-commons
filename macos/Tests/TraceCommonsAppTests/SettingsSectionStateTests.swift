import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// A daemon whose `set_consent_scopes` answers only when the test lets it,
/// so a write can be held in flight while the Settings section is switched.
///
/// The gate is waited on OUTSIDE the lock: the test reads `calls` from the
/// main actor while the write is parked, and a wait under the lock would
/// deadlock that read.
private final class GatedConsentDaemon: DaemonCalling {
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []
    private let gate = DispatchSemaphore(value: 0)
    private var confirmed: [String]
    private var refuse = false

    init(confirmed: [String]) { self.confirmed = confirmed }

    var consentWrites: [[String]] {
        lock.lock()
        defer { lock.unlock() }
        return recorded.filter { $0.method == "set_consent_scopes" }.map { call in
            let object = (try? JSONSerialization.jsonObject(with: Data(call.params.utf8))) as? [String: Any]
            return ((object?["scopes"] as? [String]) ?? []).sorted()
        }
    }

    /// Lets the parked write answer: with `scopes` as the daemon's list, or
    /// refused.
    func release(confirming scopes: [String]? = nil, refused: Bool = false) {
        lock.lock()
        if let scopes { confirmed = scopes }
        refuse = refused
        lock.unlock()
        gate.signal()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        recorded.append((method: method, params: paramsJSON))
        lock.unlock()
        guard method == "set_consent_scopes" else {
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
        gate.wait()
        lock.lock()
        defer { lock.unlock() }
        if refuse {
            return #"{"id":1,"error":{"code":"refused","message":"test-refusal"}}"#
        }
        let list = confirmed.map { "\"\($0)\"" }.joined(separator: ",")
        return #"{"id":1,"result":{"consent_scopes":["# + list + "]}}"
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// Answers `set_settings` with a declared IronWire, or refuses it.
private final class IronWireSettingsDaemon: DaemonCalling {
    let refuse: Bool
    init(refuse: Bool) { self.refuse = refuse }

    func call(_ method: String, params paramsJSON: String) -> String {
        guard method == "set_settings", !refuse else {
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
        return #"{"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,"local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,"max_uploads_per_day":100,"near_ai_configured":false,"claude_root_configured":true,"codex_root_configured":true,"ironwire":{"mode":"watch","port":9001,"token_dir":"/tmp/iw"}}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// G8 of #1229: the Settings window draws a fresh section view per section
/// (`.id(section)`), so anything a section holds as `@State` is thrown away
/// by switching section. A consent write in flight then lost its busy flag
/// (the rows came back enabled, and the next list was built from a status
/// the first write had not yet changed), and a refusal went to a view that
/// no longer existed. The write's state now lives on `AppModel`, which every
/// section view shares and none owns.
final class SettingsSectionStateTests: XCTestCase {
    private static let always = ConsentScope(
        name: "debugging_evaluation", description: "", alwaysOn: true, grantsDataUse: true)
    private static let benchmark = ConsentScope(
        name: "benchmark_only", description: "", alwaysOn: false, grantsDataUse: true)
    private static let research = ConsentScope(
        name: "research", description: "", alwaysOn: false, grantsDataUse: true)

    private func loggedIn(_ scopes: [String]) throws -> DaemonStatus {
        let list = scopes.map { "\"\($0)\"" }.joined(separator: ",")
        return try JSONDecoder().decode(DaemonStatus.self, from: Data("""
            {"logged_in":true,"consent_scopes":[\(list)]}
            """.utf8))
    }

    @MainActor
    private func waitUntil(_ condition: @MainActor () -> Bool) async throws {
        for _ in 0..<300 where !condition() {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    @MainActor
    func testASectionSwitchDuringAConsentWriteKeepsTheRowsDisabledAndTheNextListFresh() async throws {
        let daemon = GatedConsentDaemon(confirmed: [])
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try loggedIn(["debugging_evaluation", "benchmark_only"]))
        let options = [Self.always, Self.benchmark, Self.research]

        // The person unticks benchmark_only from the Consent section.
        let first = Task { await model.toggleConsentScope(Self.benchmark, granted: false, options: options) }
        try await waitUntil { daemon.consentWrites.count == 1 }
        XCTAssertEqual(daemon.consentWrites, [["debugging_evaluation"]])

        // They switch to Tools and back. The section view is a new one; what
        // it draws is the model's state, and the write is still in flight.
        XCTAssertTrue(model.consentWriteBusy, "the in-flight flag went with the discarded view")
        for scope in options {
            XCTAssertFalse(
                ConsentScopeRows.isEnabled(scope: scope, busy: model.consentWriteBusy, unavailable: !model.status.loggedIn),
                "\(scope.name) is enabled while a consent write is in flight")
        }

        // A tick on the new view while the write is in flight is not sent:
        // it would be built from a status that still holds benchmark_only.
        // (Run on its own task, so a regression that sends it parks there
        // and fails this assertion rather than hanging the test.)
        let stale = Task { await model.toggleConsentScope(Self.research, granted: true, options: options) }
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(daemon.consentWrites.count, 1, "a second write was built from stale status")
        if daemon.consentWrites.count > 1 { daemon.release(confirming: ["debugging_evaluation"]) }

        daemon.release(confirming: ["debugging_evaluation"])
        await first.value
        await stale.value
        XCTAssertFalse(model.consentWriteBusy)
        XCTAssertFalse(model.consentWriteRefused)
        XCTAssertEqual(model.status.consentScopes, ["debugging_evaluation"])

        // The next tick is built from what the daemon confirmed: the
        // withdrawn scope is not granted again.
        let second = Task { await model.toggleConsentScope(Self.research, granted: true, options: options) }
        try await waitUntil { daemon.consentWrites.count == 2 }
        XCTAssertEqual(daemon.consentWrites.last, ["debugging_evaluation", "research"])
        XCTAssertFalse(daemon.consentWrites.last?.contains("benchmark_only") ?? true)
        daemon.release(confirming: ["debugging_evaluation", "research"])
        await second.value
    }

    /// A refusal that lands after the section was switched is still drawn:
    /// it is the model's, not the discarded view's.
    @MainActor
    func testARefusalOutlivesTheSectionView() async throws {
        let daemon = GatedConsentDaemon(confirmed: [])
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try loggedIn(["debugging_evaluation", "benchmark_only"]))
        let options = [Self.always, Self.benchmark]

        let write = Task { await model.toggleConsentScope(Self.benchmark, granted: false, options: options) }
        try await waitUntil { daemon.consentWrites.count == 1 }
        daemon.release(refused: true)
        await write.value

        XCTAssertFalse(model.consentWriteBusy)
        XCTAssertTrue(model.consentWriteRefused, "the refusal went nowhere")
    }

    /// The routing draft outlives the section view, so it needs a reset of
    /// its own: once the daemon confirms the form it was applied as, the
    /// card reads the daemon again. A refused write keeps the edit.
    @MainActor
    func testTheRoutingDraftClearsOnceTheDaemonConfirmsIt() async throws {
        let applied = RoutingForm(on: true, port: 9001, tokenDir: "/tmp/iw")
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: IronWireSettingsDaemon(refuse: false)))
        model.routingDraft = applied
        model.applyIronWire(applied)
        try await waitUntil { model.daemonSettings != nil }
        XCTAssertNotNil(model.daemonSettings)
        XCTAssertNil(model.routingDraft, "a confirmed draft shadows the daemon's answer for good")

        let refused = AppModel()
        refused.setClientForTesting(DaemonClient(daemon: IronWireSettingsDaemon(refuse: true)))
        refused.routingDraft = applied
        refused.applyIronWire(applied)
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertEqual(refused.routingDraft, applied, "a refused write dropped the edit")
    }

    /// With the settings copy missing, a refusal still draws a line: the
    /// core's own request-failed sentence, and never nothing.
    func testARefusalWithNoSettingsCopyStillDrawsTheCoresLine() throws {
        let screens = try XCTUnwrap(
            MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()),
            "the core's monitor screens table did not load")
        XCTAssertEqual(
            ConsentScopeRows.refusalLine(settings: nil, screens: screens), screens.requestFailed)
        XCTAssertFalse(screens.requestFailed.isEmpty)
        XCTAssertEqual(ConsentScopeRows.refusalLine(settings: "from the core", screens: screens), "from the core")
        // With neither table, the line is the dash every surface uses for
        // "the core said nothing" -- still drawn, never an empty notice.
        XCTAssertFalse(ConsentScopeRows.refusalLine(settings: nil, screens: nil).isEmpty)
    }

    /// The other per-section state the review named: none of it may be
    /// `@State` on a view that `.id(section)` throws away.
    func testNoSectionHoldsWriteStateThatASectionSwitchDiscards() throws {
        let settings = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TraceCommonsAppTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings")
        let banned: [(file: String, names: [String])] = [
            ("ConsentSection.swift", ["busy", "saveError"]),
            ("StartupSection.swift", ["loginItemActionError"]),
            ("ToolsSection.swift", ["routingDraft"]),
            ("WitnessSection.swift", ["witnessDraft"]),
            ("WatchedFoldersSection.swift", ["busy", "saveFailed"]),
        ]
        for (file, names) in banned {
            let text = try String(contentsOf: settings.appendingPathComponent(file), encoding: .utf8)
            for name in names {
                XCTAssertFalse(
                    text.contains("@State private var \(name)"),
                    "\(file) keeps `\(name)` as @State, which a section switch discards")
            }
        }
    }
}
