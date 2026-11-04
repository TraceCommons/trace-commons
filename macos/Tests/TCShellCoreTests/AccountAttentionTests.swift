import TCShellCore
import XCTest

/// #1836 F1: the status keys a shell hands back to the Rust, and the words
/// it decodes.
final class AccountAttentionTests: XCTestCase {
    func testTheWireCarriesTheThreeKeysAsTheDaemonSentThem() throws {
        let wire = AccountAttentionWire(
            state: "needs_attention", reason: "account_record_unreadable", nearAILoginUnconfirmed: true)
        let json = try XCTUnwrap(wire.wireJSON)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object["account_state"] as? String, "needs_attention")
        XCTAssertEqual(object["account_state_reason"] as? String, "account_record_unreadable")
        XCTAssertEqual(object["near_ai_login_unconfirmed"] as? Bool, true)
    }

    /// A daemon too old to send the keys has nothing to hand over.
    func testAnOldDaemonHasNothingToShow() {
        XCTAssertNil(AccountAttentionWire.none.wireJSON)
    }

    func testTheWordsDecodeWholeOrNotAtAll() {
        let full = AccountAttention.decode(fromJSON: #"{"title":"T","body":"B","private_ai":"P","action":"A","confirm_title":"CT","confirm_body":"CB","confirm_action":"CA","confirm_cancel":"CC","failed":"F"}"#)
        XCTAssertEqual(full?.body, "B")
        XCTAssertEqual(full?.privateAI, "P")
        XCTAssertEqual(full?.confirmTitle, "CT")
        XCTAssertEqual(full?.failed, "F")
        let bare = AccountAttention.decode(
            fromJSON: #"{"title":"T","body":"B","private_ai":"P","action":"A","confirm_title":"CT","confirm_body":"CB","confirm_action":"CA","confirm_cancel":"CC","failed":"F"}"#.replacingOccurrences(of: #""B""#, with: "null"))
        XCTAssertNotNil(bare)
        XCTAssertNil(bare?.body)
        // Any required word empty: not shown at all.
        for key in ["title", "action", "confirm_title", "confirm_body", "confirm_action", "confirm_cancel", "failed"] {
            var object = try? JSONSerialization.jsonObject(with: Data(#"{"title":"T","body":"B","private_ai":"P","action":"A","confirm_title":"CT","confirm_body":"CB","confirm_action":"CA","confirm_cancel":"CC","failed":"F"}"#.utf8)) as? [String: Any]
            object?[key] = ""
            let data = try? JSONSerialization.data(withJSONObject: object ?? [:])
            XCTAssertNil(AccountAttention.decode(fromJSON: String(decoding: data ?? Data(), as: UTF8.self)), key)
        }
        XCTAssertNil(AccountAttention.decode(fromJSON: "not json"))
    }

    /// The account section's own Set up again (owner, 2026-11-04): on an
    /// enrolled app, and only while the attention block (which carries the
    /// same button) is not drawn.
    func testSetUpAgainShowsOnAnEnrolledAppWhoseAccountNeedsNoAttention() {
        XCTAssertTrue(SetUpAgainSurface.showsInAccountSection(loggedIn: true, needsAttention: false))
        XCTAssertFalse(
            SetUpAgainSurface.showsInAccountSection(loggedIn: true, needsAttention: true),
            "the attention block already carries it")
        XCTAssertFalse(
            SetUpAgainSurface.showsInAccountSection(loggedIn: false, needsAttention: false),
            "an app that is not set up has the first run")
        XCTAssertFalse(SetUpAgainSurface.showsInAccountSection(loggedIn: false, needsAttention: true))
    }

    func testTheSetUpAgainWordsDecodeWholeOrNotAtAll() {
        let json = #"{"action":"A","confirm_title":"CT","confirm_body":"CB","confirm_action":"CA","confirm_cancel":"CC","failed":"F"}"#
        let copy = SetUpAgainCopy.decode(fromJSON: json)
        XCTAssertEqual(copy?.action, "A")
        XCTAssertEqual(copy?.confirmBody, "CB")
        XCTAssertEqual(copy?.failed, "F")
        for key in SetUpAgainCopy.consumedFields {
            var object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
            object?[key] = " "
            let data = try? JSONSerialization.data(withJSONObject: object ?? [:])
            XCTAssertNil(SetUpAgainCopy.decode(fromJSON: String(decoding: data ?? Data(), as: UTF8.self)), key)
        }
        XCTAssertNil(SetUpAgainCopy.decode(fromJSON: "not json"))
    }
}
