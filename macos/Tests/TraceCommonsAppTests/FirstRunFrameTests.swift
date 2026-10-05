import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's first-run frame (#1030 `ftux-frame.tsx`): the tier as the eyebrow,
/// the tier's step labels in the progress, and "Custom setup instead" only on
/// Quick's Folders (`tool-screens.tsx`). Read from the real core table and
/// the frame's source, the house pattern for a SwiftUI view.
final class FirstRunFrameTests: XCTestCase {
    private func copy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private static func source() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/FirstRunFrame.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    func test_theFrameDrawsTheTierAndRonsStepLabels() throws {
        let frame = try copy().frame
        XCTAssertEqual(frame.steps(for: .quick), [frame.stepJoin, frame.stepFolders, frame.stepUses])
        XCTAssertEqual(
            frame.steps(for: .custom), [frame.stepJoin, frame.stepTools, frame.stepRules, frame.stepUses])
        XCTAssertEqual(frame.eyebrow(for: .quick), frame.quickSetup)
        XCTAssertEqual(frame.eyebrow(for: .custom), frame.customSetup)

        // The current node is the state's step within its tier's list.
        for tier in [FirstRunTier.quick, .custom] {
            for (index, step) in FirstRunNavigation.steps(for: tier).enumerated() {
                XCTAssertEqual(FirstRunFrameLayout.current(FirstRunState(tier: tier, step: step)), index)
            }
        }

        let source = try Self.source()
        XCTAssertTrue(source.contains("GlassPane"))
        XCTAssertTrue(source.contains("copy.frame.eyebrow(for: state.tier)"))
        XCTAssertTrue(source.contains("GlassStepProgress(labels: copy.frame.steps(for: state.tier)"))
    }

    func test_customSetupInsteadAppearsOnlyOnQuickFolders() throws {
        var offered: [FirstRunState] = []
        for tier in [FirstRunTier.quick, .custom] {
            for step in FirstRunNavigation.steps(for: tier) {
                let state = FirstRunState(tier: tier, step: step)
                if FirstRunFrameLayout.offersCustomSetupInstead(state) { offered.append(state) }
            }
        }
        XCTAssertEqual(offered.map(\.tier), [.quick])
        XCTAssertEqual(offered.map(\.step), [.folders])

        // The link exists once, behind that guard, and switches the tier.
        let source = try Self.source()
        XCTAssertEqual(source.components(separatedBy: "copy.frame.customSetupInstead").count - 1, 1)
        let guardRange = try XCTUnwrap(source.range(of: "FirstRunFrameLayout.offersCustomSetupInstead(state)"))
        let linkRange = try XCTUnwrap(source.range(of: "copy.frame.customSetupInstead"))
        XCTAssertLessThan(guardRange.lowerBound, linkRange.lowerBound)
        XCTAssertTrue(source.contains("FirstRunNavigation.switchTier(state, to: .custom)"))
    }

    /// Ron's Continue carries "Answer every tool above to continue" as its
    /// disabled help on the tool screens only; elsewhere the reason differs.
    func test_answerEveryToolIsTheDisabledHelpOnToolScreensOnly() {
        let disabled = FirstRunFooter(title: "", isEnabled: false, action: {})
        let enabled = FirstRunFooter(title: "", isEnabled: true, action: {})
        for tier in [FirstRunTier.quick, .custom] {
            for step in FirstRunNavigation.steps(for: tier) {
                let state = FirstRunState(tier: tier, step: step)
                let toolScreen = step == .folders || step == .tools
                XCTAssertEqual(FirstRunFrameLayout.showsAnswerEveryTool(state, footer: disabled), toolScreen)
                XCTAssertFalse(FirstRunFrameLayout.showsAnswerEveryTool(state, footer: enabled))
            }
        }
    }
}
