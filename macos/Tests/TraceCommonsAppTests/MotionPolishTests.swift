#if DEBUG
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// Motion rules from a review against Emil Kowalski's animation skills
/// (2026-11-09): UI motion stays under 0.3s, enters and exits start fast,
/// nothing grows out of nothing, and a spring bounces only after a gesture
/// that carried momentum.
@MainActor
final class MotionPolishTests: XCTestCase {
    /// A graph period step is pressed several times in a row, so its slide
    /// stays under 0.3s and runs on the shell's one curve.
    func test_aGraphPeriodStepSlidesUnderAThirdOfASecond() throws {
        XCTAssertLessThan(TracesGraphFooter.slideDuration, 0.3)
        XCTAssertEqual(TracesGraphFooter.slideDuration, GlassTokens.Motion.slide)
        let shell = try Self.text("Views/Monitor/MonitorShell.swift")
        XCTAssertTrue(shell.contains("reduceMotion ? nil : GlassMotion.curve(slideDuration)"))
    }

    /// The graph footer enters and leaves, so it starts fast: no ease-in.
    func test_theGraphFooterOpensOnTheShellCurve() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains(".easeInOut("), "the footer toggle eases in again")
        XCTAssertTrue(window.contains(
            "withAnimation(GlassMotion.systemReducesMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide))"))
    }

    /// A Uses-ring satellite grows from a seed of itself, never from zero.
    func test_aSatelliteNeverGrowsFromNothing() throws {
        XCTAssertGreaterThan(FirstRunStageLayout.satelliteSeedScale, 0)
        XCTAssertLessThan(FirstRunStageLayout.satelliteSeedScale, 1)
        let uses = try Self.text("Views/FirstRun/UsesScreen.swift")
        XCTAssertFalse(uses.contains(".scaleEffect(shown ? 1 : 0)"))
    }

    /// A map coin grows because a run started, so it settles without a
    /// bounce.
    func test_aGrowingCoinDoesNotBounce() throws {
        let pile = try Self.text("Views/Monitor/MapToolPile.swift")
        XCTAssertTrue(pile.contains(".spring(response: 0.3, dampingFraction: 0.9), value: grown)"))
        XCTAssertFalse(pile.contains("dampingFraction: 0.7), value: grown)"))
    }

    private static func text(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp").appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }
}
#endif
