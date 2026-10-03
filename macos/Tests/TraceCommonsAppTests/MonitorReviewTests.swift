#if DEBUG
import XCTest
@testable import TraceCommonsApp

/// A session's Review pill opens its review. The review lives in the
/// inspector, so with the inspector hidden, Review shows it rather than only
/// moving a selection nobody can see.
final class MonitorReviewTests: XCTestCase {
    func test_reviewSelectsTheSessionAndShowsTheInspector() {
        var selection = ""
        var showsInspector = false
        MonitorWindowView.review("e1", selection: &selection, showsInspector: &showsInspector)
        XCTAssertEqual(selection, "e1")
        XCTAssertTrue(showsInspector)
    }
}
#endif
