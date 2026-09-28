import TCShellCore
import XCTest

/// The switch-on notices: `status.arming_rewordings` (K5) and
/// `status.automatic_contribution_held`, and the notices the Rust words for
/// them.
final class SwitchOnNoticesTests: XCTestCase {
    private let rewordingElement = """
    {"id":2,"reworded_at":"2026-09-27T01:00:00Z","project_id":"3f1c",
     "project_label":"api","was":"model_scrubbed","now":"patterns_only"}
    """

    /// The element goes back to the core exactly as it came.
    func testTheRewordingElementRoundTripsToTheCore() throws {
        let wire = try JSONDecoder().decode(ArmingRewordingWire.self, from: Data(rewordingElement.utf8))
        XCTAssertEqual(wire.id, 2)
        XCTAssertEqual(wire.projectId, "3f1c")
        let original = try JSONSerialization.jsonObject(with: Data(rewordingElement.utf8)) as? NSDictionary
        let passed = try JSONSerialization.jsonObject(with: Data(wire.json.utf8)) as? NSDictionary
        XCTAssertEqual(original, passed)
        XCTAssertThrowsError(
            try JSONDecoder().decode(ArmingRewordingWire.self, from: Data(#"{"project_id":"p"}"#.utf8)))
    }

    private let rewordedNotice = """
    {"title":"t","body":"b","now_heading":"h","scope":"s","limit":"l",
     "no_review":"n","acknowledge":"Got it",
     "ask_first_action":"Ask me first","ask_first_failed":"f"}
    """

    func testTheRewordedNoticeDecodesWholeAndTargetsTheElementsProject() throws {
        let notice = try XCTUnwrap(ArmingRewordedNotice.decode(fromJSON: rewordedNotice))
        let wire = try JSONDecoder().decode(ArmingRewordingWire.self, from: Data(rewordingElement.utf8))
        XCTAssertEqual(notice.askFirstTarget(for: wire), "3f1c")
        let noButton = rewordedNotice
            .replacingOccurrences(of: #""ask_first_action":"Ask me first""#, with: #""ask_first_action":null"#)
            .replacingOccurrences(of: #""ask_first_failed":"f""#, with: #""ask_first_failed":null"#)
        let withheld = try XCTUnwrap(ArmingRewordedNotice.decode(fromJSON: noButton))
        XCTAssertNil(withheld.askFirstTarget(for: wire))
    }

    func testARewordedNoticeMissingASentenceIsRefused() throws {
        for field in ArmingRewordedNotice.consumedFields
        where field != "ask_first_action" && field != "ask_first_failed" {
            var object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: Data(rewordedNotice.utf8)) as? [String: Any])
            object[field] = ""
            let data = try JSONSerialization.data(withJSONObject: object)
            XCTAssertNil(ArmingRewordedNotice.decode(fromJSON: String(decoding: data, as: UTF8.self)), field)
        }
        let unpaired = rewordedNotice.replacingOccurrences(
            of: #""ask_first_failed":"f""#, with: #""ask_first_failed":null"#)
        XCTAssertNil(ArmingRewordedNotice.decode(fromJSON: unpaired))
    }

    func testTheHeldObjectDecodesAndNothingHeldIsNotHeld() throws {
        let held = try JSONDecoder().decode(GateHeld.self, from: Data("""
        {"held_sessions":3,"reasons":["admission-evidence-is-per-session"],
         "projects":[{"project_id":"3f1c","project_label":"api","held_sessions":3}]}
        """.utf8))
        XCTAssertEqual(held.heldSessions, 3)
        XCTAssertTrue(held.held)
        let passed = try JSONSerialization.jsonObject(with: Data(held.json.utf8)) as? [String: Any]
        XCTAssertEqual((passed?["projects"] as? [Any])?.count, 1, "the whole object reaches the core")
        XCTAssertFalse(GateHeld.none.held)
        XCTAssertEqual(GateHeld.label, "automatic-contribution-held")
        XCTAssertThrowsError(
            try JSONDecoder().decode(GateHeld.self, from: Data(#"{"held_sessions":-1}"#.utf8)))
    }

    private let heldNotice = """
    {"title":"t","body":"b","reasons":["r"],"release":"r2","ask_first":"a",
     "projects":[{"project_id":"3f1c","line":"api: 3 sessions waiting",
                  "ask_first_action":"Ask me first","ask_first_failed":"f"}]}
    """

    func testTheHeldNoticeDecodesWhole() throws {
        let notice = try XCTUnwrap(GateHeldNotice.decode(fromJSON: heldNotice))
        XCTAssertEqual(notice.projects.first?.projectId, "3f1c")
        XCTAssertEqual(notice.projects.first?.askFirstAction, "Ask me first")
    }

    func testAHeldNoticeMissingASentenceIsRefused() throws {
        for field in ["title", "body", "release", "ask_first"] {
            var object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: Data(heldNotice.utf8)) as? [String: Any])
            object[field] = ""
            let data = try JSONSerialization.data(withJSONObject: object)
            XCTAssertNil(GateHeldNotice.decode(fromJSON: String(decoding: data, as: UTF8.self)), field)
        }
        let noReasons = heldNotice.replacingOccurrences(of: #""reasons":["r"]"#, with: #""reasons":[]"#)
        XCTAssertNil(GateHeldNotice.decode(fromJSON: noReasons))
        let unpaired = heldNotice.replacingOccurrences(
            of: #""ask_first_failed":"f""#, with: #""ask_first_failed":null"#)
        XCTAssertNil(GateHeldNotice.decode(fromJSON: unpaired))
        XCTAssertNil(GateHeldNotice.decode(fromJSON: "null"))
    }
}
