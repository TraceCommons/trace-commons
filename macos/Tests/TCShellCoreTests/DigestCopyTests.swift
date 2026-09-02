import XCTest
@testable import TCShellCore

/// The digest used to be able to say only one thing: how many sessions are
/// waiting for review. That was complete while every upload passed through
/// review, and stopped being complete the moment a project could be armed to
/// contribute without asking -- an armed project queues nothing, so a
/// contributor who armed everything got a digest that never fired.
///
/// These pin the contribution half. The daemon composes the same sentence for
/// its own local notifier (`daemon/notify.rs`, `contribution_text`) and the
/// two must agree; the tests there and here assert the same rules.
final class DigestCopyTests: XCTestCase {
    func testNothingContributedProducesNoLine() {
        XCTAssertNil(DigestCopy.contributionLine(count: 0, projects: [], creditPending: 0))
    }

    func testSingularReadsAsOneSession() {
        let line = DigestCopy.contributionLine(count: 1, projects: ["api"], creditPending: 0)
        XCTAssertEqual(line, "1 session contributed from api.")
    }

    func testPluralNamesTheProjects() {
        let line = DigestCopy.contributionLine(
            count: 4, projects: ["api", "web"], creditPending: 0
        )
        XCTAssertEqual(line, "4 sessions contributed from api and web.")
    }

    /// A notification is rendered by the desktop environment and may be
    /// logged by it. Same rule the queue digest follows.
    func testNeverContainsAPath() {
        let line = DigestCopy.contributionLine(
            count: 2, projects: ["api", "web"], creditPending: 3.5
        )
        XCTAssertFalse(line?.contains("/") ?? false, line ?? "")
    }

    func testManyProjectsAreSummarised() {
        let line = DigestCopy.contributionLine(
            count: 9, projects: ["a", "b", "c", "d", "e"], creditPending: 0
        )
        XCTAssertTrue(line?.contains("and 2 more") ?? false, line ?? "")
    }

    func testBlankProjectListStillCounts() {
        XCTAssertEqual(
            DigestCopy.contributionLine(count: 2, projects: [], creditPending: 0),
            "2 sessions contributed."
        )
    }

    /// Credit is the other half of the value exchange and the reason this
    /// line exists -- but only when there is some. "0 credit pending" reads
    /// as a failure rather than as a fresh start, and the first digest after
    /// arming a project is exactly when that would show.
    func testCreditIsStatedOnlyWhenThereIsSome() {
        let with = DigestCopy.contributionLine(count: 2, projects: ["api"], creditPending: 4.25)
        XCTAssertEqual(with, "2 sessions contributed from api. 4.3 credit pending.")
        let without = DigestCopy.contributionLine(count: 2, projects: ["api"], creditPending: 0)
        XCTAssertFalse(without?.contains("credit") ?? true, without ?? "")
    }

    /// Settlement is off on every deployment shipped so far, so a bare figure
    /// would be read as money that exists. The word is always "pending".
    func testPendingCreditIsNeverCalledEarned() {
        let line = DigestCopy.contributionLine(count: 2, projects: ["api"], creditPending: 4.25)
        XCTAssertTrue(line?.contains("pending") ?? false, line ?? "")
        for word in ["earned", "paid", "settled", "worth"] {
            XCTAssertFalse(line?.contains(word) ?? true, "must not say \(word): \(line ?? "")")
        }
    }
}
