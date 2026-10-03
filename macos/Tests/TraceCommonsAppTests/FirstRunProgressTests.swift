import XCTest

@testable import TraceCommonsApp

/// R12 of #1173: the first-run pane's step progress follows the existing
/// onboarding sequence, and shows only steps the person will take.
final class FirstRunProgressTests: XCTestCase {
    /// Welcome comes before the steps and Done after them: no progress.
    func test_welcomeAndDoneShowNoProgress() {
        XCTAssertNil(FirstRunProgress(step: .welcome, folders: true, scan: true))
        XCTAssertNil(FirstRunProgress(step: .done, folders: true, scan: true))
    }

    /// Each step is the current one when it is on screen, in the sequence's
    /// order.
    func test_eachStepIsCurrentInOrder() throws {
        let order: [OnboardingNavigation.Step] = [.roots, .connect, .consent, .privacyScan, .projects]
        for (index, step) in order.enumerated() {
            let progress = try XCTUnwrap(FirstRunProgress(step: step, folders: true, scan: true))
            XCTAssertEqual(progress.current, index, "\(step)")
            XCTAssertEqual(progress.labels.count, order.count)
        }
    }

    /// The scan step shows only when the scan will run: a person whose scan
    /// is not configured is never shown a step they skip.
    func test_theScanStepShowsOnlyWhenItRuns() throws {
        let without = try XCTUnwrap(FirstRunProgress(step: .projects, folders: true, scan: false))
        XCTAssertFalse(without.labels.contains(FirstRunWords.scan))
        XCTAssertEqual(without.current, without.labels.count - 1)
        XCTAssertNil(FirstRunProgress(step: .privacyScan, folders: true, scan: false))
    }

    /// The progress follows the coordinator's own Back order, so the stepper
    /// never runs ahead of or behind the screen.
    func test_theProgressFollowsTheBackOrder() throws {
        var step: OnboardingNavigation.Step = .projects
        var last = try XCTUnwrap(FirstRunProgress(step: step, folders: true, scan: true)).current
        while let previous = step.previous(privacyScanConfigured: true), previous != .welcome {
            let current = try XCTUnwrap(FirstRunProgress(step: previous, folders: true, scan: true)).current
            XCTAssertLessThan(current, last, "\(previous)")
            last = current
            step = previous
        }
    }

    /// Whether the folders step shows is read only once the core's startup
    /// is known: never decided while it is still starting.
    func test_theFoldersQuestionWaitsForStartup() {
        XCTAssertNil(FirstRunProgress.asksForFolders(.starting))
        XCTAssertEqual(FirstRunProgress.asksForFolders(.needsRoots), true)
        XCTAssertEqual(FirstRunProgress.asksForFolders(.running), false)
    }

    /// The window is gated as the main window gates onboarding: the flow
    /// is drawn only while onboarding is required, the window closes when
    /// it is not, and completing never closes it by itself.
    func test_theWindowIsGatedOnRequiresOnboarding() throws {
        let source = try String(contentsOf: Self.source("Views/Monitor/FirstRunViews.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains("if model.requiresOnboarding {"))
        XCTAssertTrue(source.contains("onChange(of: model.requiresOnboarding, initial: true)"))
        XCTAssertEqual(source.components(separatedBy: "dismissWindow(id:").count - 1, 1,
                       "the window closes only from the requiresOnboarding gate")
    }

    private static func source(_ path: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
    }

    /// A daemon that already has its folders skips the roots step, so the
    /// progress starts at Join rather than showing Folders as done.
    func test_theFoldersStepShowsOnlyWhenAsked() throws {
        let progress = try XCTUnwrap(FirstRunProgress(step: .connect, folders: false, scan: false))
        XCTAssertEqual(progress.labels.first, FirstRunWords.join)
        XCTAssertEqual(progress.current, 0)
        XCTAssertNil(FirstRunProgress(step: .roots, folders: false, scan: false))
    }
}
