import XCTest
@testable import TCShellCore

/// The arming offer's shape and words. The rule that decides *when* it
/// appears lives in the daemon (`ProjectPolicy::arming_suggestion`) and is
/// tested there; this pins what a contributor reads when it does.
final class ArmingOfferTests: XCTestCase {
    func testDecodesTheDaemonsShape() throws {
        let json = """
        {"project_id": "proj_ab12", "project_label": "api", "contributed_count": 5}
        """
        let offer = try JSONDecoder().decode(ArmingOffer.self, from: Data(json.utf8))
        XCTAssertEqual(offer.projectId, "proj_ab12")
        XCTAssertEqual(offer.projectLabel, "api")
        XCTAssertEqual(offer.contributedCount, 5)
    }

    /// The offer's words are the core's (`ProjectArmingCopy`), decoded here
    /// and asserted against the real export in
    /// `TCBridgeTests/CoreCopyExportTests`.
    func testTheOffersWordsDecodeFromTheCoresTable() throws {
        let json = """
            {"evidence": "E", "question": "Q", "confirm": "C", "decline": "D", "body": "B",
             "body_with_backlog": "BB", "customize": {}, "settings_question": "SQ",
             "settings_description": "SD", "settings_decline": "SN", "settings_confirm": "SC"}
            """
        let copy = try XCTUnwrap(ProjectArmingCopy.decode(fromJSON: json))
        XCTAssertEqual(copy.evidence, "E")
        XCTAssertEqual(copy.question, "Q")
        XCTAssertEqual(copy.confirm, "C")
        XCTAssertEqual(copy.decline, "D")
        XCTAssertEqual(copy.body, "B")
        XCTAssertEqual(copy.bodyWithBacklog, "BB")
        XCTAssertEqual(copy.settingsQuestion, "SQ")
        XCTAssertEqual(copy.settingsDescription, "SD")
        XCTAssertEqual(copy.settingsDecline, "SN")
        XCTAssertEqual(copy.settingsConfirm, "SC")
        XCTAssertNil(ProjectArmingCopy.decode(fromJSON: json.replacingOccurrences(
            of: "\"Q\"", with: "\"\"")))
        XCTAssertNil(ProjectArmingCopy.decode(fromJSON: nil))
    }
}
