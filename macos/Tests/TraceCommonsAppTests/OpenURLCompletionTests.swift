import SwiftUI
import XCTest
@testable import TraceCommonsApp

/// The 0.24.4 Add funds crash (owner, 2026-10-27): the system calls
/// `OpenURLAction`'s completion on a LaunchServices queue, and a completion
/// closure written inside a main-actor view is main-actor isolated, so
/// Swift's runtime check traps when it runs there. Every view that waits on
/// the browser now waits through `OpenURLAction.opened(_:)`, whose
/// completion is built outside any actor.
final class OpenURLCompletionTests: XCTestCase {
    /// The completion, called from a background queue as LaunchServices
    /// calls it, resumes the waiter instead of trapping.
    func testTheCompletionMayBeCalledOffTheMainActor() async {
        for accepted in [true, false] {
            let result = await withCheckedContinuation { continuation in
                let complete = OpenURLAction.resuming(continuation)
                DispatchQueue.global().async { complete(accepted) }
            }
            XCTAssertEqual(result, accepted)
        }
    }

    /// No view waits on the browser with its own completion closure.
    @MainActor
    func testNoViewWaitsOnTheBrowserWithItsOwnClosure() throws {
        for file in ["Views/FundingRow.swift", "Views/CredentialSection.swift", "Views/Monitor/MonitorNotices.swift"] {
            let source = try MonitorNavigationTests.text(file)
            XCTAssertFalse(source.contains("openURL(url) { continuation.resume(returning: $0) }"), file)
            XCTAssertTrue(source.contains("await openURL.opened(url)"), file)
        }
    }
}
