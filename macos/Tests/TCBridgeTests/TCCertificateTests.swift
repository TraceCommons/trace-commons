import XCTest

@testable import TCBridge

/// The certificate readings, taken from the live ABI rather than a fixture.
///
/// `TCBridgeTests` builds against the real dylib, so these cannot go stale
/// the way a hand-maintained payload can.
final class TCCertificateTests: XCTestCase {
    /// Both readings arrive, and they are different sentences.
    func testEachReadingIsItsOwnSentence() throws {
        let uninvited = try XCTUnwrap(TCCertificate.rowLine(evidenceAdmitted: true))
        let invited = try XCTUnwrap(TCCertificate.rowLine(evidenceAdmitted: false))
        XCTAssertFalse(uninvited.isEmpty)
        XCTAssertFalse(invited.isEmpty)
        XCTAssertNotEqual(uninvited, invited, "one sentence is answering for both readings")

        let uninvitedTitle = try XCTUnwrap(TCCertificate.listTitle(evidenceAdmitted: true))
        let invitedTitle = try XCTUnwrap(TCCertificate.listTitle(evidenceAdmitted: false))
        XCTAssertNotEqual(uninvitedTitle, invitedTitle)
    }

    /// The direction, stated as an outcome rather than as a mapping.
    ///
    /// `admission_evidence_required` is TRUE for a contributor who signed up
    /// through NEAR and therefore has NO invite. If this shell ever passed
    /// the flag negated, that contributor would be told their session carries
    /// cryptographic proof when nothing has attested it. An assertion that
    /// merely repeated the mapping would be equally happy with the negation
    /// in place.
    func testAContributorWithoutAnInviteIsNeverToldItIsAttested() throws {
        let uninvited = try XCTUnwrap(TCCertificate.rowLine(evidenceAdmitted: true))
        XCTAssertFalse(
            uninvited.lowercased().contains("signed proof"),
            "a contributor with no invite was promised signed proof: \(uninvited)")
        XCTAssertTrue(
            uninvited.lowercased().contains("put forward"),
            "a contributor with no invite was not told they can put it forward: \(uninvited)")

        let invited = try XCTUnwrap(TCCertificate.rowLine(evidenceAdmitted: false))
        XCTAssertTrue(
            invited.lowercased().contains("signed proof"),
            "an invited contributor was not told what the certificate carries: \(invited)")
    }
}
