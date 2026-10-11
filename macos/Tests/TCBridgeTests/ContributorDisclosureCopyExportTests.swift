import TCBridge
import TCShellCore
import XCTest

/// The disclosure bundle's verdict, History and folder-mode words, read
/// through the real dylib the way the monitor reads them.
final class ContributorDisclosureCopyExportTests: XCTestCase {
    func testTheRealBundleDecodes() throws {
        let copy = try XCTUnwrap(ContributorDisclosureCopy.decode(fromJSON: TCCoreCopy.contributorDisclosureCopyJSON()))
        // `outcome_copy`'s words, as the core holds them.
        XCTAssertEqual(copy.outcome.verdictQuestion, "Did this trace do what you asked?")
        XCTAssertEqual([copy.outcome.worked, copy.outcome.partly, copy.outcome.failed], ["Worked", "Partly", "Failed"])
        XCTAssertEqual(copy.outcome.verdictNone, "No answer")
        XCTAssertEqual(copy.outcome.submitAllAs, "Submit as...")
        XCTAssertGreaterThan(copy.outcome.maxCorrectionChars, 0)
        // Ask me / Automatic / Never, by wire mode.
        XCTAssertEqual(copy.folderModeLabels["notify_only"], "Ask me")
        XCTAssertEqual(copy.folderModeLabels["auto_upload"], "Automatic")
        XCTAssertEqual(copy.folderModeLabels["ignore"], "Never")
        XCTAssertEqual(copy.historyUi.statusUnavailable, "Status unavailable")
        XCTAssertFalse(copy.historyUi.statusLabels.isEmpty)
    }
}
