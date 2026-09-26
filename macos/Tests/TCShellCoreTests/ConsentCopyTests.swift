import XCTest

@testable import TCShellCore

/// Decoding the consent surface's sentences, without the dylib.
///
/// Nothing here spells a sentence out. The words are asserted against the
/// payload, and what this shell is held to is that it authors none of them:
/// `ConsentCopyBridgeTests` checks the same properties against the real
/// export, and `consent_copy.rs` is where the sentences themselves are
/// asserted.
final class ConsentCopyTests: XCTestCase {
    /// A complete payload, one value per field. Every test below starts from
    /// this, so a case that means to test one empty field is not also
    /// refused for a missing one.
    private static let complete: [String: String] = [
        "gate_statement": "The statement.",
        "ready_help": "The armed tooltip.",
        "not_pinned_help": "The disarmed tooltip.",
        "auto_scrub_scope": "The scope.",
        "auto_scrub_limit": "The limit.",
        "auto_no_review": "Nobody looks.",
    ]

    private static func json(_ fields: [String: String]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    func testTheContractShapeParses() {
        guard let copy = ConsentCopy.decode(fromJSON: Self.json(Self.complete)) else {
            XCTFail("the contract shape must decode")
            return
        }
        XCTAssertEqual(copy.gateStatement, "The statement.")
        XCTAssertEqual(copy.readyHelp, "The armed tooltip.")
        XCTAssertEqual(copy.notPinnedHelp, "The disarmed tooltip.")
        XCTAssertEqual(copy.autoScrubScope, "The scope.")
        XCTAssertEqual(copy.autoScrubLimit, "The limit.")
        XCTAssertEqual(copy.autoNoReview, "Nobody looks.")
    }

    /// A field the Rust stopped exporting, or exported empty, refuses the
    /// WHOLE payload.
    ///
    /// Nil, never a partly-filled value: a blank where a safety claim goes
    /// is worse than nothing, and a Swift-authored claim is worse than both.
    /// Each case differs from a complete payload in exactly one field, so it
    /// is refused for the reason it names.
    func testAnIncompletePayloadIsRefusedWhole() {
        for field in Self.complete.keys.sorted() {
            var missing = Self.complete
            missing.removeValue(forKey: field)
            XCTAssertNil(
                ConsentCopy.decode(fromJSON: Self.json(missing)),
                "a payload without \(field) must be refused")

            var empty = Self.complete
            empty[field] = ""
            XCTAssertNil(
                ConsentCopy.decode(fromJSON: Self.json(empty)),
                "a payload with an empty \(field) must be refused")
        }
        for junk in ["not json at all", ""] {
            XCTAssertNil(ConsentCopy.decode(fromJSON: junk), "\(junk) must be refused")
        }
    }

    /// The declared inventory is the shape the decoder actually reads.
    func testTheConsumedFieldSetMatchesTheDecodedShape() {
        XCTAssertEqual(
            ConsentCopy.consumedFields.sorted(),
            Self.complete.keys.sorted())
    }
}
