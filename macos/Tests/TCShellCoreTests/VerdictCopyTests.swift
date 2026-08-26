import XCTest
@testable import TCShellCore

/// The verdict copy, pinned character for character against the Linux
/// original in `crates/trace-commons-contributor-gtk/src/copy.rs`
/// (`VERDICT_QUESTION`, `VERDICT_WORKED`, `VERDICT_PARTLY`,
/// `VERDICT_FAILED`, `VERDICT_CAPTION`, `SUBMIT_ALL_AS`,
/// `SUBMIT_ALL_AS_TOOLTIP`).
///
/// Asserting a literal against a literal looks circular and is not: this is
/// the same guard `ReadGate`'s sentence has from the Rust side. The caption
/// is the contributor-facing disclosure that the outcome fields sit outside
/// the sheet's "exactly what would be sent" guarantee, and a shell that
/// softens or drops it is claiming more about the preview than is true.
/// Changing it here should require changing it deliberately, in all three
/// shells at once.
final class VerdictCopyTests: XCTestCase {
    func testTheQuestionIsTheSharedWording() {
        XCTAssertEqual(VerdictCopy.question, "Did this session do what you asked?")
    }

    func testTheThreeAnswersAreTheSharedWording() {
        XCTAssertEqual(VerdictCopy.worked, "Worked")
        XCTAssertEqual(VerdictCopy.partly, "Partly")
        XCTAssertEqual(VerdictCopy.failed, "Failed")
    }

    func testTheDisclosureCaptionIsIntact() {
        XCTAssertEqual(
            VerdictCopy.caption,
            "Optional. This is recorded as the trace outcome; the preview above does not show it."
        )
    }

    func testTheBulkControlIsTheSharedWording() {
        XCTAssertEqual(VerdictCopy.submitAllAs, "Submit all as...")
        XCTAssertEqual(
            VerdictCopy.submitAllAsTooltip,
            "Record the same outcome for every session in this group."
        )
    }
}
