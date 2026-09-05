import XCTest
@testable import TraceCommonsApp

final class OnboardingNavigationTests: XCTestCase {
    private typealias Step = OnboardingCoordinatorView.Step

    func testWelcomeRequiresFolderConsentBeforeConnectionOnFreshInstall() {
        XCTAssertEqual(Step.afterWelcome(needsRoots: true), .roots)
        XCTAssertEqual(Step.afterWelcome(needsRoots: false), .connect)
    }

    func testBackDoesNotRequireAnotherEnrollmentOrRepeatFolderConsent() {
        XCTAssertEqual(Step.consent.previous(privacyScanConfigured: false), .connect)
        XCTAssertEqual(Step.connect.previous(privacyScanConfigured: false), .welcome)
        XCTAssertEqual(Step.roots.previous(privacyScanConfigured: false), .welcome)
        XCTAssertNil(Step.welcome.previous(privacyScanConfigured: false))
    }

    func testBackSkipsUnavailableScannerAndReturnsFromDoneToProjects() {
        XCTAssertEqual(Step.projects.previous(privacyScanConfigured: true), .privacyScan)
        XCTAssertEqual(Step.projects.previous(privacyScanConfigured: false), .consent)
        XCTAssertEqual(Step.privacyScan.previous(privacyScanConfigured: true), .consent)
        XCTAssertEqual(Step.done.previous(privacyScanConfigured: false), .projects)
    }
}
