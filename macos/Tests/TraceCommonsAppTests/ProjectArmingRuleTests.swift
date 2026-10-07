@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// An Auto override is a grant: arming goes only through the core's
/// confirmation. Never asks first only when it would clear waiting
/// sessions (#1146 `ProjectModeField`, as the Traces tree does).
final class ProjectArmingRuleTests: XCTestCase {
    private static let source: String = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/ProjectsSection.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

    private var lines: [String] { Self.source.components(separatedBy: "\n") }

    func test_chosenAutoOnlyStagesTheConfirmation() throws {
        XCTAssertFalse(Self.source.isEmpty)
        XCTAssertTrue(Self.source.contains("if wanted == .autoUpload { return .arm }"))
        XCTAssertTrue(Self.source.contains("case .arm: armingCandidate = project"), "arming must stage the dialog")
        XCTAssertTrue(Self.source.contains("case .apply: model.setProjectMode(project, mode: wanted)"))
    }

    func test_theOnlyDirectArmingCallIsTheConfirmButton() throws {
        let hits = lines.indices.filter { lines[$0].contains("mode: .autoUpload") }
        XCTAssertEqual(hits.count, 1)
        let at = try XCTUnwrap(hits.first)
        XCTAssertTrue(lines[at - 1].contains("GlassModalAction(copy.settingsConfirm, isDefault: true) {"),
                      "arming outside the confirm closure")
    }

    /// Never with sessions waiting asks in the core's words; with none
    /// waiting, or the same mode, it is a direct call or nothing.
    func test_neverAsksOnlyWhenSessionsWait() throws {
        XCTAssertFalse(Self.source.contains("ProjectCopy.modeChoiceLabel"))
        let waiting = ProjectRow(projectId: "p", projectLabel: "api", projectPath: "/x", mode: .ask, pendingCount: 2)
        XCTAssertEqual(ProjectsSection.change(waiting, to: .ignore), .ignore)
        XCTAssertNotNil(ProjectsSection.ignoreCopy(waiting))
        let idle = ProjectRow(projectId: "p", projectLabel: "api", projectPath: "/x", mode: .ask, pendingCount: 0)
        XCTAssertEqual(ProjectsSection.change(idle, to: .ignore), .apply)
        let unread = ProjectRow(projectId: "p", projectLabel: "api", projectPath: "/x", mode: .ask)
        XCTAssertEqual(ProjectsSection.change(unread, to: .ignore), .apply)
        XCTAssertEqual(ProjectsSection.change(waiting, to: .ask), .noop)
        XCTAssertEqual(ProjectsSection.change(waiting, to: .autoUpload), .arm)
        let armed = ProjectRow(projectId: "p", projectLabel: "api", projectPath: "/x", mode: .autoUpload, pendingCount: 2)
        XCTAssertEqual(ProjectsSection.change(armed, to: .ask), .apply)
        XCTAssertTrue(Self.source.contains("model.setProjectMode(project, mode: .ignore)"),
                      "Never is made only from the confirmation's button")
    }
}
