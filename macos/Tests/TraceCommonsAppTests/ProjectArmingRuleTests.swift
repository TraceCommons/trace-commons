@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Choosing Auto sets it at once, with no confirmation box (owner,
/// 2026-11-11). Never asks first only when it would clear waiting
/// sessions (#1146 `ProjectModeField`, as the Traces tree does).
final class ProjectArmingRuleTests: XCTestCase {
    private static let source: String = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/ProjectsSection.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

    private var lines: [String] { Self.source.components(separatedBy: "\n") }

    func test_chosenAutoIsSetDirectly() throws {
        XCTAssertFalse(Self.source.isEmpty)
        XCTAssertTrue(Self.source.contains("case .apply: model.setProjectMode(project, mode: wanted)"))
        XCTAssertFalse(Self.source.contains("armingCandidate"), "no arming confirmation in Settings")
        XCTAssertFalse(Self.source.contains("settingsConfirm"))
    }

    /// Never with sessions waiting asks in the core's words; with none
    /// waiting, or the same mode, it is a direct call or nothing.
    @MainActor
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
        XCTAssertEqual(ProjectsSection.change(waiting, to: .autoUpload), .apply)
        let armed = ProjectRow(projectId: "p", projectLabel: "api", projectPath: "/x", mode: .autoUpload, pendingCount: 2)
        XCTAssertEqual(ProjectsSection.change(armed, to: .ask), .apply)
        XCTAssertTrue(Self.source.contains("model.setProjectMode(project, mode: .ignore)"),
                      "Never is made only from the confirmation's button")
    }
}
