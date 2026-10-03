import XCTest

@testable import TraceCommonsApp

/// An Auto override is a grant: arming goes only through the core's
/// confirmation, and Never is never gated.
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
        let at = try XCTUnwrap(lines.firstIndex { $0.contains("if wanted == .autoUpload {") })
        XCTAssertTrue(lines[at + 1].contains("armingCandidate = project"), "arming must stage the dialog")
        XCTAssertTrue(lines[at + 2].contains("} else {"))
        XCTAssertTrue(lines[at + 3].contains("model.setProjectMode(project, mode: wanted)"))
    }

    func test_theOnlyDirectArmingCallIsTheConfirmButton() throws {
        let hits = lines.indices.filter { lines[$0].contains("mode: .autoUpload") }
        XCTAssertEqual(hits.count, 1)
        let at = try XCTUnwrap(hits.first)
        XCTAssertTrue(lines[at - 1].contains("Button(copy?.confirm"), "arming outside the confirm closure")
    }

    func test_neverIsNotGated() {
        XCTAssertFalse(Self.source.contains(".ignore"), "Never must go straight through the generic setter")
        // The picker's words are the core's one name per mode, through the
        // accessor every macOS surface uses.
        XCTAssertTrue(Self.source.contains("label: ProjectCopy.modeChoiceLabel)"))
    }
}
