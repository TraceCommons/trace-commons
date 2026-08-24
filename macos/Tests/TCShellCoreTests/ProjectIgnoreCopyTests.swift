import XCTest
@testable import TCShellCore

final class ProjectIgnoreCopyTests: XCTestCase {
    func testSingularReadsAsOneTrace() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 1)
        XCTAssertTrue(body.contains("1 waiting trace."), body)
        XCTAssertFalse(body.contains("traces"), body)
    }

    func testPluralReadsAsManyTraces() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 12)
        XCTAssertTrue(body.contains("12 waiting traces"), body)
    }

    func testNothingWaitingDropsTheRemovalClause() {
        // A group can render with every card approved or uploading.
        // "removes 0 waiting traces" would be wrong and alarming.
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 0)
        XCTAssertFalse(body.contains("0"), body)
        XCTAssertFalse(body.lowercased().contains("removes"), body)
        XCTAssertTrue(body.contains("Stops this project being offered."), body)
    }

    func testAlwaysNamesTheWayBack() {
        for n in [0, 1, 7] {
            let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: n)
            XCTAssertTrue(body.contains("undo this in Settings"), "n=\(n): \(body)")
            XCTAssertTrue(body.contains("Nothing already submitted is affected."), "n=\(n)")
        }
    }

    func testTitleNamesTheProject() {
        XCTAssertEqual(ProjectIgnoreCopy.confirmationTitle(project: "api"), "Ignore api?")
    }
}
