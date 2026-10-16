import XCTest
@testable import TraceCommonsApp

/// R-43: before onboarding completes there is no write surface outside
/// first run. The menu panel's override rows are disabled, "Manage rules"
/// opens first run rather than a Settings section, and the Settings window
/// draws its write sections as unavailable. The Monitor's own Settings
/// button stays disabled (`MonitorNavigationTests`).
@MainActor
final class WriteSurfacesBeforeOnboardingTests: XCTestCase {
    /// The sections Settings draws while onboarding is required: the ones
    /// that ask nothing first run asks. Everything else, every write
    /// section included, is unavailable until onboarding is done.
    func test_onlyTheNonConsentSectionsAreAvailableBeforeOnboarding() {
        let available: Set<SettingsSection> = [.general, .connection, .startup, .notifications, .updates, .privateAI, .compute]
        for section in SettingsSection.allCases {
            XCTAssertEqual(section.availableBeforeOnboarding, available.contains(section), section.rawValue)
        }
        for write: SettingsSection in [.consent, .publicProfile, .watchedFolders, .tools, .witness, .projects] {
            XCTAssertFalse(write.availableBeforeOnboarding, "\(write.rawValue) writes what first run asks")
        }
    }

    /// The Settings window draws an unavailable section as the Monitor's
    /// onboarding notice (the core's signed-out word and first run's Continue, which
    /// opens first run), never the section itself. No new sentence.
    func test_theSettingsWindowDrawsWriteSectionsAsUnavailable() throws {
        // Settings is Ron's modal over the Monitor (#1241 Task 10); each
        // section of its body is gated.
        let window = try MonitorNavigationTests.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(window.contains("""
                switch MonitorGate.of(
                    startup: model.startup, onboardingKnown: model.onboardingKnown,
                    requiresOnboarding: model.requiresOnboarding
                ).forSettings(availableBeforeOnboarding: item.availableBeforeOnboarding) {
        """), "a write section must be gated before onboarding")
        XCTAssertTrue(window.contains("""
                case .signedOut:
                    GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                        Button(MonitorWindowView.openFirstRun) { OpenMonitor.request() }
                    }
        """), "a write section must not draw before onboarding")
        // The section itself is drawn only when the gate is open.
        let gate = try XCTUnwrap(window.range(of: ".forSettings(availableBeforeOnboarding: item.availableBeforeOnboarding) {"))
        let open = try XCTUnwrap(window.range(of: "case .open:", range: gate.upperBound ..< window.endIndex))
        let compute = try XCTUnwrap(window.range(of: "ComputeView(model: compute)", range: gate.upperBound ..< window.endIndex))
        XCTAssertLessThan(open.lowerBound, compute.lowerBound)
    }

    /// The override rows are disabled while onboarding is required, beside
    /// the store's own evidence that the core is up.
    func test_theOverrideRowsWaitForOnboarding() throws {
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains(".disabled(!store.canChooseOverride || model.requiresOnboarding)"))
    }

    /// "Manage rules" opens first run while onboarding is required: no
    /// Settings section, so no Settings window opens beside it.
    func test_manageRulesOpensFirstRunBeforeOnboarding() throws {
        XCTAssertNil(MenuPanelData.manageRules(requiresOnboarding: true))
        XCTAssertEqual(MenuPanelData.manageRules(requiresOnboarding: false), .settings(.watchedFolders))
        let routed = LaunchRouting.opening(MenuPanelData.manageRules(requiresOnboarding: true), startup: .running, requiresOnboarding: true, onboardingKnown: true)
        XCTAssertEqual(routed.window, .firstRun)
        XCTAssertNil(routed.settings)
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains(
            "Button(MenuWords.manageRules) { open(MenuPanelData.manageRules(requiresOnboarding: model.requiresOnboarding)) }"))
    }
}
