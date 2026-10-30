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

    /// The Referrals section's live export is exactly the fields this shell
    /// consumes, decodes whole, and fills only the holes the core names.
    func testTheReferralCopyDecodesFromTheLiveExport() {
        guard let json = TCConsentCopy.referralCopyJSON(),
            let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let copy = ReferralCopy.decode(fromJSON: json)
        else {
            XCTFail("the live referral payload must decode")
            return
        }
        XCTAssertEqual(object.keys.sorted(), ReferralCopy.consumedFields.sorted())
        XCTAssertTrue(copy.joinedMany.contains("{n}"))
        XCTAssertTrue(copy.copyAccessibilityName.contains("{code}"))
        XCTAssertTrue(copy.enterScope.contains("{date}"))
        XCTAssertTrue(copy.creditSent.contains("{amount}"))
        let holes = try! NSRegularExpression(pattern: #"\{([a-z_]+)\}"#)
        for sentence in copy.sentences {
            for match in holes.matches(in: sentence, range: NSRange(sentence.startIndex..., in: sentence)) {
                let name = (sentence as NSString).substring(with: match.range(at: 1))
                XCTAssertTrue(["code", "n", "date", "amount"].contains(name), sentence)
            }
        }
    }

    /// The core reads a typed code, so the shell never parses one itself.
    func testTheReferralCodeDisplayIsTheCoresReading() {
        XCTAssertEqual(TCConsentCopy.referralCodeDisplay(" tc-7k3m9-qx2rd "), "TC-7K3M9-QX2RD")
        XCTAssertEqual(TCConsentCopy.referralCodeDisplay("7K3M9QX2RD"), "TC-7K3M9-QX2RD")
        XCTAssertNil(TCConsentCopy.referralCodeDisplay("hello"))
        XCTAssertNil(TCConsentCopy.referralCodeDisplay(""))
    }

    /// The Devices section's live export is exactly the fields this shell
    /// consumes and decodes whole.
    func testTheAccountDevicesCopyDecodesFromTheLiveExport() {
        guard let json = TCConsentCopy.accountDevicesCopyJSON(),
            let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let copy = AccountDevicesCopy.decode(fromJSON: json)
        else {
            XCTFail("the live devices payload must decode")
            return
        }
        XCTAssertEqual(object.keys.sorted(), AccountDevicesCopy.consumedFields.sorted())
        XCTAssertTrue(copy.codeCommand.contains("{code}"))
        XCTAssertTrue(copy.unlinkConfirm.contains("{device}"))
        XCTAssertTrue(copy.lastSeen.contains("{when}"))
    }

    /// The live export for adding Train coding models to sent traces is
    /// exactly the fields this shell consumes, decodes whole, and counts
    /// where it says it does.
    func testTheSentTracesTrainingCopyDecodesFromTheLiveExport() {
        guard let json = TCConsentCopy.sentTracesTrainingCopyJSON(),
            let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let copy = SentTracesTrainingCopy.decode(fromJSON: json)
        else {
            XCTFail("the live sent-traces payload must decode")
            return
        }
        XCTAssertEqual(object.keys.sorted(), SentTracesTrainingCopy.consumedFields.sorted())
        for template in [copy.bodyOther, copy.lineOther, copy.allowAccessibilityName] {
            XCTAssertTrue(template.contains("{count}"), template)
        }
        for one in [copy.bodyOne, copy.lineOne] {
            XCTAssertFalse(one.contains("{count}"), one)
        }
        XCTAssertFalse(copy.body(count: 3).contains("{count}"))
        XCTAssertTrue(copy.line(count: 3).contains("3"))
        XCTAssertEqual(copy.body(count: 1), copy.bodyOne)
    }
}
