import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Count real row construction inside native scroll views. No accessibility
/// enumeration: asking AX for every child can itself realize lazy content.
@MainActor
final class OtherCollectionsLazyTests: XCTestCase {
    private let count = 200

    func test_theRailBuildsVisibleFoldersAndScrollsToTheLastOne() async throws {
        var scale = TracesScalePerfTests.Scale()
        scale.folders = count
        scale.sessions = 0
        scale.ignoreEvery = 0
        let store = TracesStore(client: try TracesScalePerfTests.client(scale))
        await store.load()
        let last = try XCTUnwrap(store.tree.folders.last)
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        let host = Host(EdgeRailFolderList(store: store, open: { _ in }))
        defer { host.close() }
        host.settle()
        assertVisibleOnly("railFolders", total: count)
        XCTAssertFalse(built("railFolders").contains(last.id))
        try host.scrollToEnd()
        XCTAssertTrue(built("railFolders").contains(last.id), "the last folder was lost")
    }

    func test_settingsProjectsBuildVisibleRowsAndScrollToTheLastOne() async throws {
        var scale = TracesScalePerfTests.Scale()
        scale.folders = count
        scale.sessions = 0
        scale.ignoreEvery = 0
        let replies = try TracesScalePerfTests.replies(scale)
        let daemon = ProjectDaemon(projects: replies.projects)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.refreshProjects()
        let deadline = Date().addingTimeInterval(5)
        while model.projects.count != count, Date() < deadline { await Task.yield() }
        XCTAssertEqual(model.projects.count, count)
        let last = try XCTUnwrap(model.projects.last)
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        // Settings' body scrolls its cards as one. Keep the real section's
        // card wrappers between the rows and this viewport.
        let host = Host(ScrollView { VStack { ProjectsSection() } }.environmentObject(model))
        defer { host.close() }
        host.settle()
        assertVisibleOnly("projects", total: count)
        XCTAssertFalse(built("projects").contains(last.id))
        try host.scrollToEnd()
        XCTAssertTrue(built("projects").contains(last.id), "the last project's mode control was lost")
    }

