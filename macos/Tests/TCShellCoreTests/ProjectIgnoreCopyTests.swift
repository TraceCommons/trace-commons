import XCTest
@testable import TCShellCore

final class ProjectIgnoreCopyTests: XCTestCase {
    func testSingularReadsAsOneTrace() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 1)
        XCTAssertTrue(body.contains("1 waiting trace"), body)
        XCTAssertFalse(body.contains("traces"), body)
    }

    func testPluralReadsAsManyTraces() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 12)
        XCTAssertTrue(body.contains("12 waiting traces"), body)
    }

    func testNothingWaitingDropsTheRemovalClause() {
        // No group renders with nothing waiting today -- every shell groups
        // the pending list alone. The branch is defensive: this function must
        // be right about whatever count it is handed, and "removes 0 waiting
        // traces" would be wrong and alarming.
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

    func testReconciliationSpeaksOnlyWhenTheCountMoved() {
        XCTAssertNil(ProjectIgnoreCopy.reconciliation(project: "api", promised: 3, purged: 3))
        XCTAssertNil(ProjectIgnoreCopy.reconciliation(project: "api", promised: 0, purged: 0))
        let more = ProjectIgnoreCopy.reconciliation(project: "api", promised: 3, purged: 5)
        XCTAssertEqual(
            more,
            "Ignored api. The queue changed while you were deciding: "
                + "5 waiting traces were removed, not 3."
        )
        let one = ProjectIgnoreCopy.reconciliation(project: "api", promised: 3, purged: 1)
        XCTAssertEqual(
            one,
            "Ignored api. The queue changed while you were deciding: "
                + "1 waiting trace was removed, not 3."
        )
    }

    func testTitleNamesTheProject() {
        XCTAssertEqual(ProjectIgnoreCopy.confirmationTitle(project: "api"), "Ignore api?")
    }
}
