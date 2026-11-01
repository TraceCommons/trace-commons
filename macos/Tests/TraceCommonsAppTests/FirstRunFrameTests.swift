import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The animated first run's frame (Ron's 2026-10-21 canvas): the step
/// thread's names, each step's graphic, title and card spaced evenly, and
/// the action bar with Back, a link and the primary action. Read from the
/// real core table and the sources, the house pattern for a SwiftUI view.
final class FirstRunFrameTests: XCTestCase {
    private func words() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private static func source() throws -> String {
        try appSource("Views/FirstRun/FirstRunFrame.swift")
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

    /// The thread names every step after Welcome, in the core's words and
    /// `FirstRunNavigation`'s order.
    func test_theThreadNamesTheStepsInTheCoresWords() throws {
        let copy = try words()
        XCTAssertEqual(copy.threadLabels, [
            copy.frame.stepTools, copy.frame.stepUses, copy.frame.stepTraces, copy.frame.stepPrivateAi,
            copy.frame.stepJoin,
        ])
        XCTAssertNil(copy.threadLabel(for: .welcome), "Welcome shows no thread")
        XCTAssertEqual(FirstRunStageLayout.threadIndex(of: .welcome), nil)
        XCTAssertEqual(FirstRunStageLayout.threadIndex(of: .tools), 0)
        XCTAssertEqual(FirstRunStageLayout.threadIndex(of: .join), 4)
    }

    /// The stretch before a step is travelled once it is reached; the
    /// newest one leads, and the rest is dotted.
    func test_theThreadGlowsUpToTheCurrentStep() {
        XCTAssertEqual(FirstRunStageLayout.segment(before: 1, current: 2), .travelled)
        XCTAssertEqual(FirstRunStageLayout.segment(before: 2, current: 2), .leading)
        XCTAssertEqual(FirstRunStageLayout.segment(before: 3, current: 2), .ahead)
    }

    /// Back on every step after Welcome, withdrawn while a commit runs, in
    /// the core's word, as the glass bar button; it goes to the previous
    /// step.
    func test_backIsOfferedOnEveryStepAfterWelcome() throws {
        for step in FirstRunNavigation.steps {
            let state = FirstRunState(step: step)
            XCTAssertEqual(FirstRunFrameLayout.offersBack(state), step != .welcome, "\(step)")
            XCTAssertFalse(FirstRunFrameLayout.offersBack(state, isCommitting: true), "\(step)")
        }
        let frame = try Self.source()
        XCTAssertEqual(frame.components(separatedBy: "Button(copy.frame.back)").count - 1, 1)
        let back = try XCTUnwrap(frame.range(of: "Button(copy.frame.back)"))
        let after = String(frame[back.upperBound...].prefix(200))
        XCTAssertTrue(after.contains("FirstRunNavigation.back(state)"))
        XCTAssertTrue(after.contains("GlassButtonStyle(.glass, size: .stage)"))
    }

    /// The action bar: Back on its left, then a spacer, then the link
    /// (Customize), any Cancel, and the primary action, in that order.
    func test_theActionBarHoldsBackLinkCancelAndTheAction() throws {
        let frame = try Self.source()
        let bar = try XCTUnwrap(frame.range(of: "private var actionBar: some View {"))
        let body = String(frame[bar.upperBound...])
        let order = [
            "Button(copy.frame.back)", "Spacer(minLength: 0)", "if let link = footer.link {",
            "if let cancel = footer.cancel {", "Button(action: footer.action)",
        ].map { body.range(of: $0)?.lowerBound }
        XCTAssertFalse(order.contains(nil), "\(order)")
        let found = order.compactMap { $0 }
        XCTAssertEqual(found, found.sorted())
        XCTAssertTrue(body.contains("GlassButtonStyle(.secondary, size: .stage)"))
        XCTAssertTrue(body.contains("GlassButtonStyle(.link, size: .stage)"))
        XCTAssertTrue(body.contains("GlassButtonStyle(.primary, size: .stage)"), "the canvas's semibold Continue")
        XCTAssertTrue(body.contains("if footer.busy { GlassSpinner() }"))
        XCTAssertNil(FirstRunFooter(title: "", isEnabled: false, action: {}).cancel, "no Cancel unless a screen gives one")
        XCTAssertNil(FirstRunFooter(title: "", isEnabled: false, action: {}).link, "no link unless a screen gives one")
    }

    /// Text about taking the action sits directly above the action bar.
    func test_actionNotesSitDirectlyAboveTheActionBar() throws {
        XCTAssertEqual(
            FirstRunFrameLayout.actionNotes(footer: FirstRunFooter(title: "", isEnabled: true, note: "n", action: {})),
            ["n"])
        XCTAssertEqual(
            FirstRunFrameLayout.actionNotes(footer: FirstRunFooter(title: "", isEnabled: true, note: "", action: {})), [])
        let frame = try Self.source()
        let notes = try XCTUnwrap(frame.range(of: "ForEach(FirstRunFrameLayout.actionNotes(footer: footer)"))
        let bar = try XCTUnwrap(frame.range(of: "            actionBar\n"))
        XCTAssertLessThan(notes.lowerBound, bar.lowerBound)
    }

    /// Ron, 2026-10-21: each screen splits its free height equally three
    /// ways -- above the graphic, between the graphic and the title, and
    /// below the card -- measured from under the step thread, with the
    /// title under the graphic.
    func test_everyStepIsSpacedEvenlyUnderTheThread() throws {
        let frame = try Self.source()
        let body = try XCTUnwrap(frame.range(of: "var body: some View {"))
        let rest = String(frame[body.upperBound...])
        XCTAssertEqual(rest.components(separatedBy: "Spacer(minLength: FirstRunFrameLayout.minimumGap)").count - 1, 3)
        let graphic = try XCTUnwrap(rest.range(of: "                graphic\n"))
        let title = try XCTUnwrap(rest.range(of: "                title\n"))
        let content = try XCTUnwrap(rest.range(of: "            content\n"))
        XCTAssertLessThan(graphic.lowerBound, title.lowerBound)
        XCTAssertLessThan(title.lowerBound, content.lowerBound)
        XCTAssertTrue(rest.contains(".padding(.top, FirstRunFrameLayout.threadBottom)"))
    }

    /// The step screens, each with its copy group and layout enum.
    private static let screens: [(file: String, group: String, layout: String, type: String)] = [
        ("ToolsScreen.swift", "tools", "ToolsScreenLayout", "ToolsScreen"),
        ("UsesScreen.swift", "uses", "UsesScreenLayout", "UsesScreen"),
        ("TracesScreen.swift", "traces", "TracesScreenLayout", "TracesScreen"),
        ("PrivateAIScreen.swift", "privateAi", "PrivateAIScreenLayout", "PrivateAIScreen"),
        ("JoinScreen.swift", "join", "JoinScreenLayout", "JoinScreen"),
    ]

    /// Every step's title goes through one helper, with one weight for the
    /// bold half, and is a VoiceOver heading.
    func test_everyStepDrawsItsTitleThroughOneHelper() throws {
        for screen in Self.screens {
            let source = try Self.appSource("Views/FirstRun/\(screen.file)")
            XCTAssertTrue(
                source.contains(
                    "FirstRunTitle(light: copy.\(screen.group).titleLight, bold: copy.\(screen.group).titleBold)"),
                screen.file)
            XCTAssertFalse(source.contains("titleBold).bold()"), screen.file)
        }
        let frame = try Self.source()
        let title = try XCTUnwrap(frame.range(of: "struct FirstRunTitle: View {"))
        XCTAssertTrue(String(frame[title.upperBound...].prefix(500)).contains(".accessibilityAddTraits(.isHeader)"))
    }

    /// Every screen takes the copy and the runner and keeps its decisions in
    /// a `<Screen>Layout`; the host draws each one.
    func test_everyScreenTakesTheRunnerAndNamesItsLayoutAlike() throws {
        let coordinator = try Self.appSource("Views/OnboardingCoordinatorView.swift")
        for screen in Self.screens + [("WelcomeScreen.swift", "welcome", "WelcomeScreenLayout", "WelcomeScreen")] {
            let source = try Self.appSource("Views/FirstRun/\(screen.file)")
            XCTAssertTrue(source.contains("enum \(screen.layout) {"), screen.file)
            XCTAssertTrue(source.contains("@ObservedObject var runner: FirstRunRunner"), screen.file)
            XCTAssertFalse(source.contains("@Binding var state: FirstRunState"), screen.file)
            XCTAssertTrue(coordinator.contains("\(screen.type)(copy: copy, runner: runner"), screen.type)
            // Back is the frame's alone.
            XCTAssertFalse(source.contains("copy.frame.back"), screen.file)
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
    }

    /// The first-run window fills with the stage; the Private AI tab hosts
    /// the same stage, scaled down to fit (`FirstRunStageFit`).
    func test_theStageFillsItsHosts() throws {
        let window = try Self.appSource("Views/Monitor/FirstRunViews.swift")
        XCTAssertTrue(window.contains("FirstRunBackdrop()"))
        XCTAssertTrue(window.contains("OnboardingCoordinatorView(onComplete: {})"))
        XCTAssertFalse(window.contains("fixedSize("), "a vertical fixedSize grows the stage past the window")
        let inference = try Self.appSource("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(inference.contains("OnboardingCoordinatorView(startAt: .tools, takesInvites: false, onComplete: {})"))
        let host = try Self.appSource("Views/OnboardingCoordinatorView.swift")
        XCTAssertTrue(host.contains("FirstRunStageFit {"))
        XCTAssertEqual(FirstRunStageLayout.canvas, CGSize(width: 860, height: 760))
    }

    /// Ron's review of #1235, item 15, as it still applies: the add box is
    /// dashed and turns the accent while a folder is dragged over it, the
    /// required use reads "required" inline in the on colour, and Start
    /// sharing spins while it runs.
    func test_theSmallerDetailsFollowRonsDesign() throws {
        let tools = try Self.appSource("Views/FirstRun/ToolsScreen.swift")
        XCTAssertTrue(tools.contains("dash: [4, 3]"))
        XCTAssertTrue(tools.contains("GlassTokens.Color.purpleText"))
        let uses = try Self.appSource("Views/FirstRun/UsesScreen.swift")
        XCTAssertFalse(uses.contains("GlassTag(copy.uses.required"))
        // The required use is one "Always on" line now (owner, 2026-10-28).
        XCTAssertFalse(uses.contains("Text(copy.uses.required)"))
        XCTAssertTrue(uses.contains("GlassTokens.Color.statusOnText"))
        let join = try Self.appSource("Views/FirstRun/JoinScreen.swift")
        XCTAssertTrue(join.contains("busy: starting && runner.isCommitting"))
        XCTAssertFalse(join.contains("bodyEmphasis"))
        let privateAI = try Self.appSource("Views/FirstRun/PrivateAIScreen.swift")
        // Ron's switch row (2026-10-22) is gone (owner, 2026-10-27: the
        // switch changed little); Learn more stays on the row's right.
        XCTAssertFalse(privateAI.contains("GlassToggleStyle(.watch"))
        XCTAssertTrue(privateAI.contains("Text(copy.privateAi.learnMore)"))
    }
}
