import XCTest

@testable import TraceCommonsApp

/// A health line may be a title alone (owner, 2026-11-12: "Waiting on the
/// privacy scan."), and the core sends its detail as an empty string. The
/// banner draws no empty line under the title for it.
@MainActor
final class TracesHealthBannerTests: XCTestCase {
    func test_anEmptyDetailIsNoDetail() {
        XCTAssertNil(TracesHealth.Banner(title: "Waiting on the privacy scan.", detail: "", tone: .ask).detail)
        XCTAssertNil(TracesHealth.Banner(title: "t", detail: nil, tone: .ask).detail)
        XCTAssertEqual(TracesHealth.Banner(title: "t", detail: "d", tone: .ask).detail, "d")
    }
}
