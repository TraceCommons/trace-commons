import AppKit
import SwiftUI
@testable import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Traces tree at the owner's scale (231 watched folders, 180 sessions
/// waiting), counted rather than timed: how many rows a change rebuilds.
/// Every row built costs layout on the main thread (about 0.7 ms a folder
/// row at this scale, `TracesScalePerfTests`), so these bounds are what
/// keeps the tab smooth. Loose on purpose: they fail on a tree that builds
/// every row again, never on a row or two more than today.
@MainActor
final class TracesTreeScaleTests: XCTestCase {
    typealias Perf = TracesScalePerfTests

    private func hosted(_ store: TracesStore, box: Perf.SelectionBox) -> Perf.Host {
        let host = Perf.Host(Perf.Hosted(store: store, box: box, model: AppModel()))
        host.draw()
        _ = Perf.settle(host) { true }
        Perf.breakdown = []
        return host
    }

    /// Pumps the main run loop for `seconds`, drawing as it goes, so a
    /// scroll animation and anything scheduled after a change lands.
    private func drain(_ host: Perf.Host, _ seconds: Double = 0.3) {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            host.draw()
        }
    }

    private func loaded() async throws -> (TracesStore, SampleDaemonClient) {
        let client = try Perf.client()
        let store = TracesStore(client: client)
        await store.load()
        XCTAssertEqual(store.tree.folders.count, 231)
        return (store, client)
    }

    /// Opening the tab builds the rows on screen, not all 231.
    func test_openingTheTabBuildsOnlyTheRowsOnScreen() async throws {
        let (store, _) = try await loaded()
        TracesRowBodies.reset()
        let host = hosted(store, box: Perf.SelectionBox())
        defer { host.close() }
        print("COUNT open rows=\(TracesRowBodies.total)")
        XCTAssertGreaterThan(TracesRowBodies.folders, 0, "the tree drew nothing")
        XCTAssertLessThan(TracesRowBodies.total, 80, "every folder row was built")
    }

    /// A click rebuilds the row it left and the row it selected.
    func test_selectingARowRebuildsOnlyTheRowsItChanged() async throws {
        let (store, _) = try await loaded()
        let box = Perf.SelectionBox()
        let host = hosted(store, box: box)
        defer { host.close() }
        var counts: [Int] = []
        for folder in store.tree.folders.prefix(6) {
            TracesRowBodies.reset()
            box.selection = .folder(projectID: folder.id)
            drain(host)
            counts.append(TracesRowBodies.total)
        }
        print("COUNT select rows=\(counts)")
        XCTAssertLessThanOrEqual(counts.max() ?? 0, 6, "a click rebuilt rows it did not change: \(counts)")
    }

    /// A reload that changes one session rebuilds its folder's row, not
    /// the tree.
    func test_aReloadThatChangesOneSessionRebuildsOneRow() async throws {
        let (store, client) = try await loaded()
        let host = hosted(store, box: Perf.SelectionBox())
        defer { host.close() }
        var counts: [Int] = []
        for bump in 1...3 {
            client.replaceReply("list_pending", with: try Perf.replies(bump: bump).pending)
            TracesRowBodies.reset()
            await store.load()
            drain(host)
            counts.append(TracesRowBodies.total)
        }
        print("COUNT reload rows=\(counts)")
        XCTAssertLessThanOrEqual(counts.max() ?? 0, 4, "a one-session reload rebuilt the tree: \(counts)")
        XCTAssertGreaterThanOrEqual(counts.min() ?? 0, 1, "the changed folder was not redrawn: \(counts)")
    }

    /// The window redraws for reasons of its own (its other state, the app
    /// model): the tree's rows are not rebuilt for them.
    func test_aWindowRedrawRebuildsNoRow() async throws {
        let (store, _) = try await loaded()
        let box = Perf.SelectionBox()
        let host = Perf.Host(Perf.WindowLike(store: store, box: box, model: AppModel()))
        defer { host.close() }
        host.draw()
        drain(host)
        var counts: [Int] = []
        for _ in 0..<3 {
            TracesRowBodies.reset()
            box.unrelated += 1
            drain(host)
            counts.append(TracesRowBodies.total)
        }
        print("COUNT window rows=\(counts)")
        XCTAssertEqual(counts.max(), 0, "an unrelated window redraw rebuilt rows: \(counts)")
    }

    /// A session selected from elsewhere, in a folder far down the list and
    /// closed: its folder opens and the tree scrolls to it, so its row is
    /// built even though it was off screen.
    func test_aSelectionOffScreenIsRevealedAndBuilt() async throws {
        let (store, _) = try await loaded()
        let box = Perf.SelectionBox()
        let host = hosted(store, box: box)
        defer { host.close() }
        let folder = try XCTUnwrap(store.tree.folders.last { !$0.sessions.isEmpty })
        let session = try XCTUnwrap(folder.sessions.last)
        let id = TracesTreeView.rowID(.session(entryID: session.entryId))
        TracesRowBodies.reset()
        box.selection = .session(entryID: session.entryId)
        drain(host, 0.8)
        XCTAssertTrue(TracesRowBodies.built.contains(id), "the selected session's row was never built")
    }

    /// The keyboard walks down past the rows on screen: the tree follows,
    /// building the row it moved to.
    func test_aFolderOffScreenIsScrolledToAndBuilt() async throws {
        let (store, _) = try await loaded()
        let box = Perf.SelectionBox()
        let host = hosted(store, box: box)
        defer { host.close() }
        let last = try XCTUnwrap(store.tree.folders.last)
        let id = TracesTreeView.rowID(.folder(projectID: last.id))
        XCTAssertFalse(TracesRowBodies.built.contains(id), "the last folder was built before it was on screen")
        TracesRowBodies.reset()
        box.selection = .folder(projectID: last.id)
        drain(host, 0.8)
        XCTAssertTrue(TracesRowBodies.built.contains(id), "the selected folder's row was never built")
    }
}

