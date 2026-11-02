import TCShellCore
import XCTest

/// #1836 F1: the status keys a shell hands back to the Rust, and the words
/// it decodes.
final class AccountAttentionTests: XCTestCase {
    func testTheWireCarriesTheThreeKeysAsTheDaemonSentThem() throws {
        let wire = AccountAttentionWire(
            state: "needs_attention", reason: "account_record_unreadable", nearAILoginUnconfirmed: true)
        XCTAssertTrue(wire.needsAttention)
        let json = try XCTUnwrap(wire.wireJSON)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object["account_state"] as? String, "needs_attention")
        XCTAssertEqual(object["account_state_reason"] as? String, "account_record_unreadable")
        XCTAssertEqual(object["near_ai_login_unconfirmed"] as? Bool, true)
    }

    /// A daemon too old to send the keys has nothing to hand over.
    func testAnOldDaemonHasNothingToShow() {
        XCTAssertNil(AccountAttentionWire.none.wireJSON)
        XCTAssertFalse(AccountAttentionWire.none.needsAttention)
        let ok = AccountAttentionWire(state: "ok", reason: nil, nearAILoginUnconfirmed: false)
        XCTAssertFalse(ok.needsAttention)
    }

    func testTheWordsDecodeWholeOrNotAtAll() {
        let full = AccountAttention.decode(
            fromJSON: #"{"title":"T","body":"B","private_ai":"P","action":"A"}"#)
        XCTAssertEqual(full?.body, "B")
        XCTAssertEqual(full?.privateAI, "P")
        let bare = AccountAttention.decode(fromJSON: #"{"title":"T","body":null,"private_ai":null,"action":"A"}"#)
        XCTAssertNotNil(bare)
        XCTAssertNil(bare?.body)
        XCTAssertNil(AccountAttention.decode(fromJSON: #"{"title":"","body":null,"private_ai":null,"action":"A"}"#))
        XCTAssertNil(AccountAttention.decode(fromJSON: "not json"))
    }
}
