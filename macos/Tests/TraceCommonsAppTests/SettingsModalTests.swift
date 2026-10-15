import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Settings is its own window (owner, 2026-10-08), beside the Monitor and
/// not blocking it; its body is still Ron's #1146 Settings (`settings-modal.tsx`):
/// the opaque pane, the section list and one scrolling body. Until then it
/// was a modal over the Monitor (#1241 Task 10, owner 2026-10-05).
@MainActor
final class SettingsModalTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Cmd-comma, the gear, the menu-bar panel and a Settings destination
    /// each ask for Settings and open its window; the Monitor draws no
    /// Settings of its own.
    func test_settingsIsItsOwnWindow() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("Window(MonitorWords.table?.settingsTitle ?? \"\", id: WindowID.settings) {"))
        XCTAssertTrue(main.contains("SettingsWindowView(navigation: navigation)"))
        XCTAssertTrue(main.contains("static let settings = \"trace-commons-settings\""))
        XCTAssertTrue(main.contains("MonitorWindowView(navigation: navigation, "))
        // Cmd-comma: the app menu's Settings item opens the window.
        XCTAssertTrue(main.contains("CommandGroup(replacing: .appSettings)"))
        XCTAssertTrue(main.contains(".keyboardShortcut(\",\", modifiers: .command)"))
        let command = try XCTUnwrap(main.components(separatedBy: "struct OpenSettingsButton: View {").last)
        XCTAssertTrue(command.contains("navigation.requestSettings()\n            openWindow(id: WindowID.settings)"))
        // A Settings destination opens the window at its section.
        XCTAssertTrue(main.contains("navigation.requestSettings(at: section)\n            openWindow(id: WindowID.settings)"))
        // The window draws the Settings body at the last request.
        let host = try XCTUnwrap(main.components(separatedBy: "struct SettingsWindowView: View {").last)
        XCTAssertTrue(host.contains("request: navigation.settingsRequest ?? SettingsRequest(section: nil)"))
        XCTAssertTrue(host.contains("onClose: { dismiss() }"))
        XCTAssertTrue(host.contains(".glassModalHost()"), "a section's confirmations have no host")

        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("navigation.requestSettings()\n                    openWindow(id: WindowID.settings)"),
                      "the gear does not open the Settings window")
        XCTAssertFalse(window.contains("SettingsModal("), "the Monitor still draws Settings over its panes")
        XCTAssertFalse(window.contains("navigation.settingsRequest"), "the Monitor still blocks on Settings")
        // The name #1242 still mounts is kept, and it routes to the window.
        let kept = try XCTUnwrap(window.components(separatedBy: "struct MonitorSettingsWindow: View {").last)
        XCTAssertTrue(kept.contains("navigation.requestSettings()"))
        XCTAssertTrue(kept.contains("openWindow(id: WindowID.settings)"))
        XCTAssertFalse(kept.contains("NavigationSplitView"), "the old window is still drawn")

        // Nothing opens the system Settings scene: this window is the one.
        let walker = try XCTUnwrap(FileManager.default.enumerator(
            at: GlassSurfaceRulesTests.root, includingPropertiesForKeys: nil))
        for case let url as URL in walker where url.pathExtension == "swift" {
            let source = try String(contentsOf: url, encoding: .utf8)
            XCTAssertFalse(source.contains("openSettings()"), url.lastPathComponent)
        }

        // A request names its section, or none (the gear); each is new, so
        // asking again for the same section scrolls to it again.
        let navigation = MainWindowNavigation()
        XCTAssertNil(navigation.settingsRequest)
        navigation.requestSettings(at: .witness)
        let first = try XCTUnwrap(navigation.settingsRequest)
        XCTAssertEqual(first.section, .witness)
        navigation.requestSettings(at: .witness)
        XCTAssertNotEqual(navigation.settingsRequest, first)
        navigation.requestSettings()
        XCTAssertNotNil(navigation.settingsRequest)
        XCTAssertNil(navigation.settingsRequest?.section)
    }

    /// The window's body fills it on the opaque pane base, never a
    /// translucent material, with no pane inside the window (owner,
    /// 2026-10-09: no window framed within the window), and no longer a
    /// scrim over the Monitor.
    func test_theWindowIsOpaque() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".background(GlassTokens.Color.paneOpaque.color)"), "Settings is not on the opaque base")
        XCTAssertTrue(modal.contains(".environment(\\.glassPaneIsContent, true)"))
        XCTAssertFalse(modal.contains("GlassPane("), "a pane framed within the Settings window")
        XCTAssertEqual(GlassMaterial.current(content: true), .opaque)
        XCTAssertFalse(modal.contains("GlassTokens.Color.modalScrim.color"), "a scrim is left in a window")
        for translucent in ["Material", ".glassSurface(", "glassEffect", "floating: true"] {
            XCTAssertFalse(modal.contains(translucent), translucent)
        }
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/Monitor/SettingsModal.swift"))
        // The Monitor is neither dimmed nor blurred while Settings is open.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("GlassTokens.Size.modalScrimBlur"))
    }

    /// Two columns: the section list, and one scrolling body holding every
    /// section in order, Compute included. The list scrolls the body to a
    /// section; it never swaps one section for another.
    func test_theSectionListScrollsOneBody() throws {
        XCTAssertEqual(SettingsModal.sections, SettingsSection.allCases)
        XCTAssertEqual(SettingsModal.sections.last, .compute)
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertEqual(modal.components(separatedBy: "ScrollViewReader").count - 1, 1)
        XCTAssertTrue(modal.contains(".frame(width: GlassTokens.Size.modalNavWidth"))
        XCTAssertTrue(modal.contains("proxy.scrollTo(item, anchor: .top)"), "the list does not scroll the body")
        XCTAssertTrue(modal.contains(".id(item)"))
        XCTAssertTrue(modal.contains("ComputeView(model: compute)"))
        XCTAssertTrue(modal.contains("GlassSettingsContent(navigation: navigation, section: item)"))
        XCTAssertFalse(modal.contains(".id(section)"), "a section replaces the body instead of scrolling to it")
        XCTAssertFalse(modal.contains("List(selection:"), "the list selects a view instead of scrolling")
    }

    /// #1146's header and list (P26, P27): the title 15/700 at the modal's
    /// header insets over the 0.5pt modal rule; the list's rows 12pt
    /// secondary text with a faint hover, never the blue menu selection,
    /// beside a 0.5pt rule; and each listed section opens with its name as
    /// a section rule in the body.
    func test_theHeaderAndListAreRons() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".glassType(GlassTokens.TypeScale.title.weight(.bold))"), "the title is not 15/700")
        XCTAssertTrue(modal.contains(".padding(.top, GlassTokens.Space.s7)"))
        XCTAssertTrue(modal.contains(".padding(.trailing, GlassTokens.Space.s8)"))
        XCTAssertTrue(modal.contains(".padding(.bottom, GlassTokens.Space.s5)"))
        XCTAssertEqual(SettingsModal.headerLeading, 18)
        XCTAssertTrue(modal.contains("GlassHairline(GlassTokens.Color.rule.color)"))
        XCTAssertTrue(modal.contains("GlassHairline(GlassColor.ink(Self.navRuleInk), axis: .vertical)"))
        XCTAssertFalse(modal.contains("frame(height: 1)"), "a 1pt hairline is left")
        XCTAssertTrue(modal.contains(".buttonStyle(GlassSectionNavRowStyle())"))
        XCTAssertFalse(modal.contains("GlassMenuRowStyle"), "the list still uses the blue menu selection")
        XCTAssertTrue(modal.contains("ForEach(Self.listed)"))
        XCTAssertTrue(modal.contains("item.listRow(words?.settingsNav)"))
        XCTAssertTrue(modal.contains("GlassSectionRule(name)"), "no section rule opens a section")
        XCTAssertEqual(SettingsModal.navRuleInk, 0.1)
        XCTAssertEqual(SettingsModal.bodyInset, 20)
        XCTAssertEqual(GlassSectionNavRowStyle.hoverInk, 0.08)
        XCTAssertEqual(GlassSectionNavRowStyle.radius, 8)
        XCTAssertEqual(GlassTokens.Size.modalNavWidth, 180)
    }

    /// Escape closes the window; the window's own close button does the
    /// rest, so the body draws none, and nothing in the Monitor is disabled
    /// or hidden while Settings is open.
    func test_escapeClosesTheWindow() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".onExitCommand(perform: onClose)"), "Escape does not close it")
        XCTAssertFalse(modal.contains("systemImage: \"xmark\""), "a second close button beside the window's")
        XCTAssertFalse(modal.contains(".accessibilityAddTraits(.isModal)"), "a window is not modal")
        XCTAssertTrue(modal.contains(".focusSection()"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains(".disabled(navigation.settingsRequest != nil)"))
        XCTAssertFalse(window.contains(".accessibilityHidden(navigation.settingsRequest != nil)"))
    }

    /// Opening at a section (the gear opens at the top; the menu, a deep
    /// link or the Private AI pointer at their section) scrolls straight to
    /// it, on opening and on each new request while open.
    func test_openingAtASectionScrollsToIt() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".onAppear { Self.scroll(to: request.section, proxy) }"))
        XCTAssertTrue(modal.contains(".onChange(of: request) { _, new in Self.scroll(to: new.section, proxy) }"))
        XCTAssertTrue(modal.contains("proxy.scrollTo(section, anchor: .top)"))
        // The menu-bar popover's Settings opens the Settings window.
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("navigation.requestSettings()\n                openWindow(id: WindowID.settings)"))
        let navigation = MainWindowNavigation()
        navigation.requestSettings(at: .compute)
        XCTAssertEqual(navigation.settingsRequest?.section, .compute)
    }

    /// The Private AI section draws the standard settings, the tools and
    /// the connection itself (owner, 2026-10-10), so it no longer points
    /// back at the Monitor; the Private AI tab's foot card opens it.
    func test_privateAISectionHoldsTheToolsAndTheConnection() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        let host = try XCTUnwrap(main.components(separatedBy: "struct SettingsWindowView: View {").last)
        XCTAssertFalse(host.contains("onPrivateAI:"), "Settings points back at the Monitor again")

        let content = try Self.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertTrue(content.contains("case .privateAI: PrivateAISection()"))
        let section = try Self.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(section.contains("PrivateAISettingsPanels(store: store)"))
        XCTAssertTrue(section.contains("store.attach(MonitorWindowView.sampleClient() ?? model.daemonData"))
    }
}
