import TCBridge
import TCShellCore
import XCTest

/// The void notice through the real dylib: the element a daemon reports
/// goes in, the Rust's notice comes out, and this shell decodes all of it.
final class GrantVoidNoticeBridgeTests: XCTestCase {
    private func wire(_ json: String) throws -> GrantVoidWire {
        try JSONDecoder().decode(GrantVoidWire.self, from: Data(json.utf8))
    }

    func testAProjectVoidBecomesTheRustsNotice() throws {
        let void = try wire("""
        {"id":4,"kind":"project","voided_at":"2026-09-26T12:00:00Z",
         "project_id":"3f1c","project_label":"api",
         "reasons":["witness-measurement-admitted"]}
        """)
        let json = try XCTUnwrap(TCConsentCopy.voidNoticeJSON(forVoid: void.json))
        let notice = try XCTUnwrap(GrantVoidNotice.decode(fromJSON: json))
        XCTAssertTrue(notice.title.contains("api"))
        XCTAssertEqual(notice.reasons.count, 1)
    }

    /// The grant's own void gets different words, chosen by the ABI.
    func testTheGrantsVoidIsWordedByTheAbiNotHere() throws {
        let project = try wire("""
        {"id":4,"kind":"project","project_label":"api","reasons":["destination-changed"]}
        """)
        let grant = try wire("""
        {"id":5,"kind":"automatic_grant","project_label":null,"reasons":["destination-changed"]}
        """)
        let a = try XCTUnwrap(
            TCConsentCopy.voidNoticeJSON(forVoid: project.json).flatMap(GrantVoidNotice.decode(fromJSON:)))
        let b = try XCTUnwrap(
            TCConsentCopy.voidNoticeJSON(forVoid: grant.json).flatMap(GrantVoidNotice.decode(fromJSON:)))
        XCTAssertNotEqual(a.title, b.title)
        XCTAssertNotEqual(a.body, b.body)
    }

    /// The exported field set is exactly what this shell decodes, so a
    /// sentence added in Rust cannot be silently dropped here.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let void = try wire(#"{"id":1,"kind":"automatic_grant","reasons":["scopes-widened"]}"#)
        let json = try XCTUnwrap(TCConsentCopy.voidNoticeJSON(forVoid: void.json))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), GrantVoidNotice.consumedFields.sorted())
    }

    /// An element the ABI cannot place still gets the ABI's words, so this
    /// shell never needs a fallback sentence of its own.
    func testAnElementTheAbiCannotPlaceIsStillWordedByTheAbi() throws {
        let void = try wire(#"{"id":9,"kind":"folder"}"#)
        let json = try XCTUnwrap(TCConsentCopy.voidNoticeJSON(forVoid: void.json))
        XCTAssertNotNil(GrantVoidNotice.decode(fromJSON: json))
    }
}