/// The sample day over the live client, with one method held until the
/// test lets it answer: a review action in flight, for as long as the
/// test looks at the tree.
private final class HeldMethodTransport: DaemonTransport, @unchecked Sendable {
    let set: SampleDaemonClient.SampleSet
    let gated: String
    private let lock = NSLock()
    private let gate = DispatchSemaphore(value: 0)
    private var held = false

    init(_ set: SampleDaemonClient.SampleSet, gated: String) {
        self.set = set
        self.gated = gated
    }

    /// Whether the gated call has arrived and is being held.
    var holding: Bool { lock.withLock { held } }

    func release() { gate.signal() }

    func call(_ method: String, params paramsJSON: String) -> String {
        if method == gated {
            lock.withLock { held = true }
            gate.wait()
            lock.withLock { held = false }
        }
        return SampleDaemonData.reply(method, in: set).map { #"{"id":0,"result":\#($0)}"# }
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}

@MainActor
final class TracesRowBusyTests: XCTestCase {
    /// A review action dims its row and turns its pill off at once, before
    /// the daemon answers; the row is still drawn, since nothing is applied
    /// until the daemon's answer is read.
    func test_aReviewActionMarksItsRowAtOnce() async throws {
        let transport = HeldMethodTransport(.normalDay, gated: "keep")
        let store = TracesStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        let entry = try XCTUnwrap(store.tree.allSessions.first)
        let id = TracesTreeView.rowID(.session(entryID: entry.entryId))
        let box = TracesScalePerfTests.SelectionBox()
        box.selection = .session(entryID: entry.entryId)
        let host = TracesScalePerfTests.Host(TracesScalePerfTests.Hosted(store: store, box: box, model: AppModel()))
        defer { host.close() }
        host.draw()
        drain(host)
        XCTAssertFalse(store.acting.contains(entry.entryId))

        TracesRowBodies.reset()
        let action = Task { await store.perform(.keep, on: entry.entryId) }
        let deadline = Date().addingTimeInterval(5)
        while !transport.holding, Date() < deadline { await Task.yield() }
        XCTAssertTrue(transport.holding, "the keep never reached the daemon")
        drain(host)
        XCTAssertTrue(store.acting.contains(entry.entryId))
        XCTAssertTrue(TracesRowBodies.built.contains(id), "the row did not change while its action was in flight")
        XCTAssertTrue(store.tree.allSessions.contains { $0.entryId == entry.entryId }, "applied before the answer")

        TracesRowBodies.reset()
        transport.release()
        await action.value
        drain(host)
        XCTAssertFalse(store.acting.contains(entry.entryId))
        XCTAssertTrue(TracesRowBodies.built.contains(id), "the row stayed busy after the answer")

        let source = try TracesParityTests.text("Views/Monitor/TracesViews.swift")
        for needle in ["acting: store.acting.contains(entry.entryId)", "onSubmit: acting ? nil : onSubmit",
                       "accessory: acting ? AnyView(GlassSpinner()) : nil", ".opacity(acting ? TracesRowBusy.opacity : 1)",
                       "let writing = store.writing.contains(folder.id)", ".opacity(writing ? TracesRowBusy.opacity : 1)"] {
            XCTAssertTrue(source.contains(needle), "TracesViews.swift lacks \(needle)")
        }
    }

    private func drain(_ host: TracesScalePerfTests.Host, _ seconds: Double = 0.3) {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            host.draw()
        }
    }
}
