import Foundation
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The menu-bar panel showed a dash for every value (mode, shared, kept,
/// both chart chips) while the daemon's `status`
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
    }

    /// One failed read blanks only what comes from it: the mode (status)
    /// and the kept count (pending) still draw, and shared, whose read
    /// failed, reads as the core's unknown word rather than a dash.
    func test_aFailedHistoryReadLeavesTheLocalValuesDrawn() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, failing: ["list_history"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()

        XCTAssertFalse(store.stale, "a failed history read is not a failed status read")
        XCTAssertEqual(try drawnMode(store), try Self.modeCopy.mixed)
        XCTAssertNotNil(store.keptCount)
        XCTAssertNil(store.sharedCount, "shared comes from the history read, which failed")

        let unknown = MonitorWords.unknown
        XCTAssertFalse(unknown.isEmpty, "the core's word for a value it did not report")
        XCTAssertNotEqual(unknown, "—")
        XCTAssertEqual(MenuPanelData.countWord(store.sharedCount, unknown: unknown), unknown)
        XCTAssertEqual(MenuPanelData.countWord(store.keptCount, unknown: unknown), String(store.keptCount ?? -1))
    }

    /// Traces calls sessions still waiting here "Kept"; explicit Keep decisions
    /// are a separate daemon list and must not replace that population.
    func test_keptLegendMatchesTracesWhenNoExplicitKeepDecisionsExist() async throws {
        let now = Date()
        let pending = try Self.waitingFixture(now: now)
        let transport = ScriptedTransport(status: Self.mixedStatus, pendingReply: pending)
        let client = LiveDaemonClient(transport: transport)
        let store = MenuPanelStore(client: client)
        store.clock = { now }
        await store.load()
        let sessions = try await client.listPending(projectId: nil)
        let traces = TracesGraphModel.buckets(
            shared: [], kept: TracesGraphModel.keptTimes(sessions, tool: nil),
            offset: 0, now: now, range: .days)
        XCTAssertEqual(store.keptCount, 464)
        XCTAssertEqual(store.buckets.map(\.kept), traces.map(\.kept))
        XCTAssertEqual(MenuPanelData.countWord(store.keptCount, unknown: "unknown"), "464")
        XCTAssertEqual(store.stripTraces.reduce(0) { $0 + $1.down }, 464,
                       "the 24-hour strip uses waiting discovery times within its own span")
        XCTAssertEqual(transport.keptCalls, 0, "explicit Keep decisions are not this graph's input")
    }

    func test_aFailedPendingReadMakesKeptUnknownWithoutBlankingShared() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, pendingReply: try Self.waitingFixture(now: Date()))
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.keptCount, 464)
        transport.failing = ["list_pending"]
        await store.load()
        XCTAssertFalse(store.stale)
        XCTAssertNil(store.keptCount, "last waiting sessions cannot look current after failure")
        XCTAssertNotNil(store.sharedCount)
        XCTAssertFalse(store.columnsCurrent)
        transport.failing = []
        await store.load()
        XCTAssertEqual(store.keptCount, 464)
    }

    func test_unusedExplicitKeepReadCannotInvalidateWaitingCounts() async throws {
        let transport = ScriptedTransport(
            status: Self.mixedStatus, failing: ["list_kept"], pendingReply: try Self.waitingFixture(now: Date()))
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.keptCount, 464)
        XCTAssertTrue(store.columnsCurrent)
        XCTAssertEqual(transport.keptCalls, 0)
    }

    private static func waitingFixture(now: Date) throws -> String {
        let formatter = ISO8601DateFormatter()
        func entry(_ id: String, discovered: Date?, started: Date? = nil) -> [String: Any] {
            var row: [String: Any] = ["entry_id": id, "source": "codex", "project_id": "project",
                                      "project_label": "Project", "state": "pending"]
            row["discovered_at"] = discovered.map(formatter.string)
            row["started_at"] = started.map(formatter.string)
            return row
        }
        let recent = now.addingTimeInterval(-60)
        let rows = (0..<464).map { entry("waiting-\($0)", discovered: recent) } + [
            entry("outside-window", discovered: now.addingTimeInterval(-20 * 86_400)),
            entry("undated", discovered: nil, started: recent)
        ]
        return String(decoding: try JSONSerialization.data(withJSONObject: ["pending": rows]), as: UTF8.self)
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

        let statusDown = ScriptedTransport(status: Self.mixedStatus, failing: ["status"])
        let down = MenuPanelStore(client: LiveDaemonClient(transport: statusDown))
        await down.load()
        XCTAssertTrue(down.stale, "no current status: the panel cannot vouch for anything")
        XCTAssertEqual(down.modeRollup, .none)
        XCTAssertEqual(try drawnMode(down), "—")
        XCTAssertNil(down.sharedCount)
        XCTAssertFalse(down.canChooseOverride)
    }

    /// The menu-bar item's Private AI lines come from `inference_activity`
    /// alone: a failed read, or an older daemon refusing it, draws none and
    /// stales nothing else. A read that answers puts the trace lines on its
    /// clock, step for step.
    func test_aFailedActivityReadDrawsNoPrivateAILinesAndStalesNothing() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, failing: ["inference_activity"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertFalse(store.stale)
        XCTAssertTrue(store.columnsCurrent, "the trace lines are still current")
        XCTAssertNil(store.stripPrivateAI)
        XCTAssertEqual(store.stripTraces.count, 6)

        transport.failing = []
        await store.load()
        let privateAI = try XCTUnwrap(store.stripPrivateAI)
        XCTAssertEqual(privateAI.count, 6)
        XCTAssertEqual(store.stripTraces.count, privateAI.count)
        XCTAssertTrue(privateAI.allSatisfy { $0.up <= $0.down }, "verified calls are some of the calls")
    }

    /// The item's 24 hours move forward with no event: the daemon's steps
    /// end at the moment of its read, so an idle daemon is re-read on a
    /// timer, never only when something happens.
    func test_theActivityIsReReadWithNoEvent() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus)
        let first = Date(timeIntervalSince1970: 1_800_000_000)
        transport.activityEnd = first
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        store.activityRefresh = .milliseconds(20)
        let running = Task { await store.run() }
        defer { running.cancel() }

        try await waitUntil { store.activity.flatMap(MenuPanelData.span)?.end == first }
        let later = first.addingTimeInterval(5 * 3_600)
        transport.activityEnd = later
        try await waitUntil { store.activity.flatMap(MenuPanelData.span)?.end == later }
        XCTAssertEqual(store.stripSpan.end, later, "the trace bars follow the Private AI steps' clock")
    }

    /// With no Private AI steps to follow (an older daemon, or a failed
    /// read), the trace bars' span still ends at the store's clock, and the
    /// clock moves on the same timer, so the bars move forward and the
    /// label is redrawn.
    func test_withoutActivityTheTraceSpanStillMovesForward() async throws {
        let transport = ScriptedTransport(status: Self.mixedStatus, failing: ["inference_activity"])
        let store = MenuPanelStore(client: LiveDaemonClient(transport: transport))
        var now = Date(timeIntervalSince1970: 1_800_000_000)
        store.clock = { now }
        store.activityRefresh = .milliseconds(20)
        let running = Task { await store.run() }
        defer { running.cancel() }

        try await waitUntil { !store.stale }
        XCTAssertNil(store.stripPrivateAI)
        XCTAssertEqual(store.stripSpan.end, now)
        now = now.addingTimeInterval(5 * 3_600)
        try await waitUntil { store.stripSpan.end == now }
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
        // No mode pill (owner, 2026-10-19), so no roll-up is drawn.
        XCTAssertFalse(panel.contains("store.modeRollup"))
        // The legend counts the hovered day only while its read is current
        // (owner, 2026-10-22: the app's graph, whose legend follows the
        // pointer).
        XCTAssertTrue(panel.contains("let shared = store.sharedCount.map { shown?.shared ?? $0 }"))
        XCTAssertTrue(panel.contains("let kept = store.keptCount.map { shown?.kept ?? $0 }"))
        XCTAssertTrue(panel.contains("MenuPanelData.countWord(shared, unknown: MonitorWords.unknown)"))
        XCTAssertTrue(panel.contains("MenuPanelData.countWord(kept, unknown: MonitorWords.unknown)"))
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
    private let pendingReply: String?
    private var _keptCalls = 0
    private var _failing: Set<String>
    private var _unreachable = false
    private var _statusCalls = 0
    private var _activityEnd: Date?

    init(status: String, failing: Set<String> = [], pendingReply: String? = nil) {
        self.pendingReply = pendingReply
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
    var keptCalls: Int { lock.withLock { _keptCalls } }

    /// When set, `inference_activity` answers with steps ending here, as the
    /// daemon's end at the moment of its read.
    var activityEnd: Date? {
        get { lock.withLock { _activityEnd } }
        set { lock.withLock { _activityEnd = newValue } }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        let (down, refused) = lock.withLock { () -> (Bool, Bool) in
            if method == "status" { _statusCalls += 1 }
            if method == "list_kept" { _keptCalls += 1 }
            return (_unreachable, _failing.contains(method))
        }
        if down { return #"{"id":0,"error":{"code":"unavailable","message":"daemon-stopped"}}"# }
        if refused { return #"{"id":0,"error":{"code":"unavailable","message":"history-read-failed"}}"# }
        if method == "status" { return #"{"id":0,"result":\#(status)}"# }
        if method == "list_pending", let pendingReply { return #"{"id":0,"result":\#(pendingReply)}"# }
        if method == "list_kept", pendingReply != nil { return #"{"id":0,"result":{"kept":[]}}"# }
        if method == "inference_activity", let end = activityEnd {
            return #"{"id":0,"result":\#(SampleDaemonData.inferenceActivity(.normalDay, now: end))}"#
        }
        guard let reply = SampleDaemonData.reply(method, in: .normalDay) else {
            return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
        }
        return #"{"id":0,"result":\#(reply)}"#
    }
}
