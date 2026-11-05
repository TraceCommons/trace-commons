import Combine
import Foundation
import os
import TCBridge
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Settings > Projects' Manage sheet (owner design, 2026-11-04; #1952
/// review): a bulk change is one `set_project_modes` call the daemon plans
/// and checks, never Automatic, and the sheet's group and bulk menus offer
/// only the daemon's `bulk_modes`.
@MainActor
final class ProjectsManagerSheetTests: XCTestCase {
    private final class RecordingDaemon: DaemonCalling, @unchecked Sendable {
        private let state = OSAllocatedUnfairLock(initialState: [(String, String)]())
        /// What `set_project_modes` answers.
        let setAnswer: String
        init(setAnswer: String = #"{"id":1,"result":{"changed":[],"total":0,"purged":0}}"#) {
            self.setAnswer = setAnswer
        }

        var calls: [(method: String, params: [String: Any])] {
            state.withLock { $0 }.map { method, json in
                (method, (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]) ?? [:])
            }
        }

        func call(_ method: String, params paramsJSON: String) -> String {
            state.withLock { $0.append((method, paramsJSON)) }
            switch method {
            case "set_project_modes": return setAnswer
            case "plan_project_modes":
                return #"{"id":1,"result":{"outcome":"confirm","project_ids":["a","b"],"turns_off_automatic":1,"pending":2,"#
                    + #""confirmation":{"title":"Set 2 projects to Never?","body":"This turns off Automatic for 1 project.","#
                    + #""button":"Never","keep":"Keep","destructive":true},"bulk_modes":["notify_only","ignore"]}}"#
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

    private func model(_ daemon: RecordingDaemon) -> AppModel {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        return model
    }

    /// The daemon's plan arrives whole: what it reaches, both counts, and
    /// the confirmation's words, which this shell never composes.
    func test_thePlanIsTheDaemons() async throws {
        let daemon = RecordingDaemon()
        let answer = await model(daemon).planProjectModes(["a", "b", "c"], mode: .ignore)
        let plan = try XCTUnwrap(answer)
        XCTAssertEqual(plan.outcome, .confirm)
        XCTAssertEqual(plan.projectIds, ["a", "b"])
        XCTAssertEqual(plan.turnsOffAutomatic, 1)
        XCTAssertEqual(plan.pending, 2)
        XCTAssertEqual(plan.confirmation?.title, "Set 2 projects to Never?")
        XCTAssertEqual(plan.bulkModes, [.ask, .ignore])
        let call = try XCTUnwrap(daemon.calls.first { $0.method == "plan_project_modes" })
        XCTAssertEqual(call.params["project_ids"] as? [String], ["a", "b", "c"])
        XCTAssertEqual(call.params["mode"] as? String, "ignore")
    }

    /// A bulk change is one call naming every project, carrying the counts
    /// the person agreed to; only the ids that changed are reported.
    func test_aBulkChangeIsOneCallWithTheAgreedCounts() async throws {
        let daemon = RecordingDaemon(setAnswer: #"{"id":1,"result":{"changed":["a","b"],"total":2,"purged":2}}"#)
        let model = model(daemon)
        let agreed = ProjectsBulkPlan(outcome: .confirm, projectIds: ["a", "b"], turnsOffAutomatic: 1, pending: 2)
        let outcome = await model.setProjectModes(["a", "b", "c"], mode: .ignore, agreed: agreed)
        XCTAssertEqual(outcome, .ran(ProjectsBulkResult(changed: ["a", "b"], total: 2)))
        let sets = daemon.calls.filter { $0.method == "set_project_modes" }
        XCTAssertEqual(sets.count, 1)
        XCTAssertEqual(sets[0].params["project_ids"] as? [String], ["a", "b", "c"])
        XCTAssertEqual(sets[0].params["mode"] as? String, "ignore")
        let sent = try XCTUnwrap(sets[0].params["agreed"] as? [String: Int])
        XCTAssertEqual(sent, ["turns_off_automatic": 1, "pending": 2])
        XCTAssertFalse(daemon.calls.contains { $0.method == "set_project_mode" }, "no per-project calls")
        XCTAssertNil(model.lastActionError)
    }

    /// A change that asks first and had nothing agreed sends no `agreed`.
    func test_anUnconfirmedChangeSendsNoAgreement() async throws {
        let daemon = RecordingDaemon()
        _ = await model(daemon).setProjectModes(["a"], mode: .ask, agreed: nil)
        let call = try XCTUnwrap(daemon.calls.first { $0.method == "set_project_modes" })
        XCTAssertNil(call.params["agreed"])
        XCTAssertEqual(call.params["mode"] as? String, "notify_only")
    }

    /// Counts that grew since the confirmation was drawn come back stale,
    /// so the sheet plans and asks again; nothing is reported as failed.
    func test_staleCountsAskAgain() async {
        let daemon = RecordingDaemon(
            setAnswer: #"{"id":1,"error":{"code":"bad_params","message":"bulk-confirmation-stale"}}"#)
        let model = model(daemon)
        let outcome = await model.setProjectModes(["a"], mode: .ignore, agreed: nil)
        XCTAssertEqual(outcome, .stale)
        XCTAssertNil(model.lastActionError)
    }

    /// A change stopped part way says how many of how many changed.
    func test_aPartialChangeSaysHowManyChanged() async throws {
        let daemon = RecordingDaemon(
            setAnswer: #"{"id":1,"result":{"changed":["a"],"total":3,"purged":0,"refused":"project-key-unrecognized"}}"#)
        let model = model(daemon)
        let outcome = await model.setProjectModes(["a", "b", "c"], mode: .ask, agreed: nil)
        XCTAssertEqual(outcome, .ran(ProjectsBulkResult(changed: ["a"], total: 3, refused: "project-key-unrecognized")))
        XCTAssertEqual(model.lastActionError, "1 of 3 changed.")
    }

    /// A refusal says so in the core's words.
    func test_aRefusalIsReported() async throws {
        let daemon = RecordingDaemon(
            setAnswer: #"{"id":1,"error":{"code":"bad_params","message":"bulk-automatic-refused"}}"#)
        let model = model(daemon)
        let outcome = await model.setProjectModes(["a"], mode: .autoUpload, agreed: nil)
        XCTAssertEqual(outcome, .failed)
        XCTAssertEqual(model.lastActionError, try XCTUnwrap(ShellWords.table?.settings.projectModeFailed))
    }

    // MARK: The sheet's source

    /// Group and bulk menus list the daemon's `bulk_modes` and nothing
    /// else; only a row's own menu offers its `offerableModes`. Every bulk
    /// path goes through the daemon's plan.
    func test_groupAndBulkMenusOfferTheDaemonsBulkModesOnly() {
        let source = Self.source
        XCTAssertFalse(source.isEmpty)
        XCTAssertTrue(source.contains("ProjectModeChoices.options(for: bulkModes ?? [], copy: modeCopy)"))
        XCTAssertEqual(source.components(separatedBy: "options: bulkOptions,").count - 1, 2,
                       "the group header and the bulk bar")
        XCTAssertEqual(source.components(separatedBy: "ProjectModeChoices.options(for: project.offerableModes").count - 1, 1,
                       "only a row offers its own modes")
        XCTAssertFalse(source.contains("ProjectsManager.plan("), "the shell plans nothing itself")
        XCTAssertEqual(source.components(separatedBy: "model.setProjectModes(").count - 1, 1)
        XCTAssertTrue(source.contains("model.planProjectModes(ids, mode: mode)"))
    }

    /// While a change runs, the bulk and group menus and every row's menu
    /// wait for it.
    func test_controlsWaitWhileAChangeRuns() {
        let source = Self.source
        XCTAssertTrue(source.contains(".disabled(selected.isEmpty || running || bulkModes == nil)"))
        XCTAssertTrue(source.contains(".disabled(running || bulkModes == nil)"))
        XCTAssertEqual(source.components(separatedBy: ".disabled(running)").count - 1, 1, "the row's menu")
        XCTAssertTrue(source.contains("guard !running, !ids.isEmpty else { return }"))
    }

    /// The Settings body stays a plain stack (#1941); the sheet's list is
    /// the lazy one.
    func test_onlyTheSheetsListIsLazy() throws {
        let source = Self.source
        let list = try XCTUnwrap(source.range(of: "struct ProjectsManagerList"))
        XCTAssertFalse(source[..<list.lowerBound].contains("LazyVStack"))
        XCTAssertTrue(source[list.lowerBound...].contains("LazyVStack"))
    }

    /// Keyboard and VoiceOver (docs/ui-copy-guide.md sections 4 and 5): the
    /// group's disclosure, its menu and its checkbox have distinct names,
    /// the bulk menu says it acts on the selection, the search field's
    /// prompt is its own key, search takes focus on open, the bulk bar comes
    /// before the list, and the disclosure is disabled while searching.
    func test_controlsAreNamedAndReachable() throws {
        let source = Self.source
        XCTAssertTrue(source.contains("DisclosureGroup(isExpanded: Binding("))
        XCTAssertTrue(source.contains("words.groupAccessibility, [\"title\": group.name, \"count\": count]"))
        XCTAssertTrue(source.contains("words.groupMenuAccessibility, [\"title\": group.name]"))
        XCTAssertTrue(source.contains("words.selectGroupAccessibility, [\"title\": group.name]"))
        XCTAssertTrue(source.contains("words.bulkMenuAccessibility, [\"count\": count]"))
        XCTAssertTrue(source.contains(".accessibilityLabel(ShellWordsCopy.fill(words.selectAccessibility, [\"title\": project.displayLabel]))"))
        XCTAssertTrue(source.contains("prompt: words.searchPrompt"))
        XCTAssertTrue(source.contains("searchFocused = true"))
        XCTAssertTrue(source.contains(".disabled(searching)"))
        let bar = try XCTUnwrap(source.range(of: "bulkBar(shown: shown, selected: selected)"))
        let list = try XCTUnwrap(source.range(of: "ProjectsManagerList(lines: lines)"))
        XCTAssertLessThan(bar.lowerBound, list.lowerBound, "the bulk bar is above the list")
        // The summary's count is not a bare number to VoiceOver.
        XCTAssertTrue(source.contains("ShellWordsCopy.fill($0.projectsCount, [\"count\": count])"))
    }

    /// The bulk confirmation draws the daemon's words, destructive only for
    /// a Never.
    func test_theBulkConfirmationIsTheDaemons() {
        let never = ProjectsBulkPlan.Confirmation(
            title: "t", body: "b", button: "Never", keep: "Keep", help: "h", destructive: true)
        XCTAssertEqual(ProjectsManagerSheet.confirmAction(never) {}.role, .destructive)
        let ask = ProjectsBulkPlan.Confirmation(
            title: "t", body: "b", button: "Ask", keep: "Keep", help: nil, destructive: false)
        let action = ProjectsManagerSheet.confirmAction(ask) {}
        XCTAssertEqual(action.role, .standard)
        XCTAssertEqual(action.title, "Ask")
    }
}
