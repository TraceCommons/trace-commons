import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The menu-bar panel showed a dash for every value (mode, shared, kept,
/// both chart chips, "Worth a second look") while the daemon's `status`
/// answered `contribution_mode: "mixed"` and the bars still drew. Every one
/// of those values was gated on one `stale` bit that any of six reads, or
/// the event stream ending, set; nothing cleared it until a later full read.
/// These pin the fix: each value is gated on its own read, a stream that
/// ends is re-opened, and an unknown count reads as the core's unknown word.
@MainActor
final class MenuPanelReadFailureTests: XCTestCase {
    /// The status the daemon gave when the dashes were seen, reduced to its
    /// shape: a mixed roll-up, no override, uploads being refused.
    static let mixedStatus = #"""
    {"schema_version":"trace_commons.daemon.v1_1","logged_in":true,"tenant_id":"nearai-test",
     "paused":false,"queue_depth":449,"decisions_owed":500,
     "contribution_mode":"mixed","contribution_mode_partial":false,"contribution_override":null,
     "health":{"last_error_label":"admission-refused"}}
    """#

    private static var modeCopy: ContributionModeCopy {
        get throws { try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())) }
    }

    /// The pill's value as the panel draws it.
    private func drawnMode(_ store: MenuPanelStore) throws -> String {
        MenuPanelData.modeValue(store.modeRollup, mode: store.status?.contributionMode, copy: try Self.modeCopy)
    }

    func test_aMixedStatusDrawsMixed() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus)
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()

        XCTAssertFalse(store.stale)
        XCTAssertEqual(store.modeRollup, .mixed)
        let copy = try Self.modeCopy
        XCTAssertFalse(copy.mixed.isEmpty)
        XCTAssertEqual(try drawnMode(store), copy.mixed)
        XCTAssertNotNil(store.sharedCount)
        XCTAssertNotNil(store.keptCount)
        XCTAssertNotNil(store.flaggedCount)
    }

    /// One failed read blanks only what comes from it: the mode (status)
    /// and the waiting count (pending) still draw, and shared, whose read
    /// failed, reads as the core's unknown word rather than a dash.
    func test_aFailedHistoryReadLeavesTheLocalValuesDrawn() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, failing: ["list_history"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()

        XCTAssertFalse(store.stale, "a failed history read is not a failed status read")
        XCTAssertEqual(try drawnMode(store), try Self.modeCopy.mixed)
        XCTAssertNotNil(store.flaggedCount, "the waiting count comes from list_pending, which answered")
        XCTAssertNotNil(store.keptCount)
        XCTAssertNil(store.sharedCount, "shared comes from the history read, which failed")

        let unknown = MonitorWords.unknown
        XCTAssertFalse(unknown.isEmpty, "the core's word for a value it did not report")
        XCTAssertNotEqual(unknown, "—")
        XCTAssertEqual(MenuPanelData.countWord(store.sharedCount, unknown: unknown), unknown)
        XCTAssertEqual(MenuPanelData.countWord(store.keptCount, unknown: unknown), String(store.keptCount ?? -1))
    }

    /// Every other read failing still leaves the mode drawn.
    func test_onlyAFailedStatusReadBlanksTheMode() async throws {
        let others = ScriptedTransport(
            status: Self.mixedStatus,
            failing: ["list_projects", "list_pending", "list_kept", "list_history", "inference_calls"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: others))
        await store.load()
        XCTAssertEqual(try drawnMode(store), try Self.modeCopy.mixed)
        XCTAssertNil(store.sharedCount)
        XCTAssertNil(store.keptCount)
        XCTAssertNil(store.flaggedCount)

        let statusDown = ScriptedTransport(status: Self.mixedStatus, failing: ["status"])
        let down = MenuPanelStore(client: LiveDaemonClient(transport: statusDown))
        await down.load()
        XCTAssertTrue(down.stale, "no current status: the panel cannot vouch for anything")
        XCTAssertEqual(down.modeRollup, .none)
        XCTAssertEqual(try drawnMode(down), "—")
        XCTAssertNil(down.sharedCount)
        XCTAssertFalse(down.canChooseOverride)
    }

    /// A read that recovers clears its own unknown.
    func test_aReadThatRecoversDrawsAgain() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, failing: ["list_history"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertNil(store.sharedCount)
        transport.failing = []
        await store.load()
        XCTAssertNotNil(store.sharedCount)
    }

    /// The event stream ending used to latch the panel stale for good: `run`
    /// returned, and only a new client re-subscribed. It now re-reads and
    /// re-subscribes, so a daemon that answers again is drawn again.
    func test_aStreamThatEndsIsReopened() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus)
        transport.unreachable = true
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        store.resubscribeDelay = .milliseconds(20)
        let running = Task { await store.run() }
        defer { running.cancel() }

        // Unreachable: the read fails and the stream finishes at once.
        try await waitUntil { transport.statusCalls >= 2 }
        XCTAssertTrue(store.stale)

        transport.unreachable = false
        try await waitUntil { !store.stale }
        XCTAssertEqual(try drawnMode(store), try Self.modeCopy.mixed)
    }

    /// The panel draws through the store's per-read values, so the tests
    /// above are about what is on screen; and the strip, which draws the
    /// columns, is unavailable when they are not current.
    func test_thePanelDrawsThePerReadValues() throws {
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("private var rollup: MenuPanelData.ModeRollup { store.modeRollup }"))
        XCTAssertTrue(panel.contains("MenuPanelData.countWord(store.sharedCount, unknown: MonitorWords.unknown)"))
        XCTAssertTrue(panel.contains("MenuPanelData.countWord(store.keptCount, unknown: MonitorWords.unknown)"))
        XCTAssertTrue(panel.contains("MenuPanelData.countWord(store.flaggedCount, unknown: MonitorWords.unknown)"))
        XCTAssertTrue(panel.contains("stale: store.stale || !store.columnsCurrent"))
        XCTAssertFalse(panel.contains("stale ? \"—\""), "a count is never a bare dash")
    }

    private func waitUntil(
        _ condition: @MainActor () -> Bool, file: StaticString = #filePath, line: UInt = #line
    ) async throws {
        // Counts attempts, never wall-clock, beyond a generous bound.
        for _ in 0..<500 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("condition never held", file: file, line: line)
    }
}

/// Sample replies, with `status` replaced, chosen methods refused the way the
/// daemon refuses them, or every call answered as a stopped daemon.
private final class ScriptedTransport: DaemonTransport, @unchecked Sendable {
    private let lock = NSLock()
    private let status: String
    private var _failing: Set<String>
    private var _unreachable = false
    private var _statusCalls = 0

    init(status: String, failing: Set<String> = []) {
        self.status = status
        self._failing = failing
    }

    var failing: Set<String> {
        get { lock.withLock { _failing } }
        set { lock.withLock { _failing = newValue } }
    }

    var unreachable: Bool {
        get { lock.withLock { _unreachable } }
        set { lock.withLock { _unreachable = newValue } }
    }

    var statusCalls: Int { lock.withLock { _statusCalls } }

    func call(_ method: String, params paramsJSON: String) -> String {
        let (down, refused) = lock.withLock { () -> (Bool, Bool) in
            if method == "status" { _statusCalls += 1 }
            return (_unreachable, _failing.contains(method))
        }
        if down { return #"{"id":0,"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        if refused { return #"{"id":0,"error":{"code":"unavailable","message":"history-read-failed"}}"# }
        if method == "status" { return #"{"id":0,"result":\#(status)}"# }
        guard let reply = SampleDaemonData.reply(method, in: .normalDay) else {
            return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
        }
        return #"{"id":0,"result":\#(reply)}"#
    }
}
