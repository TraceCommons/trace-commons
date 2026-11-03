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
}
