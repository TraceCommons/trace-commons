import TCBridge
import TCShellCore
import XCTest

/// The account-attention status through the real dylib: the status keys go
/// in, the Rust's words come out, and this shell decodes all of them.
final class AccountAttentionBridgeTests: XCTestCase {
    private func words(_ wire: AccountAttentionWire) -> AccountAttention? {
        wire.wireJSON.flatMap(TCConsentCopy.accountAttentionJSON(forStatus:))
            .flatMap(AccountAttention.decode(fromJSON:))
    }

    func testEachReasonGetsTheRustsSentence() throws {
        let unreadable = try XCTUnwrap(words(AccountAttentionWire(
            state: "needs_attention", reason: "account_record_unreadable", nearAILoginUnconfirmed: false)))
        let refused = try XCTUnwrap(words(AccountAttentionWire(
            state: "needs_attention", reason: "account_address_refused", nearAILoginUnconfirmed: true)))
        XCTAssertEqual(unreadable.title, refused.title)
        XCTAssertNotEqual(unreadable.body, refused.body, "the sentence is chosen by the ABI")
        XCTAssertNil(unreadable.privateAI)
        XCTAssertNotNil(refused.privateAI)
        XCTAssertFalse(unreadable.action.isEmpty)
    }

    func testAHealthyAccountShowsNothing() {
        XCTAssertNil(words(AccountAttentionWire(state: "ok", reason: nil, nearAILoginUnconfirmed: false)))
        XCTAssertNil(words(.none))
    }

    /// The exported field set is exactly what this shell decodes.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let json = try XCTUnwrap(TCConsentCopy.accountAttentionJSON(
            forStatus: #"{"account_state":"needs_attention","account_state_reason":"account_record_unreadable"}"#))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), AccountAttention.consumedFields.sorted())
    }
}