    func test_managedSessionsBuildVisibleRowsAndScrollToTheLastOne() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let data = try Data(contentsOf: root.appendingPathComponent("tests/fixtures/managed/snapshot.json"))
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let base = try XCTUnwrap((value["sessions"] as? [[String: Any]])?.first)
        value["sessions"] = (0..<count).map { index in
            var row = base
            row["id"] = "session-\(index)"
            row["state"] = "exited"
            row["can_start_again"] = true
            return row
        }
        // Keep this test on the session list rather than the sign-in card.
        value["capabilities"] = ["managed_launch": true, "near_ai_signin": false, "terminal_launch": true]
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: JSONSerialization.data(withJSONObject: value))
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        let host = Host(ScrollView { VStack { ManagedSessionsSection() } }.environmentObject(model))
        defer { host.close() }
        host.settle()
        assertVisibleOnly("managed", total: count)
        XCTAssertFalse(built("managed").contains("session-\(count - 1)"))
        try host.scrollToEnd()
        XCTAssertTrue(built("managed").contains("session-\(count - 1)"), "the last session's actions were lost")
    }

    func test_customizeBuildsVisibleReposAndScrollsToTheLastOne() async throws {
        let (runner, repos, copy, candidate) = try await customize(projects: count, sessions: 0)
        let last = try XCTUnwrap(repos.projects?.last)
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        let host = Host(ToolChecks(copy: copy, runner: runner, adding: AddToolModel(), repos: repos,
                                   rows: [candidate], showsRepos: true, open: true, shown: true)
            .environmentObject(AppModel()))
        defer { host.close() }
        host.settle()
        assertVisibleOnly("repos", total: count)
        XCTAssertFalse(built("repos").contains(last.id))
        try host.scrollToEnd()
        XCTAssertTrue(built("repos").contains(last.id), "the last repo's rule control was lost")
    }

    func test_customizeBuildsVisiblePastSessionsAndScrollsToTheLastOne() async throws {
        let (runner, repos, copy, candidate) = try await customize(projects: 1, sessions: 500)
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        let host = Host(ToolChecks(copy: copy, runner: runner, adding: AddToolModel(), repos: repos,
                                   rows: [candidate], showsRepos: true, open: true, shown: true)
            .environmentObject(AppModel()))
        defer { host.close() }
        host.settle()
        assertVisibleOnly("pastSessions", total: 500)
        XCTAssertFalse(built("pastSessions").contains("past-499"))
        try host.scrollToEnd()
        XCTAssertTrue(built("pastSessions").contains("past-499"), "the last past trace's toggle was lost")
        // Laziness must not change approval: even unbuilt rows remain
        // available to the existing Include every action.
        let project = try XCTUnwrap(repos.projects?.first)
        let sessions = try XCTUnwrap(repos.sessions[project.projectId])
        RulesScreenLayout.includeEvery(&runner.state, projectID: project.projectId, sessions: sessions, on: true)
        XCTAssertEqual(runner.state.pastSelections[project.projectId]?.count, 500)
    }

    private func built(_ key: String) -> Set<String> { OtherCollectionRowBodies.built[key] ?? [] }

    private func assertVisibleOnly(_ key: String, total: Int, file: StaticString = #filePath, line: UInt = #line) {
        let rows = built(key).count
        XCTAssertGreaterThan(rows, 0, "no \(key) rows drawn", file: file, line: line)
        XCTAssertLessThan(rows, 80, "built \(rows) of \(total) \(key) rows before scrolling", file: file, line: line)
    }

    private func customize(projects: Int, sessions: Int) async throws -> (FirstRunRunner, CustomizeRepos, FirstRunCopy, SourceCandidate) {
        var scale = TracesScalePerfTests.Scale()
        scale.folders = projects
        scale.sessions = 0
        scale.ignoreEvery = 0
        let client = try TracesScalePerfTests.client(scale)
        let offered = try await client.listProjects().projects
        let source = PickerSource(projects: offered, sessionCount: sessions)
        var state = FirstRunState(step: .tools)
        state.answer(.claudeCode, .watch(path: "/sample/claude"))
        state.answer(.codex, .off)
        let runner = FirstRunRunner(state: state, daemon: RecordingFirstRunDaemon())
        let repos = CustomizeRepos()
        await repos.load(runner: runner, source: source)
        XCTAssertEqual(repos.projects?.count, projects)
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let candidate = SourceCandidate(source: .claudeCode, path: "/sample/claude", exists: true,
                                        sessionCount: UInt64(sessions), mostRecent: nil, relocatedByEnv: false)
        return (runner, repos, copy, candidate)
    }

    @MainActor private final class PickerSource: FirstRunRulesSource {
        let projects: [ProjectRow]
        let sessionCount: Int
        init(projects: [ProjectRow], sessionCount: Int) { self.projects = projects; self.sessionCount = sessionCount }
        func rulesProjects() async -> [ProjectRow]? { projects }
        func pastSessions(projectID: String) async -> PastSessionList? {
            let rows = (0..<sessionCount).map { index in
                ["session_id": "past-\(index)", "state": "not_queued", "selectable": true,
                 "size_bytes": 100, "source": "claude-code"] as [String: Any]
            }
            return try? JSONDecoder().decode(PastSessionList.self, from: JSONSerialization.data(withJSONObject:
                ["sessions": rows, "total": rows.count, "not_listed": 0, "project_mode": "notify_only"]))
        }
    }

    private final class ProjectDaemon: DaemonCalling, @unchecked Sendable {
        let projects: String
        init(projects: String) { self.projects = projects }
        func call(_ method: String, params paramsJSON: String) -> String {
            method == "list_projects" ? "{\"id\":1,\"result\":\(projects)}"
                : #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
        func searchOriginal(entryID: String, needle: String) -> Int? { nil }
        func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    }

    @MainActor private final class Host {
        let window: NSWindow
        let hosting: NSView
        init(_ view: some View) {
            _ = NSApplication.shared
            let bounds = NSRect(x: 0, y: 0, width: 700, height: 480)
            let hosting = NSHostingView(rootView: view.frame(width: bounds.width, height: bounds.height))
            hosting.frame = bounds
            let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            self.window = window
            self.hosting = hosting
        }
        func settle() {
            let deadline = Date().addingTimeInterval(0.2)
            repeat {
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            } while Date() < deadline
        }
        func scrollToEnd() throws {
            func scrollViews(_ view: NSView) -> [NSScrollView] {
                (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap(scrollViews)
            }
            let scroll = try XCTUnwrap(scrollViews(hosting).first)
            // Lazy stacks refine estimated heights while scrolling. Repeat
            // the native scroll so the final row remains reachable.
            for _ in 0..<6 {
                let document = try XCTUnwrap(scroll.documentView)
                let y = document.isFlipped ? max(0, document.bounds.height - scroll.contentView.bounds.height) : 0
                scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
                scroll.reflectScrolledClipView(scroll.contentView)
                settle()
            }
        }
        func close() { window.close() }
    }
}
