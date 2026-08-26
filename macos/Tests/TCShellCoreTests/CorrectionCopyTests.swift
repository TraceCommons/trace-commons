import XCTest
@testable import TCShellCore

/// The correction copy, pinned character for character against the Linux
/// original in `crates/trace-commons-contributor-gtk/src/copy.rs`
/// (`CORRECTION_QUESTION`, `CORRECTION_PLACEHOLDER`, `CORRECTION_CAPTION`,
/// `CORRECTION_CREDENTIAL_HEADLINE`, `CORRECTION_CREDENTIAL_BODY`).
///
/// Asserting a literal against a literal looks circular and is not, and here
/// it matters more than it does for the verdict copy. The published policy
/// page says redaction happens locally and is re-applied on the server; a
/// correction is the one exception and the page does not yet say so. Until
/// it does, the caption is the entire disclosure a contributor gets that
/// their own words are stored verbatim, so a shell that shortens it for
/// layout is shipping the exception undisclosed. Changing it should require
/// changing it deliberately, in all three shells at once.
final class CorrectionCopyTests: XCTestCase {
    func testThePromptAndPlaceholderAreTheSharedWording() {
        XCTAssertEqual(CorrectionCopy.question, "What did it get wrong?")
        XCTAssertEqual(CorrectionCopy.placeholder, "Optional")
    }

    func testTheDisclosureCaptionIsIntact() {
        XCTAssertEqual(
            CorrectionCopy.caption,
            "Stored exactly as you write it. Unlike the rest of the trace, a correction is not scrubbed here or on the server -- so leave out anything you would not want in the corpus: someone else's personal information, employer-confidential material, or anything you are not free to share."
        )
        // The halves that must never quietly drop out: what is different
        // about a correction, and what not to put in one.
        XCTAssertTrue(CorrectionCopy.caption.contains("Stored exactly as you write it"))
        XCTAssertTrue(CorrectionCopy.caption.contains("not scrubbed here or on the server"))
        XCTAssertTrue(CorrectionCopy.caption.contains("personal information"))
        XCTAssertTrue(CorrectionCopy.caption.contains("employer-confidential"))
        XCTAssertTrue(CorrectionCopy.caption.contains("not free to share"))
    }

    /// The refusal says both things it has to: nothing was sent, and the
    /// credential has to be rotated because it has already been typed.
    func testTheCredentialRefusalSaysNothingWasSentAndToRotate() {
        XCTAssertEqual(
            CorrectionCopy.credentialHeadline,
            "Nothing was sent. Your correction looks like it contains a credential."
        )
        XCTAssertTrue(CorrectionCopy.credentialBody.contains("rotate it"))
        XCTAssertTrue(CorrectionCopy.credentialBody.contains("already been typed"))
    }

    func testTheRefusalLabelIsTheDaemonsWireSpelling() {
        XCTAssertEqual(CorrectionCopy.credentialRefusalLabel, "correction-credential-detected")
    }

    /// Blank is not an empty correction: it is no correction. `toSend`
    /// answers `nil` so the caller omits the key entirely rather than
    /// declaring `correction_included` for content that is not there.
    func testABlankBoxSendsNothing() {
        XCTAssertNil(CorrectionCopy.toSend(""))
        XCTAssertNil(CorrectionCopy.toSend("   "))
        XCTAssertNil(CorrectionCopy.toSend("\n\t "))
    }

    func testWrittenTextIsSentTrimmed() {
        XCTAssertEqual(
            CorrectionCopy.toSend("  it stopped halfway  "),
            "it stopped halfway"
        )
    }

    /// The cap is the daemon's, so an over-long correction is shortened at
    /// the keyboard rather than refused as `correction-too-long`.
    func testTheCapMatchesTheDaemons() {
        XCTAssertEqual(CorrectionCopy.maxCharacters, 2000)
    }

    /// The refusal is recognised off the response, and nothing else is.
    func testOnlyTheCredentialRefusalIsRecognisedAsOne() {
        let refused = ApproveResponse(
            approved: 0,
            flagged: 0,
            redactions: [:],
            skipped: [
                ApproveSkip(entryID: "e1", reasonLabel: CorrectionCopy.credentialRefusalLabel)
            ],
            holdSecs: 0,
            holdUntil: nil
        )
        XCTAssertTrue(refused.wasRefusedForACorrectionCredential)

        let otherSkip = ApproveResponse(
            approved: 0,
            flagged: 0,
            redactions: [:],
            skipped: [ApproveSkip(entryID: "e1", reasonLabel: "envelope-too-large")],
            holdSecs: 0,
            holdUntil: nil
        )
        XCTAssertFalse(otherSkip.wasRefusedForACorrectionCredential)

        let clean = ApproveResponse(
            approved: 1,
            flagged: 0,
            redactions: [:],
            skipped: [],
            holdSecs: 5,
            holdUntil: nil
        )
        XCTAssertFalse(clean.wasRefusedForACorrectionCredential)
    }
}
