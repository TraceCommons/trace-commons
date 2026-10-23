import TCBridge
import TCShellCore
import XCTest

/// The consent bundle through the real dylib.
final class ConsentCopyBridgeTests: XCTestCase {
    func testTheLivePayloadDecodes() {
        guard let json = TCConsentCopy.copyJSON(),
            let copy = ConsentCopy.decode(fromJSON: json)
        else {
            XCTFail("the live payload must decode")
            return
        }
        for sentence in copy.sentences {
            XCTAssertFalse(sentence.isEmpty)
        }
    }

    /// The exported field set is exactly what this shell decodes.
    ///
    /// The decode above proves no required field is missing. It cannot prove
    /// the reverse -- a field ADDED in Rust that this shell silently ignores
    /// would be a sentence the other two shells show and this one does not.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() {
        guard let json = TCConsentCopy.copyJSON(),
            let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            XCTFail("the live payload must be a JSON object")
            return
        }
        XCTAssertEqual(object.keys.sorted(), ConsentCopy.consumedFields.sorted())
    }

    /// The branch crosses. This shell asks which sentence, it does not
    /// choose.
    func testTheHelpSentenceComesFromTheAbiForBothAnswers() {
        guard let json = TCConsentCopy.copyJSON(),
            let copy = ConsentCopy.decode(fromJSON: json)
        else {
            XCTFail("the live payload must decode")
            return
        }
        XCTAssertEqual(TCConsentCopy.gateHelp(pinned: true), copy.readyHelp)
        XCTAssertEqual(TCConsentCopy.gateHelp(pinned: false), copy.notPinnedHelp)
    }

    /// The payout section's live export is exactly the fields this shell
    /// consumes, decodes whole, and has no passkey wording.
    func testThePayoutLinkCopyDecodesFromTheLiveExport() {
        guard let json = TCConsentCopy.payoutLinkCopyJSON(),
            let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let copy = PayoutLinkCopy.decode(fromJSON: json)
        else {
            XCTFail("the live payout payload must decode")
            return
        }
        XCTAssertEqual(object.keys.sorted(), PayoutLinkCopy.consumedFields.sorted())
        for sentence in copy.sentences {
            XCTAssertFalse(sentence.lowercased().contains("passkey"), sentence)
        }
        XCTAssertTrue(copy.body.contains("{org_name}"))
    }
}
