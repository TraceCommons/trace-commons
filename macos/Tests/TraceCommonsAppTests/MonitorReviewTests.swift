#if DEBUG
import XCTest
@testable import TraceCommonsApp

/// A session's review lives in the inspector, so selecting a session in the
/// tree shows the inspector rather than only moving a selection nobody can
/// see (`MonitorWindowView.select`, which replaced the Review pill's opener).
final class MonitorReviewTests: XCTestCase {
    func test_selectingASessionShowsTheInspector() {
        var selection: MonitorSelection?
        var showsInspector = false
        MonitorWindowView.select(.session(entryID: "e1"), selection: &selection, showsInspector: &showsInspector)
        XCTAssertEqual(selection, .session(entryID: "e1"))
        XCTAssertTrue(showsInspector)
    }

    /// History was the last caller of the old opener; it is gone.
    func test_theOldReviewOpenerIsGone() throws {
        let window = try HistoryParityTests.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("static func review("))
    }
}
#endif
