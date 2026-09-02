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

    /// The evidence is stated before the question, so a contributor who
    /// reads only the first line still learns why they are being asked.
    func testEvidenceNamesTheProjectAndTheCount() {
        XCTAssertEqual(
            ArmingOfferCopy.evidence(project: "api", count: 5),
            "You've contributed from api 5 times."
        )
    }

    /// The daemon's threshold is five, so this branch is unreachable today.
    /// It is here because the sentence must be right about whatever count it
    /// is handed, and "contributed from api 1 times" is not.
    func testEvidenceIsSingularForOne() {
        XCTAssertEqual(
            ArmingOfferCopy.evidence(project: "api", count: 1),
            "You've contributed from api once."
        )
    }

    func testTheQuestionNamesTheProject() {
        XCTAssertEqual(
            ArmingOfferCopy.question(project: "api"),
            "Contribute from api automatically?"
        )
    }

    /// A confirm button reading "OK" would make the reader reconstruct what
    /// they had agreed to from the question above it.
    func testTheButtonsCarryTheirActions() {
        XCTAssertEqual(ArmingOfferCopy.confirm, "Turn on automatic contributing")
        XCTAssertEqual(ArmingOfferCopy.decline, "Not now")
    }

    /// "Not now", not "No": the daemon silences the offer for thirty days
    /// rather than forever, and the button must not promise otherwise.
    func testDeclineIsNotPermanentSoundingCopy() {
        XCTAssertFalse(ArmingOfferCopy.decline.lowercased().contains("never"))
        XCTAssertFalse(ArmingOfferCopy.decline.lowercased().contains("don't ask"))
    }
}
