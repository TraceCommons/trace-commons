import XCTest

@testable import TraceCommonsApp

/// R12 of #1173: the first-run window's gate. The step progress it used to
/// draw is now `FirstRunFrame`'s, from `FirstRunNavigation`'s step lists
/// (`FirstRunFrameTests.test_theFrameDrawsTheTierAndRonsStepLabels`,
/// `FirstRunNavigationTests.test_quickHasJoinFoldersUses` and
/// `test_customHasJoinToolsRulesUses`).
final class FirstRunProgressTests: XCTestCase {
    /// The window is gated as the main window gates onboarding: the flow
    /// is drawn only while onboarding is required, the window closes when
    /// it is not, and completing never closes it by itself.
    func test_theWindowIsGatedOnRequiresOnboarding() throws {
        let source = try String(contentsOf: Self.source("Views/Monitor/FirstRunViews.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains("if OnboardingNavigation.hostsFirstRun("))
        XCTAssertTrue(source.contains("requiresOnboarding: model.requiresOnboarding, entered: entered)"))
        XCTAssertTrue(source.contains(".onAppear { entered = true }"))
        XCTAssertTrue(source.contains("onChange(of: model.requiresOnboarding, initial: true)"))
        XCTAssertEqual(source.components(separatedBy: "dismissWindow(id:").count - 1, 1,
                       "the window closes only from the requiresOnboarding gate")
    }

    /// Review Focus 1 at the main window: its gate is the rule's, fed the
    /// window's own persistent `firstRunEntered`, set when the coordinator
    /// appears. A constant there would swap a failed live start's runner for
    /// the startup notice and drop every answer.
    func test_theMainWindowKeepsAnEnteredFirstRunOnARefusal() throws {
        let source = try String(contentsOf: Self.source("Views/MainWindowView.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains("@State private var firstRunEntered = false"))
        XCTAssertTrue(source.contains(
            "requiresOnboarding: model.requiresOnboarding, entered: firstRunEntered)"))
        XCTAssertTrue(source.contains("firstRunEntered = true"))
        XCTAssertEqual(source.components(separatedBy: "OnboardingNavigation.hostsFirstRun(").count - 1, 1)
        XCTAssertTrue(OnboardingNavigation.hostsFirstRun(
            startup: .refused("x"), requiresOnboarding: true, entered: true))
    }

    private static func source(_ path: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
    }

    /// The window draws no pane or progress of its own: every step is drawn
    /// in `FirstRunFrame`, which is the pane and carries the progress, so a
    /// second one here would put a pane inside a pane with two steppers.
    func test_theWindowDrawsNoSecondPaneOrProgress() throws {
        let source = try String(contentsOf: Self.source("Views/Monitor/FirstRunViews.swift"), encoding: .utf8)
        XCTAssertFalse(source.contains("GlassPane"))
        XCTAssertFalse(source.contains("GlassStepProgress"))
        XCTAssertTrue(source.contains("OnboardingCoordinatorView("))
    }
}
