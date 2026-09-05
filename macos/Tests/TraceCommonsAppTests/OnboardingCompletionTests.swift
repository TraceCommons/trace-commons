import Combine
import XCTest
@testable import TraceCommonsApp

/// Covers the transition out of onboarding -- the one the Done button drives.
///
/// This target exists because that button shipped broken: pressing it wrote
/// the completion marker and nothing else, and since `isOnboardingComplete`
/// is computed from `UserDefaults` rather than a `@Published` property, the
/// write notified nobody and `MainWindowView` never re-evaluated. The screen
/// simply stayed. Nothing could catch it, because the app target had no test
/// target at all.
///
/// `testMarkingCompleteNotifiesObservers` is the regression: it fails on the
/// code as shipped and passes on the fix.
final class OnboardingCompletionTests: XCTestCase {
    /// A tenant nobody else uses, because these tests write to
    /// `UserDefaults.standard` -- the same store the real app uses. Keyed
    /// per-test and removed in `tearDown` so a run leaves no marker behind
    /// on a developer's machine.
    private var tenant = ""
    private var cancellables: Set<AnyCancellable> = []

    override func setUp() {
        super.setUp()
        tenant = "test-tenant-\(UUID().uuidString)"
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: "trace_commons.onboarding_complete.\(tenant)")
        cancellables.removeAll()
        super.tearDown()
    }

    @MainActor
    private func enrolledModel(tenantID: String?) -> AppModel {
        let model = AppModel()
        model.setStatusForTesting(
            DaemonStatus(
                schemaVersion: "1.1",
                loggedIn: true,
                tenantID: tenantID,
                consentScopes: ["debugging_evaluation"],
                paused: false,
                queueDepth: 0,
                nextDigestAt: nil,
                health: DaemonHealth(lastErrorLabel: nil, since: nil)
            )
        )
        return model
    }

    /// The bug itself. A view only re-renders when the model it observes
    /// says something changed, so a marker write that publishes nothing
    /// leaves the contributor on the Done screen forever -- and
    /// `publishIfChanged` means no unrelated refresh will rescue them
    /// either, on a daemon whose status is not moving.
    @MainActor
    func testMarkingCompleteNotifiesObservers() {
        let model = enrolledModel(tenantID: tenant)
        var notifications = 0
        model.objectWillChange.sink { _ in notifications += 1 }.store(in: &cancellables)

        model.markOnboardingComplete()

        XCTAssertEqual(notifications, 1, "pressing Done must notify observers, or the view never re-renders")
    }

    @MainActor
    func testMarkingCompleteFlipsTheFlagObserversWillRead() {
        let model = enrolledModel(tenantID: tenant)
        XCTAssertFalse(model.isOnboardingComplete)

        model.markOnboardingComplete()

        XCTAssertTrue(model.isOnboardingComplete)
    }

    /// The marker is keyed by tenant on purpose: re-enrolling into a
    /// different commons is a different consent decision, and inheriting the
    /// previous tenant's "done" would skip the screen where scopes are
    /// chosen.
    @MainActor
    func testCompletionDoesNotCarryToAnotherTenant() {
        let model = enrolledModel(tenantID: tenant)
        model.markOnboardingComplete()
        XCTAssertTrue(model.isOnboardingComplete)

        let other = "test-tenant-\(UUID().uuidString)"
        model.setStatusForTesting(
            DaemonStatus(
                schemaVersion: "1.1",
                loggedIn: true,
                tenantID: other,
                consentScopes: [],
                paused: false,
                queueDepth: 0,
                nextDigestAt: nil,
                health: DaemonHealth(lastErrorLabel: nil, since: nil)
            )
        )

        XCTAssertFalse(model.isOnboardingComplete, "a new tenant must walk onboarding again")
        UserDefaults.standard.removeObject(forKey: "trace_commons.onboarding_complete.\(other)")
    }

    /// Fail-closed: with no tenant there is nothing to key a marker to, so
    /// nothing is recorded. The contributor stays in onboarding rather than
    /// landing in the main window with a consent choice they never
    /// confirmed.
    @MainActor
    func testNoTenantRecordsNothing() {
        let model = enrolledModel(tenantID: nil)

        model.markOnboardingComplete()

        XCTAssertFalse(model.isOnboardingComplete)
    }
    @MainActor
    func testMissingRootsAlwaysRequiresOnboardingEvenForCompletedEnrollment() {
        let model = enrolledModel(tenantID: tenant)
        model.markOnboardingComplete()
        model.setStartupForTesting(.running)
        XCTAssertFalse(model.requiresOnboarding)
        model.setStartupForTesting(.needsRoots)
        XCTAssertTrue(model.requiresOnboarding)
    }
}
