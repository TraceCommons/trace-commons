import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Owner decision, 2026-10-02: a folder mode reads "Ask me", "Automatic" or
/// "Never" on every surface. `ProjectCopy.modeChoiceLabel(_:)`, which the
/// legacy Settings picker and the glass screens call, reads the core's name
/// from the pill's table (`tc_contribution_mode_copy_json`), never a Swift
/// spelling.
final class ProjectModeWordsTests: XCTestCase {
    func testEachModeReadsByTheCoresName() throws {
        let pill = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        for mode in [ProjectMode.ask, .autoUpload, .ignore] {
            XCTAssertEqual(ProjectCopy.modeChoiceLabel(mode), pill.label(for: mode), mode.rawValue)
        }
        XCTAssertEqual(
            [ProjectCopy.modeChoiceLabel(.ask), ProjectCopy.modeChoiceLabel(.autoUpload), ProjectCopy.modeChoiceLabel(.ignore)],
            ["Ask me", "Automatic", "Never"]
        )
    }
}
