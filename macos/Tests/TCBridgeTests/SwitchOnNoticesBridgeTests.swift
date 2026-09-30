import TCBridge
import TCShellCore
import XCTest

/// The switch-on notices through the real dylib: the daemon's element or
/// object goes in, the Rust's notice comes out, and this shell decodes all
/// of it.
final class SwitchOnNoticesBridgeTests: XCTestCase {
    private func rewording(_ json: String) throws -> ArmingRewordingWire {
        try JSONDecoder().decode(ArmingRewordingWire.self, from: Data(json.utf8))
    }

    private func held(_ json: String) throws -> GateHeld {
        try JSONDecoder().decode(GateHeld.self, from: Data(json.utf8))
    }

    func testARewordingBecomesTheRustsNotice() throws {
        let wire = try rewording("""
        {"id":2,"project_id":"3f1c","project_label":"api",
         "was":"model_scrubbed","now":"patterns_only"}
        """)
        let json = try XCTUnwrap(TCConsentCopy.armingRewordedNoticeJSON(forRewording: wire.json))
        let notice = try XCTUnwrap(ArmingRewordedNotice.decode(fromJSON: json))
        XCTAssertTrue(notice.title.contains("api"), notice.title)
        XCTAssertEqual(notice.askFirstTarget(for: wire), "3f1c")
    }

    func testTheExportedRewordingFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let wire = try rewording(#"{"id":1,"project_id":"p","project_label":"api"}"#)
        let json = try XCTUnwrap(TCConsentCopy.armingRewordedNoticeJSON(forRewording: wire.json))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), ArmingRewordedNotice.consumedFields.sorted())
    }

    func testWhatTheGateHoldsBecomesTheRustsNotice() throws {
        let object = try held("""
        {"held_sessions":2,"reasons":["admission-evidence-is-per-session"],
         "projects":[{"project_id":"3f1c","project_label":"api","held_sessions":2}]}
        """)
        let json = try XCTUnwrap(TCConsentCopy.gateHeldNoticeJSON(forHeld: object.json))
        let notice = try XCTUnwrap(GateHeldNotice.decode(fromJSON: json))
        XCTAssertTrue(notice.body.hasPrefix("2 "), notice.body)
        XCTAssertEqual(notice.projects.count, 1)
        XCTAssertEqual(notice.projects.first?.projectId, "3f1c")
        let keys = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]).keys
        XCTAssertEqual(keys.sorted(), GateHeldNotice.consumedFields.sorted())
    }

    /// Nothing held is nothing to say, decided by the ABI.
    func testNothingHeldGetsNoNotice() {
        XCTAssertNil(TCConsentCopy.gateHeldNoticeJSON(forHeld: GateHeld.none.json))
    }
}
