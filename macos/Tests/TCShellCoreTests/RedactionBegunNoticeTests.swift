import TCShellCore
import XCTest

/// "Data redaction has begun." (owner, 2026-11-05): once per app run, on
/// the first foreground submission that goes to the privacy witness, and
/// never for a background one, which does not use up the once.
final class RedactionBegunNoticeTests: XCTestCase {
    func testTheFirstForegroundSubmissionShowsIt() {
        let notice = RedactionBegunNotice()
        XCTAssertTrue(notice.claim(.foreground, witnessReview: true))
    }

    func testASecondForegroundSubmissionDoesNot() {
        let notice = RedactionBegunNotice()
        XCTAssertTrue(notice.claim(.foreground, witnessReview: true))
        XCTAssertFalse(notice.claim(.foreground, witnessReview: true))
        XCTAssertFalse(notice.claim(.foreground, witnessReview: true))
    }

    func testABackgroundSubmissionNeverShowsItAndDoesNotUseUpTheOnce() {
        let notice = RedactionBegunNotice()
        XCTAssertFalse(notice.claim(.background, witnessReview: true))
        XCTAssertFalse(notice.claim(.background, witnessReview: true))
        XCTAssertTrue(notice.claim(.foreground, witnessReview: true), "the background ones used up the once")
        XCTAssertFalse(notice.claim(.background, witnessReview: true))
    }

    func testASubmissionThatDoesNotGoToTheWitnessSaysNothingAndKeepsTheOnce() {
        let notice = RedactionBegunNotice()
        XCTAssertFalse(notice.claim(.foreground, witnessReview: false))
        XCTAssertTrue(notice.claim(.foreground, witnessReview: true))
    }

    func testTheLineComesFromTheWordsItIsGiven() throws {
        let words = try JSONDecoder().decode(MonitorUndoCopy.self, from: Data("""
            {"approval_saved":"A","approved":"B","unavailable":"C","within":"D",
             "may_have_started":"E","undoing":"F","redaction_begun":"Data redaction has begun."}
            """.utf8))
        let notice = RedactionBegunNotice()
        XCTAssertNil(notice.line(.background, witnessReview: true, words: words))
        XCTAssertEqual(notice.line(.foreground, witnessReview: true, words: words), "Data redaction has begun.")
        XCTAssertNil(notice.line(.foreground, witnessReview: true, words: words))
    }

    func testTheApproveResponseCarriesWitnessReviewAndAnOlderDaemonOmitsIt() throws {
        let with = try JSONDecoder().decode(ApproveResponse.self, from: Data("""
            {"approved":1,"hold_secs":5,"hold_until":null,"flagged":0,"redactions":{},"skipped":[],"witness_review":true}
            """.utf8))
        XCTAssertEqual(with.witnessReview, true)
        let older = try JSONDecoder().decode(ApproveResponse.self, from: Data("""
            {"approved":1,"hold_secs":5,"hold_until":null,"flagged":0,"redactions":{},"skipped":[]}
            """.utf8))
        XCTAssertNil(older.witnessReview)
    }
}
