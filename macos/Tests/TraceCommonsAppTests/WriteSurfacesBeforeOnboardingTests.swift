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
        let available: Set<SettingsSection> = [.connection, .startup, .notifications, .updates, .privateAI, .compute]
        for section in SettingsSection.allCases {
            XCTAssertEqual(section.availableBeforeOnboarding, available.contains(section), section.rawValue)
        }
        for write: SettingsSection in [.consent, .publicProfile, .watchedFolders, .tools, .witness, .projects] {
            XCTAssertFalse(write.availableBeforeOnboarding, "\(write.rawValue) writes what first run asks")
        }
    }

    /// The Settings window draws an unavailable section as the Monitor's
    /// onboarding notice (the core's signed-out word and Get started, which
    /// opens first run), never the section itself. No new sentence.
    func test_theSettingsWindowDrawsWriteSectionsAsUnavailable() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("""
                        if model.requiresOnboarding && !section.availableBeforeOnboarding {
                            GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                                Button(OnboardingWelcomeWords.getStarted) { OpenMonitor.request() }
                            }
        """), "a write section must not draw before onboarding")
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
        let routed = LaunchRouting.opening(MenuPanelData.manageRules(requiresOnboarding: true), requiresOnboarding: true, onboardingKnown: true)
        XCTAssertEqual(routed.window, .firstRun)
        XCTAssertNil(routed.settings)
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains(
            "Button(MenuWords.manageRules) { open(MenuPanelData.manageRules(requiresOnboarding: model.requiresOnboarding)) }"))
    }
}
