import Foundation
import XCTest

@testable import TCUpdates

final class UpdatePolicyTests: XCTestCase {
    private let feed = "https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"

    private func managed() -> HomebrewInstallState {
        HomebrewInstallState(
            isManaged: true, caskName: "trace-commons",
            caskroomPath: "/opt/homebrew/Caskroom/trace-commons"
        )
    }

    private func unmanaged() -> HomebrewInstallState {
        HomebrewInstallState(isManaged: false, caskName: "trace-commons", caskroomPath: nil)
    }

    func testADragInstalledAppWithAFeedSelfUpdates() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: feed),
            .selfUpdating
        )
    }

    func testAHomebrewInstallDefersAndCarriesTheCommand() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: managed(), feedURL: feed),
            .managedByHomebrew(upgradeCommand: "brew upgrade --cask trace-commons")
        )
    }

    func testHomebrewWinsEvenWhenNoFeedIsConfigured() {
        // A Homebrew install must never reach the Sparkle branch, and must
        // never be told "updates are unavailable" when the real answer is
        // "brew owns this".
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: managed(), feedURL: nil),
            .managedByHomebrew(upgradeCommand: "brew upgrade --cask trace-commons")
        )
    }

    func testAMissingFeedDisablesUpdatesRatherThanStartingSparkleBlind() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: nil),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
    }

    func testAnEmptyFeedStringCountsAsMissing() {
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: ""),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
        XCTAssertEqual(
            UpdatePolicy.mode(homebrew: unmanaged(), feedURL: "   "),
            .disabled(reason: UpdatePolicy.noFeedReason)
        )
    }

    func testANonHttpsFeedIsRefused() {
        // The appcast authorizes an install. Fetching it over a transport
        // anybody on the path can rewrite is not a downgrade in security to
        // be weighed -- it is the whole control gone.
        XCTAssertEqual(
            UpdatePolicy.mode(
                homebrew: unmanaged(),
                feedURL: "http://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"
            ),
            .disabled(reason: UpdatePolicy.insecureFeedReason)
        )
    }

    func testTheDisabledReasonsAreStableLabelsSafeToLog() {
        XCTAssertEqual(UpdatePolicy.noFeedReason, "update_feed_not_configured")
        XCTAssertEqual(UpdatePolicy.insecureFeedReason, "update_feed_not_https")
    }
}
