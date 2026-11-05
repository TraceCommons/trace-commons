import Combine
import Foundation
import os
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Settings > Projects' Manage sheet (owner design, 2026-11-04): a bulk
/// change is the existing per-project call made once per project, never
/// Automatic, and the sheet's group and bulk menus offer Ask and Never only.
@MainActor
final class ProjectsManagerSheetTests: XCTestCase {
    private final class RecordingDaemon: DaemonCalling, @unchecked Sendable {
        private let state = OSAllocatedUnfairLock(initialState: [(String, String)]())
        /// Project ids whose `set_project_mode` is refused.
        let refuse: Set<String>
        init(refuse: Set<String> = []) { self.refuse = refuse }

        var calls: [(method: String, params: [String: Any])] {
            state.withLock { $0 }.map { method, json in
                (method, (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]) ?? [:])
            }
        }

        func call(_ method: String, params paramsJSON: String) -> String {
            state.withLock { $0.append((method, paramsJSON)) }
            switch method {
            case "set_project_mode":
                let params = (try? JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as? [String: Any]) ?? [:]
                if let id = params["project_id"] as? String, refuse.contains(id) {
                    return #"{"id":1,"error":{"code":"refused","message":"project-key-unrecognized"}}"#
                }
                return #"{"id":1,"result":{"purged":0}}"#
            case "list_projects": return #"{"id":1,"result":{"projects":[]}}"#
            case "list_audit": return #"{"id":1,"result":{"entries":[]}}"#
            default: return #"{"id":1,"result":{}}"#
            }
        }
        func searchOriginal(entryID: String, needle: String) -> Int? { nil }
        func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
    }

    private static let source: String = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/ProjectsSection.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

    /// Waits for the refreshes that follow a bulk change, then lets the
    /// main actor take their answers.
    private func settle(_ daemon: RecordingDaemon, until: () -> Bool) async {
        let deadline = Date().addingTimeInterval(5)
        while !until(), Date() < deadline {
            try? await Task.sleep(nanoseconds: 5_000_000)
        }
        try? await Task.sleep(nanoseconds: 50_000_000)
    }

    private func modeCalls(_ daemon: RecordingDaemon) -> [(String, String)] {
        daemon.calls.filter { $0.method == "set_project_mode" }.map {
            ($0.params["project_id"] as? String ?? "", $0.params["mode"] as? String ?? "")
        }
    }

    func test_bulkNeverIsOneExistingCallPerProjectInOrder() async {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setProjectModes(["b", "a", "c"], mode: .ignore)
        await settle(daemon) { daemon.calls.contains { $0.method == "list_audit" } }
        let calls = modeCalls(daemon)
        XCTAssertEqual(calls.map(\.0), ["b", "a", "c"])
        XCTAssertEqual(Set(calls.map(\.1)), ["ignore"])
        // Other refreshes this stub does not answer may report; a mode
        // change did not.
        if let line = ShellWords.table?.settings.projectModeFailed {
            XCTAssertNotEqual(model.lastActionError, line)
        }
    }

    func test_bulkAskIsAllowed() async {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setProjectModes(["a"], mode: .ask)
        await settle(daemon) { daemon.calls.contains { $0.method == "list_audit" } }
        XCTAssertEqual(modeCalls(daemon).map(\.1), ["notify_only"])
    }

    /// Automatic is a grant: a bulk Automatic sends nothing at all.
    func test_bulkAutomaticSendsNothing() async {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setProjectModes(["a", "b"], mode: .autoUpload)
        try? await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertTrue(daemon.calls.isEmpty, "a bulk Automatic reached the daemon")
    }

    /// The first refusal stops the rest, says so in the core's words, and
    /// the list is still refreshed so it shows what did change.
    func test_aRefusalStopsTheRestAndIsReported() async throws {
        let daemon = RecordingDaemon(refuse: ["b"])
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        // Every error the model reports, in order: a later refresh this
        // stub does not answer may replace the mode change's.
        var reported: [String] = []
        let watch = model.$lastActionError.compactMap { $0 }.sink { reported.append($0) }
        defer { watch.cancel() }
        model.setProjectModes(["a", "b", "c"], mode: .ignore)
        await settle(daemon) { daemon.calls.contains { $0.method == "list_audit" } }
        XCTAssertEqual(modeCalls(daemon).map(\.0), ["a", "b"], "c was sent after b was refused")
        let line = try XCTUnwrap(ShellWords.table?.settings.projectModeFailed)
        XCTAssertEqual(reported.first, line)
    }

    // MARK: The sheet's source

    /// Group and bulk menus list `ProjectsManager.bulkModes` and nothing
    /// else; only a row's own menu offers its `offerableModes`.
    func test_groupAndBulkMenusOfferAskAndNeverOnly() {
        let source = Self.source
        XCTAssertFalse(source.isEmpty)
        XCTAssertTrue(source.contains("ProjectModeChoices.options(for: ProjectsManager.bulkModes, copy: modeCopy)"))
        XCTAssertEqual(source.components(separatedBy: "options: bulkOptions,").count - 1, 2,
                       "the group header and the bulk bar")
        XCTAssertEqual(source.components(separatedBy: "ProjectModeChoices.options(for: project.offerableModes").count - 1, 1,
                       "only a row offers its own modes")
        // Every bulk path goes through the plan, which refuses Automatic.
        XCTAssertEqual(source.components(separatedBy: "model.setProjectModes(").count - 1, 2)
        XCTAssertTrue(source.contains("switch ProjectsManager.plan(rows, to: mode) {"))
        XCTAssertTrue(source.contains("model.setProjectModes(request.ids, mode: .ignore)"))
    }

    /// The Settings body stays a plain stack (#1941); the sheet's list is
    /// the lazy one.
    func test_onlyTheSheetsListIsLazy() throws {
        let source = Self.source
        let list = try XCTUnwrap(source.range(of: "struct ProjectsManagerList"))
        XCTAssertFalse(source[..<list.lowerBound].contains("LazyVStack"))
        XCTAssertTrue(source[list.lowerBound...].contains("LazyVStack"))
    }

    /// Keyboard and VoiceOver: a group header is a native disclosure named
    /// by the group and its count, and each checkbox names its project.
    func test_headersAndCheckboxesAreNamed() {
        let source = Self.source
        XCTAssertTrue(source.contains("words.groupAccessibility, [\"title\": group.name, \"count\": String(group.members.count)]"))
        XCTAssertTrue(source.contains("DisclosureGroup(isExpanded: Binding("))
        XCTAssertTrue(source.contains(".accessibilityLabel(ShellWordsCopy.fill(words.selectAccessibility, [\"title\": project.displayLabel]))"))
    }

    /// A group's Never with traces waiting asks in the one Never
    /// confirmation's words, named by the group.
    func test_groupNeverWithTracesWaitingAsksByName() throws {
        let request = ProjectsManagerSheet.BulkNever(ids: ["a", "b"], name: "api", pending: 3)
        let copy = try XCTUnwrap(ProjectsManagerSheet.bulkNeverCopy(request))
        let one = try XCTUnwrap(ProjectsSection.ignoreCopy(ProjectRow(projectId: "a", projectLabel: "api", mode: .ask, pendingCount: 3)))
        XCTAssertEqual(copy, one)
    }
}
