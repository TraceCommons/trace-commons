import TCShellCore
import XCTest

/// R6's void notice: the element off `status.grant_voids`, and the notice
/// the Rust assembles for it.
final class GrantVoidNoticeTests: XCTestCase {
    private let projectElement = """
    {"id":4,"kind":"project","voided_at":"2026-09-26T12:00:00Z",
     "project_id":"3f1c","project_label":"api",
     "reasons":["witness-measurement-admitted"]}
    """

    /// The element goes back to the core exactly as it came, so the core --
    /// not this shell -- reads `kind` and `reasons` to choose the words.
    func testTheWireElementRoundTripsToTheCore() throws {
        let void = try JSONDecoder().decode(GrantVoidWire.self, from: Data(projectElement.utf8))
        XCTAssertEqual(void.id, 4)
        let original = try JSONSerialization.jsonObject(with: Data(projectElement.utf8)) as? NSDictionary
        let passed = try JSONSerialization.jsonObject(with: Data(void.json.utf8)) as? NSDictionary
        XCTAssertEqual(original, passed)
    }

    /// An element this build does not recognise is still a void. It keeps
    /// its id so it can be reported, rather than failing the whole status.
    func testAnUnfamiliarElementWithAnIdStillDecodes() throws {
        let void = try JSONDecoder().decode(
            GrantVoidWire.self, from: Data(#"{"id":9,"kind":"folder"}"#.utf8))
        XCTAssertEqual(void.id, 9)
        XCTAssertThrowsError(
            try JSONDecoder().decode(GrantVoidWire.self, from: Data(#"{"kind":"project"}"#.utf8)))
    }

    private let notice = """
    {"title":"Automatic contributing stopped for api","body":"b",
     "reasons_heading":"What changed","reasons":["r1","r2"],
     "rearm":"re","acknowledge":"Got it"}
    """

    func testTheNoticeDecodesWhole() throws {
        let decoded = try XCTUnwrap(GrantVoidNotice.decode(fromJSON: notice))
        XCTAssertEqual(decoded.title, "Automatic contributing stopped for api")
        XCTAssertEqual(decoded.reasons, ["r1", "r2"])
        XCTAssertEqual(decoded.acknowledge, "Got it")
    }

    /// Shown whole or not at all: a notice without why, or without how to
    /// turn it back on, is refused rather than drawn in part.
    func testANoticeMissingASentenceIsRefused() throws {
        for field in GrantVoidNotice.consumedFields {
            var object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: Data(notice.utf8)) as? [String: Any])
            object[field] = field == "reasons" ? [String]() : ""
            let data = try JSONSerialization.data(withJSONObject: object)
            XCTAssertNil(
                GrantVoidNotice.decode(fromJSON: String(decoding: data, as: UTF8.self)), field)
        }
        XCTAssertNil(GrantVoidNotice.decode(fromJSON: "null"))
    }
}
