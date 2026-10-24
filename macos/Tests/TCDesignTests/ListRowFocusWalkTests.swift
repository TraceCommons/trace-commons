import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// A list row carries one `Button` at most: its Submit pill.
///
/// SwiftUI walks the window's whole responder tree for accessibility focus
/// (`AccessibilityNode.updateFocus` -> `ViewRendererHost.focusResponder`)
/// once for every `Button` that appears or whose accessibility changes. A
/// tap gesture, an accessibility element or action, and the button trait
/// cost none. Measured with a breakpoint on `updateFocusResponder()`, 50
/// folder rows cost 200 walks with the chevron and the kebab as buttons and
/// 50 with them as taps; 50 session rows 100 and 50. On the Traces tab at a
/// few hundred folders the walks of the rows drawn made a tab switch freeze.
///
/// The chevron and the kebab are taps (`GlassTapControl`) that still read to
/// VoiceOver as a disclosure triangle and a named button.
@MainActor
final class ListRowFocusWalkTests: XCTestCase {
    func test_aRowsChevronAndMenuAreNotButtons() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let row = try XCTUnwrap(sources["ListRow.swift"])
        let body = try XCTUnwrap(row.components(separatedBy: "public var body: some View {").last)
        XCTAssertEqual(body.components(separatedBy: "Button(").count - 1, 1, "a row holds a Button besides Submit")
        XCTAssertTrue(body.contains("Button(submitTitle)"))
        XCTAssertTrue(body.contains(".glassTapControl(action: onToggleExpand)"))
        XCTAssertTrue(body.contains("GlassKebab(menuLabel, open: menuOpen, control: .tap, action: onMenu)"))

        let controls = try XCTUnwrap(sources["Controls.swift"])
        let kebab = try XCTUnwrap(controls.components(separatedBy: "public struct GlassKebab: View {").last?
            .components(separatedBy: "public struct GlassExpander: View {").first)
        let tap = try XCTUnwrap(kebab.components(separatedBy: "case .tap:").last?.components(separatedBy: "private var dots").first)
        XCTAssertFalse(tap.contains("Button("), "the row's kebab is a Button")
        XCTAssertTrue(tap.contains(".accessibilityAddTraits(.isButton)"))
        XCTAssertTrue(tap.contains(".accessibilityAction"))
    }

    /// What VoiceOver gets is unchanged: the chevron is a disclosure
    /// triangle, the kebab a button named in the core's words. Passes on
    /// the Button version too: it pins that the change kept them.
    func test_theRowsControlsStillReadForVoiceOver() throws {
        final class Presses { var expand = 0, menu = 0 }
        let presses = Presses()
        let row = GlassListRow(
            depth: .folder, tile: .folder, title: "Project", sub: "Ask", expanded: false,
            expandLabel: "Project", menuLabel: "Folder menu",
            onToggleExpand: { presses.expand += 1 }, onSelect: {}, onMenu: { presses.menu += 1 })
            .frame(width: 360)
        _ = NSApplication.shared
        let bounds = NSRect(x: 0, y: 0, width: 360, height: 60)
        let hosting = NSHostingView(rootView: row)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        // SwiftUI builds its accessibility tree once an assistive client
        // asks; this is the flag such a client sets.
        NSApp.setValue(true, forKey: "accessibilityEnhancedUserInterface")
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()

        func nodes(_ element: Any) -> [AnyObject] {
            let node = element as AnyObject
            let children = (node.accessibilityChildren?() ?? nil) ?? []
            return [node] + children.flatMap(nodes)
        }
        let all = nodes(hosting)
        let disclosure = try XCTUnwrap(all.first { ($0.accessibilityRole?() ?? nil) == .disclosureTriangle },
                                       "the chevron is not a disclosure triangle")
        let menu = try XCTUnwrap(all.first {
            ($0.accessibilityRole?() ?? nil) == .button && (($0.accessibilityLabel?() ?? nil) ?? "") == "Folder menu"
        }, "the kebab is not a button named Folder menu")
        // A press through this offscreen window reaches neither control,
        // Button or tap alike, so only the roles and names are pinned here.
        _ = (disclosure, menu, presses)
    }
}
