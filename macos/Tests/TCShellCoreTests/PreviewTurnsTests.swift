import TCShellCore
import XCTest

/// `tc_preview_turns_json`'s turn index: an overlay of byte ranges on the
/// redacted body, never text.
final class PreviewTurnsTests: XCTestCase {
    private static let fixture = #"""
    {
      "entry_id": "0b6f1f9e-3f53-4d43-9f43-6a0b9c7f1a11",
      "body_digest": "sha256:ab",
      "envelope_digest": "sha256:cd",
      "turn_count": 2,
      "turns": [
        {"index": 0, "role": "user_message", "byte_offset": 0, "byte_len": 40},
        {"index": 1, "role": "tool_call", "tool_name": "bash", "byte_offset": 40, "byte_len": 12}
      ],
      "leaves_this_mac": {"ignored": true}
    }
    """#

    func testTheIndexDecodesWithAnOptionalToolName() throws {
        let turns = try XCTUnwrap(PreviewTurns.decode(fromJSON: Self.fixture))
        XCTAssertEqual(turns.entryId, "0b6f1f9e-3f53-4d43-9f43-6a0b9c7f1a11")
        XCTAssertEqual(turns.bodyDigest, "sha256:ab")
        XCTAssertEqual(turns.envelopeDigest, "sha256:cd")
        XCTAssertEqual(turns.turnCount, 2)
        XCTAssertEqual(turns.turns.count, 2)
        XCTAssertNil(turns.turns[0].toolName)
        XCTAssertEqual(turns.turns[1].toolName, "bash")
        XCTAssertEqual(turns.turns[1].role, "tool_call")
        XCTAssertEqual(turns.turns[1].byteOffset, 40)
        XCTAssertEqual(turns.turns[1].byteLen, 12)
    }

    /// An index for a different body than the one on screen is not shown:
    /// its offsets would still look like a transcript.
    func testAnIndexIsOnlyForTheBodyItWasAnchoredTo() throws {
        let turns = try XCTUnwrap(PreviewTurns.decode(fromJSON: Self.fixture))
        XCTAssertTrue(turns.indexes(bodyDigest: "sha256:ab"))
        XCTAssertFalse(turns.indexes(bodyDigest: "sha256:00"))
    }

    func testUnreadableIsNothing() {
        XCTAssertNil(PreviewTurns.decode(fromJSON: nil))
        XCTAssertNil(PreviewTurns.decode(fromJSON: "{}"))
        XCTAssertNil(PreviewTurns.decode(fromJSON: "not json"))
    }
}
