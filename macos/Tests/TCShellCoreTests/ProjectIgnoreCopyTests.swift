import XCTest
@testable import TCShellCore

/// Decoding the core's ignore-project table. The words themselves are the
/// core's and are asserted against the real export in
/// `TCBridgeTests/CoreCopyExportTests`.
final class ProjectIgnoreCopyTests: XCTestCase {
    private let fixture = """
        {"title": "T", "body": "B", "button": "Btn", "tooltip": "Tip", "keep": "K"}
        """

    func testDecodesEveryField() throws {
        let copy = try XCTUnwrap(ProjectIgnoreCopy.decode(fromJSON: fixture))
        XCTAssertEqual(copy.title, "T")
        XCTAssertEqual(copy.body, "B")
        XCTAssertEqual(copy.button, "Btn")
        XCTAssertEqual(copy.tooltip, "Tip")
        XCTAssertEqual(copy.keep, "K")
    }

    /// Nil, never a partly-filled value: the confirmation is not offered
    /// without the words that say what it removes.
    func testRefusesAnEmptyFieldAMissingFieldAndNoPayload() {
        XCTAssertNil(ProjectIgnoreCopy.decode(fromJSON: fixture.replacingOccurrences(
            of: "\"B\"", with: "\"\"")))
        XCTAssertNil(ProjectIgnoreCopy.decode(fromJSON: #"{"title": "T"}"#))
        XCTAssertNil(ProjectIgnoreCopy.decode(fromJSON: "not json"))
        XCTAssertNil(ProjectIgnoreCopy.decode(fromJSON: nil))
    }
}
