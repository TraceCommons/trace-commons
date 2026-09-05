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

final class OnboardingTransitionTests: XCTestCase {
    func testRootsStartAndRevisitedConnectRejectOldEnrollmentCompletion() {
        var flow = OnboardingNavigation()
        flow.enter(.roots)
        flow.enter(.connect)
        let oldVisit = flow.connectVisit
        flow.enter(.welcome)
        flow.enter(.connect)
        flow.enrolled(visit: oldVisit)
        XCTAssertEqual(flow.step, .connect)
        flow.enrolled(visit: flow.connectVisit)
        XCTAssertEqual(flow.step, .consent)
    }

    func testUnknownSettingsCannotAdvanceOrStartSavingConsent() {
        var flow = OnboardingNavigation(step: .consent)
        XCTAssertFalse(flow.beginConsentSave(scanConfigured: nil))
        XCTAssertFalse(flow.consentSaveInProgress)
        XCTAssertEqual(flow.step, .consent)
    }

    func testConsentSaveBlocksBackAndDuplicateSaveAndPreservesScanDecision() {
        var flow = OnboardingNavigation(step: .consent)
        XCTAssertTrue(flow.beginConsentSave(scanConfigured: true))
        XCTAssertFalse(flow.beginConsentSave(scanConfigured: false))
        flow.enter(.connect)
        XCTAssertEqual(flow.step, .consent)
        flow.finishConsentSave(succeeded: true)
        XCTAssertEqual(flow.step, .privacyScan)
        flow.enter(.projects)
        XCTAssertEqual(flow.step.previous(privacyScanConfigured: flow.scanIncluded), .privacyScan)
    }

    func testFailureStaysOnConsentAndUnavailableScanStaysSkippedOnBack() {
        var flow = OnboardingNavigation(step: .consent)
        XCTAssertTrue(flow.beginConsentSave(scanConfigured: false))
        flow.finishConsentSave(succeeded: false)
        XCTAssertEqual(flow.step, .consent)
        XCTAssertTrue(flow.beginConsentSave(scanConfigured: false))
        flow.finishConsentSave(succeeded: true)
        XCTAssertEqual(flow.step, .projects)
        XCTAssertEqual(flow.step.previous(privacyScanConfigured: flow.scanIncluded), .consent)
    }
}
