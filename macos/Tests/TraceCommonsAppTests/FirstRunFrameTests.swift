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
        let guardRange = try XCTUnwrap(source.range(of: "FirstRunFrameLayout.offersCustomSetupInstead(state, isCommitting: isCommitting)"))
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

    private static func appSource(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    /// The five screens, each with its copy group and layout enum.
    private static let screens: [(file: String, group: String, layout: String, type: String)] = [
        ("JoinScreen.swift", "join", "JoinScreenLayout", "JoinScreen"),
        ("FoldersScreen.swift", "folders", "FoldersScreenLayout", "FoldersScreen"),
        ("ToolsScreen.swift", "tools", "ToolsScreenLayout", "ToolsScreen"),
        ("RulesScreen.swift", "rules", "RulesScreenLayout", "RulesScreen"),
        ("UsesScreen.swift", "uses", "UsesScreenLayout", "UsesScreen"),
    ]

    /// Ron draws every screen's title with one `ScreenTitle`; so does this
    /// shell, with one weight for the bold half.
    func test_everyScreenDrawsItsTitleThroughOneHelper() throws {
        for screen in Self.screens {
            let source = try Self.appSource("Views/FirstRun/\(screen.file)")
            XCTAssertTrue(
                source.contains(
                    "FirstRunTitle(light: copy.\(screen.group).titleLight, bold: copy.\(screen.group).titleBold)"),
                screen.file)
            XCTAssertFalse(source.contains("titleBold).bold()"), screen.file)
            XCTAssertFalse(source.contains("titleBold).fontWeight("), screen.file)
            XCTAssertFalse(source.contains(".semibold"), screen.file)
        }
        XCTAssertTrue(try Self.source().contains("struct FirstRunTitle: View"))
    }

    /// Every screen has the same shape: the copy and the runner, and a
    /// `<Screen>Layout` holding its decisions.
    func test_everyScreenTakesTheRunnerAndNamesItsLayoutAlike() throws {
        for screen in Self.screens {
            let source = try Self.appSource("Views/FirstRun/\(screen.file)")
            XCTAssertTrue(source.contains("enum \(screen.layout) {"), screen.file)
            XCTAssertTrue(source.contains("@ObservedObject var runner: FirstRunRunner"), screen.file)
            XCTAssertFalse(source.contains("@Binding var state: FirstRunState"), screen.file)
        }
        let coordinator = try Self.appSource("Views/OnboardingCoordinatorView.swift")
        for screen in Self.screens {
            XCTAssertTrue(coordinator.contains("\(screen.type)(copy: copy, runner: runner"), screen.type)
        }
    }

    /// One folder panel and one placeholder filler, used everywhere.
    func test_oneFolderPanelAndOnePlaceholderFiller() throws {
        let base = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
        let walker = try XCTUnwrap(FileManager.default.enumerator(at: base, includingPropertiesForKeys: nil))
        var panels = 0
        var fills: [String] = []
        for case let url as URL in walker where url.pathExtension == "swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            panels += text.components(separatedBy: "NSOpenPanel()").count - 1
            if url.path.contains("/Views/FirstRun/"), text.contains("replacingOccurrences(of: \"{") {
                fills.append(url.lastPathComponent)
            }
        }
        XCTAssertEqual(panels, 1, "one folder panel")
        XCTAssertEqual(fills, [], "placeholders are filled by FirstRunCopy.fill")
        XCTAssertEqual(
            FirstRunCopy.fill("{count} of {total} in {folder}", ["count": "2", "total": "5", "folder": "app"]),
            "2 of 5 in app")
    }

    /// Every first-run screen's title is a VoiceOver heading, so heading
    /// navigation (VO-Command-H) finds it.
    func test_theScreenTitleIsAVoiceOverHeading() throws {
        let source = try Self.source()
        let title = try XCTUnwrap(source.range(of: "struct FirstRunTitle: View {"))
        let body = String(source[title.upperBound...].prefix(400))
        XCTAssertTrue(body.contains(".accessibilityAddTraits(.isHeader)"), "FirstRunTitle must carry the header trait")
    }
}
