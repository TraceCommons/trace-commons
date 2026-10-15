import TCBridge
import TCShellCore
import XCTest

/// The legacy invite migration notice through the real dylib: the notice
/// object a daemon reports goes in, the Rust's words come out, and this
/// shell decodes all of them.
final class LegacyMigrationNoticeBridgeTests: XCTestCase {
    private func wire(_ json: String) throws -> LegacyMigrationWire {
        try JSONDecoder().decode(LegacyMigrationWire.self, from: Data(json.utf8))
    }

    func testTheNoticeIsTheRustsAndSaysWhetherFoldersWereKept() throws {
        let kept = try wire(#"{"offered":false,"notice":{"folders_kept":1,"automatic_grant_kept":false}}"#)
        let none = try wire(#"{"offered":false,"notice":{"folders_kept":0,"automatic_grant_kept":false}}"#)
        let a = try XCTUnwrap(
            TCConsentCopy.legacyMigrationNoticeJSON(forNotice: try XCTUnwrap(kept.noticeJSON))
                .flatMap(LegacyMigrationNotice.decode(fromJSON:)))
        let b = try XCTUnwrap(
            TCConsentCopy.legacyMigrationNoticeJSON(forNotice: try XCTUnwrap(none.noticeJSON))
                .flatMap(LegacyMigrationNotice.decode(fromJSON:)))
        XCTAssertTrue(a.title.contains("NEAR AI account"))
        XCTAssertEqual(a.title, b.title)
        XCTAssertNotEqual(a.folders, b.folders, "the folders sentence is chosen by the ABI")
    }

    /// The exported field set is exactly what this shell decodes.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let json = try XCTUnwrap(
            TCConsentCopy.legacyMigrationNoticeJSON(forNotice: #"{"folders_kept":0}"#))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), LegacyMigrationNotice.consumedFields.sorted())
    }

    /// The offer's exported field set is exactly what this shell decodes,
    /// and every word arrives.
    func testTheOfferIsTheRustsAndEveryFieldIsConsumed() throws {
        let json = try XCTUnwrap(TCCoreCopy.legacyMigrationOfferJSON())
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), LegacyMigrationOffer.consumedFields.sorted())
        let offer = try XCTUnwrap(LegacyMigrationOffer.decode(fromJSON: json))
        XCTAssertTrue(offer.title.contains("NEAR AI account"))
        XCTAssertNotEqual(offer.dismiss, offer.action)
    }

    /// The refusal that asks for the invite is worded by the Rust, under the
    /// label this shell watches for.
    func testTheInviteNeededRefusalIsWorded() throws {
        let line = try XCTUnwrap(
            TCCoreCopy.legacyMigrationRefusalLine(label: LegacyMigrationOffer.inviteNeededLabel))
        let generic = try XCTUnwrap(TCCoreCopy.legacyMigrationRefusalLine(label: "no-such-label"))
        XCTAssertNotEqual(line, generic)
    }

    func testNullIsNothingToShow() {
        XCTAssertNil(TCConsentCopy.legacyMigrationNoticeJSON(forNotice: "null"))
    }
}
