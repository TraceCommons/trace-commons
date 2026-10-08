import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's first-run frame (#1030 `ftux-frame.tsx`): the tier as the eyebrow
/// (Custom setup only, owner 2026-10-08),
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
        // No tier tag on either setup (owner, 2026-10-08): the tier is only
        // the pane's accessible name.
        XCTAssertEqual(source.components(separatedBy: "copy.frame.eyebrow(for: state.tier)").count - 1, 1)
        XCTAssertTrue(source.contains(".accessibilityLabel(copy.frame.eyebrow(for: state.tier))"))
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

        // Withdrawn while a commit runs.
        XCTAssertFalse(
            FirstRunFrameLayout.offersCustomSetupInstead(
                FirstRunState(tier: .quick, step: .folders), isCommitting: true))

        // The button exists once, behind that guard, and switches the tier.
        let source = try Self.source()
        XCTAssertEqual(source.components(separatedBy: "copy.frame.customSetupInstead").count - 1, 1)
        let guardRange = try XCTUnwrap(source.range(of: "FirstRunFrameLayout.offersCustomSetupInstead(state, isCommitting: isCommitting)"))
        let linkRange = try XCTUnwrap(source.range(of: "copy.frame.customSetupInstead"))
        XCTAssertLessThan(guardRange.lowerBound, linkRange.lowerBound)
        XCTAssertTrue(source.contains("FirstRunNavigation.switchTier(state, to: .custom)"))

        // Owner, 2026-10-08: in the footer's right side, immediately left of
        // Continue, as the neutral secondary button, not a link.
        let row = try XCTUnwrap(source.range(of: "private var footerRow: some View {"))
        let rowBody = String(source[row.upperBound...])
        let spacer = try XCTUnwrap(rowBody.range(of: "Spacer(minLength: 0)"))
        let customize = try XCTUnwrap(rowBody.range(of: "copy.frame.customSetupInstead"))
        let primary = try XCTUnwrap(rowBody.range(of: "Button(action: footer.action)"))
        XCTAssertLessThan(spacer.lowerBound, customize.lowerBound)
        XCTAssertLessThan(customize.lowerBound, primary.lowerBound)
        let customizeBody = String(rowBody[customize.upperBound...].prefix(200))
        XCTAssertTrue(customizeBody.contains(".buttonStyle(GlassButtonStyle(.glass))"), customizeBody)
        XCTAssertFalse(source.contains("GlassButtonStyle(.link)"))
    }

    /// Owner, 2026-10-08 (reversing Ron's review of #1235, item 9): Back on
    /// the footer's left on every step after Join, in either tier, never on
    /// Join, withdrawn while a commit runs, in the core's word, as the
    /// neutral secondary button. It goes to the previous step.
    func test_backIsOfferedOnEveryStepAfterJoin() throws {
        var offered: [FirstRunState] = []
        for tier in [FirstRunTier.quick, .custom] {
            for step in FirstRunNavigation.steps(for: tier) {
                let state = FirstRunState(tier: tier, step: step)
                XCTAssertFalse(FirstRunFrameLayout.offersBack(state, isCommitting: true), "\(tier) \(step)")
                if FirstRunFrameLayout.offersBack(state) { offered.append(state) }
            }
        }
        XCTAssertEqual(offered.map(\.step), [.folders, .uses, .tools, .rules, .uses])
        XCTAssertFalse(offered.contains { $0.step == .join })

        // Back from Quick's Folders is Join, from Quick's Uses is Folders.
        XCTAssertEqual(FirstRunNavigation.back(FirstRunState(tier: .quick, step: .folders)).step, .join)
        XCTAssertEqual(FirstRunNavigation.back(FirstRunState(tier: .quick, step: .uses)).step, .folders)

        XCTAssertEqual(try copy().frame.back, "Back")
        let source = try Self.source()
        let row = try XCTUnwrap(source.range(of: "private var footerRow: some View {"))
        let rowBody = String(source[row.upperBound...])
        let back = try XCTUnwrap(rowBody.range(of: "if FirstRunFrameLayout.offersBack(state, isCommitting: isCommitting) {"))
        let spacer = try XCTUnwrap(rowBody.range(of: "Spacer(minLength: 0)"))
        XCTAssertLessThan(back.lowerBound, spacer.lowerBound, "Back sits on the left")
        let backBody = String(rowBody[back.upperBound..<spacer.lowerBound])
        XCTAssertTrue(backBody.contains("Button(copy.frame.back)"))
        XCTAssertTrue(backBody.contains("state = FirstRunNavigation.back(state)"))
        XCTAssertTrue(backBody.contains(".buttonStyle(GlassButtonStyle(.glass))"))
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

    /// Cancel during the near.ai browser wait (owner, 2026-10-07): only
    /// while the sign-in runs, in the core's first-run "Cancel", as a small
    /// secondary button beside the spinning Continue, on Folders and Tools
    /// only, and nowhere else in Ron's footer.
    func test_cancelAppearsOnlyWhileTheSignInRuns() throws {
        let words: FirstRunCopy = try copy()
        XCTAssertNil(FoldersScreenLayout.signInCancel(waiting: false, copy: words, action: {}))
        let cancel = try XCTUnwrap(FoldersScreenLayout.signInCancel(waiting: true, copy: words, action: {}))
        XCTAssertEqual(cancel.title, words.passkey.cancel)
        XCTAssertNil(FirstRunFooter(title: "", isEnabled: false, action: {}).cancel, "no Cancel unless a screen gives one")

        for file in ["FoldersScreen.swift", "ToolsScreen.swift"] {
            let source = try Self.appSource("Views/FirstRun/\(file)")
            XCTAssertTrue(
                source.contains("cancel: FoldersScreenLayout.signInCancel(waiting: runner.signInWaiting, copy: copy)"),
                file)
            XCTAssertTrue(source.contains("await runner.cancelSignIn()"), file)
        }
        for file in ["JoinScreen.swift", "RulesScreen.swift", "UsesScreen.swift"] {
            XCTAssertFalse(try Self.appSource("Views/FirstRun/\(file)").contains("signInCancel"), file)
        }

        // In the footer row: after the spacer, before Continue, secondary.
        let frame = try Self.source()
        let row = try XCTUnwrap(frame.range(of: "private var footerRow: some View {"))
        let rowBody = String(frame[row.upperBound...])
        let spacer = try XCTUnwrap(rowBody.range(of: "Spacer(minLength: 0)"))
        let cancelButton = try XCTUnwrap(rowBody.range(of: "if let cancel = footer.cancel {"))
        let primary = try XCTUnwrap(rowBody.range(of: "Button(action: footer.action)"))
        XCTAssertLessThan(spacer.lowerBound, cancelButton.lowerBound)
        XCTAssertLessThan(cancelButton.lowerBound, primary.lowerBound)
        let cancelBody = String(rowBody[cancelButton.upperBound..<primary.lowerBound])
        XCTAssertTrue(cancelBody.contains("Button(cancel.title, action: cancel.action)"))
        XCTAssertTrue(cancelBody.contains("GlassButtonStyle(.secondary)"))
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

    /// Ron's review of #1235, items 10 to 12 (the tier tag since removed,
    /// owner 2026-10-08): the 450pt pane, titles fixed while only the cards
    /// scroll, and an error notice in the body after the cards. Back is the
    /// frame's alone (owner, 2026-10-08): no screen draws one of its own.
    func test_theFrameMatchesRonsLayout() throws {
        let frame = try Self.source()
        XCTAssertEqual(frame.components(separatedBy: "Button(copy.frame.back)").count - 1, 1)
        // No tier tag on either setup (owner, 2026-10-08).
        XCTAssertFalse(frame.contains("private var bar: some View {"))
        // The header is fixed above the one ScrollView; the notice follows
        // the cards inside it.
        let body = try XCTUnwrap(frame.range(of: "GlassPane {"))
        let frameBody = String(frame[body.upperBound...].prefix(1200))
        let header = try XCTUnwrap(frameBody.range(of: "header\n"))
        let scroll = try XCTUnwrap(frameBody.range(of: "ScrollView {"))
        let content = try XCTUnwrap(frameBody.range(of: "content\n"))
        let notice = try XCTUnwrap(frameBody.range(of: "if let notice {"))
        XCTAssertLessThan(header.lowerBound, scroll.lowerBound)
        XCTAssertLessThan(scroll.lowerBound, content.lowerBound)
        XCTAssertLessThan(content.lowerBound, notice.lowerBound)

        for screen in Self.screens {
            let source = try Self.appSource("Views/FirstRun/\(screen.file)")
            XCTAssertFalse(source.contains("onBack"), screen.file)
            XCTAssertFalse(source.contains("copy.frame.back"), screen.file)
            XCTAssertFalse(source.contains("FirstRunNavigation.back("), screen.file)
            XCTAssertFalse(source.contains("ScrollView"), "\(screen.file): only the frame scrolls")
            // The frame's header closure, between its footer and
            // `content:`, holds the title.
            let frameCall = try XCTUnwrap(source.range(of: "FirstRunFrame("), screen.file)
            let cards = try XCTUnwrap(source.range(of: "} content: {"), screen.file)
            let header = String(source[frameCall.upperBound..<cards.lowerBound])
            XCTAssertTrue(
                header.contains("FirstRunTitle(")
                    || header.split(separator: "\n").contains { $0.trimmingCharacters(in: .whitespaces) == "title" },
                "\(screen.file): the title is the fixed header")
        }

        // The pane is Ron's 450pt, in the first-run window and in the
        // Inference tab's host alike.
        XCTAssertEqual(FirstRunProgress.paneWidth, 450)
        let inference = try Self.appSource("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(inference.contains(".frame(width: FirstRunProgress.paneWidth)"))
    }

    /// Ron's review of #1235, item 15: the smaller visual details.
    func test_theSmallerDetailsFollowRonsDesign() throws {
        // The add-tool box: dashed border, a "+" tile, purple while dragged.
        let tools = try Self.appSource("Views/FirstRun/ToolsScreen.swift")
        XCTAssertTrue(tools.contains("GlassToolTile(.add, large: true)"))
        XCTAssertTrue(tools.contains("dash: [4, 3]"))
        XCTAssertTrue(tools.contains("GlassTokens.Color.purpleText"))
        XCTAssertFalse(tools.contains(".opacity(dragging"))
        // "Get {tool}": the neutral glass pill with the download icon.
        let row = try Self.appSource("Views/FirstRun/ToolAnswerRow.swift")
        XCTAssertTrue(row.contains("GlassButtonStyle(.glass)"))
        XCTAssertTrue(row.contains("systemImage: \"arrow.down.to.line\""))
        // Start sharing shows a spinner while it runs.
        let uses = try Self.appSource("Views/FirstRun/UsesScreen.swift")
        XCTAssertTrue(uses.contains("busy: runner.isCommitting"))
        // Continue on Folders and Tools too: leaving them can wait on the
        // near.ai browser sign-in.
        for file in ["FoldersScreen.swift", "ToolsScreen.swift"] {
            XCTAssertTrue(try Self.appSource("Views/FirstRun/\(file)").contains("busy: runner.isCommitting"), file)
        }
        let frame = try Self.source()
        XCTAssertTrue(frame.contains("if footer.busy { GlassSpinner() }"))
        // "required" is inline text in the on colour, not a tag.
        XCTAssertFalse(uses.contains("GlassTag(copy.uses.required"))
        XCTAssertTrue(uses.contains("Text(copy.uses.required)"))
        XCTAssertTrue(uses.contains("GlassTokens.Color.statusOnText"))
        // The Private AI switch is the settings style.
        XCTAssertTrue(uses.contains("GlassToggleStyle(.settings, showsLabel: false)"))
        // Join's bold sentence is in the primary text colour, its own
        // paragraph (owner, 2026-10-08).
        let join = try Self.appSource("Views/FirstRun/JoinScreen.swift")
        let emphasis = try XCTUnwrap(join.range(of: "Text(copy.join.bodyEmphasis)"))
        let emphasisStyle = String(join[emphasis.upperBound...].prefix(200))
        XCTAssertTrue(emphasisStyle.contains(".weight(.bold)"))
        XCTAssertTrue(emphasisStyle.contains(".foregroundStyle(GlassColor.textPrimary)"))
        // Never rows on Rules are dimmed by ink, never by an opacity that
        // takes their words under 4.5:1.
        let rules = try Self.appSource("Views/FirstRun/RulesScreen.swift")
        XCTAssertFalse(rules.contains(".opacity(GlassTokens.Opacity.rowOff)"))
        XCTAssertTrue(rules.contains("dimmed by ink, not"))
    }
}
