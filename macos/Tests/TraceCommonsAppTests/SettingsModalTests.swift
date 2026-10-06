import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// #1241 Task 10 (owner, 2026-10-05): Settings is Ron's #1146 opaque modal
/// over the Monitor (`settings-modal.tsx`, `surfaces.tsx` `Modal`, `.tc-scrim`
/// and `.tc-modal`), not a separate, translucent window.
@MainActor
final class SettingsModalTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Settings opens inside the Monitor window, over all three panes: the
    /// Settings scene is gone, Cmd-comma and the gear ask the Monitor for the
    /// modal, and nothing opens the old window.
    func test_settingsIsAModalInTheMonitorNotAWindow() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertFalse(main.contains("Settings {"), "a separate Settings scene is still declared")
        XCTAssertTrue(main.contains("MonitorWindowView(navigation: navigation)"))
        // Cmd-comma: the app menu's Settings item opens the Monitor and asks
        // it for the modal.
        XCTAssertTrue(main.contains("CommandGroup(replacing: .appSettings)"))
        XCTAssertTrue(main.contains(".keyboardShortcut(\",\", modifiers: .command)"))
        let command = try XCTUnwrap(main.components(separatedBy: "struct OpenSettingsModalButton: View {").last)
        XCTAssertTrue(command.contains("openWindow(id: WindowID.monitor)\n            navigation.requestSettings()"))

        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("onSettings: { navigation.requestSettings() }"), "the gear does not open the modal")
        XCTAssertTrue(window.contains("SettingsModal(\n"))
        // The name #1242 still mounts is kept, and it routes to the modal
        // instead of drawing a window of its own.
        let kept = try XCTUnwrap(window.components(separatedBy: "struct MonitorSettingsWindow: View {").last)
        XCTAssertTrue(kept.contains("navigation.requestSettings()"))
        XCTAssertTrue(kept.contains("openWindow(id: WindowID.monitor)"))
        XCTAssertFalse(kept.contains("NavigationSplitView"), "the old window is still drawn")

        // Nothing in the app opens the system Settings window any more.
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

    /// The modal is one more pane on the opaque base, with the pane edge and
    /// the modal shadow, over the scrim; never a translucent material alone.
    func test_theModalIsOpaque() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains("GlassPane(padding: 0, isContent: true)"), "the modal is not on the opaque pane base")
        XCTAssertEqual(GlassMaterial.current(content: true), .opaque)
        XCTAssertTrue(modal.contains("GlassTokens.Shadow.modal"))
        // Ron's modal scrim and insets, as every GlassModal has them.
        XCTAssertTrue(modal.contains("GlassTokens.Color.modalScrim.color"))
        XCTAssertTrue(modal.contains(".padding(.top, GlassTokens.Space.modalInsetTop)"))
        XCTAssertTrue(modal.contains("GlassTokens.Size.modalWidth"))
        for translucent in ["Material", ".glassSurface(", "glassEffect", "floating: true"] {
            XCTAssertFalse(modal.contains(translucent), translucent)
        }
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains("Views/Monitor/SettingsModal.swift"))
        // The panes behind are dimmed by the scrim and blurred.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(".blur(radius: navigation.settingsRequest == nil ? 0 : GlassTokens.Size.modalScrimBlur)"))
        // The modal sits inside Ron's window-root host, so a section's own
        // modals and confirmations cover the whole window over it.
        let overlay = try XCTUnwrap(window.range(of: "SettingsModal("))
        let host = try XCTUnwrap(window.range(of: ".glassModalHost()"))
        XCTAssertLessThan(overlay.lowerBound, host.lowerBound)
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
        XCTAssertTrue(modal.contains("GlassSettingsContent(navigation: navigation, section: item, onPrivateAI: onPrivateAI)"))
        XCTAssertFalse(modal.contains(".id(section)"), "a section replaces the body instead of scrolling to it")
        XCTAssertFalse(modal.contains("List(selection:"), "the list selects a view instead of scrolling")
    }

    /// Escape, the close button and a click on the scrim close it; while it
    /// is open the panes behind take no focus and no clicks.
    func test_escapeAndTheScrimClose() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".onExitCommand(perform: onClose)"), "Escape does not close it")
        // Nothing in the modal takes focus by default and the panes behind
        // are disabled, so Escape also binds to the close button, as every
        // other dismissal in the app does.
        let close = try XCTUnwrap(modal.range(of: "systemImage: \"xmark\", small: true, action: onClose)"), "no close button")
        let afterClose = modal[close.upperBound...].prefix(400)
        XCTAssertTrue(afterClose.contains(".keyboardShortcut(.cancelAction)"),
                      "Escape is not bound to the close button")
        XCTAssertTrue(modal.contains(".onTapGesture(perform: onClose)"), "the scrim does not close it")
        XCTAssertTrue(modal.contains("systemImage: \"xmark\", small: true, action: onClose)"), "no close button")
        XCTAssertTrue(modal.contains(".focusSection()"))
        XCTAssertTrue(modal.contains(".accessibilityAddTraits(.isModal)"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("onClose: { navigation.settingsRequest = nil }"))
        XCTAssertTrue(window.contains(".disabled(navigation.settingsRequest != nil)"), "focus can leave the modal")
        XCTAssertTrue(window.contains(".accessibilityHidden(navigation.settingsRequest != nil)"))
    }

    /// Opening at a section (the gear opens at the top; the menu, a deep
    /// link or the Private AI pointer at their section) scrolls straight to
    /// it, on opening and on each new request while open.
    func test_openingAtASectionScrollsToIt() throws {
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains(".onAppear { Self.scroll(to: request.section, proxy) }"))
        XCTAssertTrue(modal.contains(".onChange(of: request) { _, new in Self.scroll(to: new.section, proxy) }"))
        XCTAssertTrue(modal.contains("proxy.scrollTo(section, anchor: .top)"))
        // The menu-bar popover's Settings opens the Monitor at the modal.
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("openWindow(id: WindowID.monitor)\n                navigation.requestSettings()"))
        let navigation = MainWindowNavigation()
        navigation.requestSettings(at: .compute)
        XCTAssertEqual(navigation.settingsRequest?.section, .compute)
    }

    /// The Private AI pointer closes the modal and opens the Inference tab,
    /// as Ron's `navigate(routePaths["private-ai"])` does.
    func test_privateAIPointerOpensInference() throws {
        let navigation = MainWindowNavigation()
        navigation.requestSettings(at: .privateAI)
        var tab = MonitorWindowView.Tab.home
        MonitorWindowView.openPrivateAI(tab: &tab, navigation: navigation)
        XCTAssertEqual(tab, .inference)
        XCTAssertNil(navigation.settingsRequest)

        let content = try Self.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertTrue(content.contains("case .privateAI: PrivateAISection(navigation: navigation, onPointer: onPrivateAI)"))
        let section = try Self.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(section.contains("if let onPointer {\n                                onPointer()"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("onPrivateAI: { Self.openPrivateAI(tab: &tab, navigation: navigation) }"))
    }
}
