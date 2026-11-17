import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

final class SettingsAccessoryTests: XCTestCase {
    /// Watching has no Refresh control (owner, 2026-11-04): the Trace
    /// discovery card and its button, which only re-read settings and read
    /// as a rescan, are gone.
    @MainActor
    func testWatchingHasNoRefreshButton() async throws {
        _ = NSApplication.shared
        let previous = NSApp.value(forKey: "accessibilityEnhancedUserInterface")
        NSApp.setValue(true, forKey: "accessibilityEnhancedUserInterface")
        defer { NSApp.setValue(previous, forKey: "accessibilityEnhancedUserInterface") }

        let model = AppModel()
        defer { model.shutdown() }
        let bounds = NSRect(x: 0, y: 0, width: 408, height: 500)
        let hosting = NSHostingView(rootView: WatchingSection().environmentObject(model).frame(width: 408))
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        await Task.yield()
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()

        func nodes(_ element: Any) -> [AnyObject] {
            let node = element as AnyObject
            let children = (node.accessibilityChildren?() ?? nil) ?? []
            return [node] + children.flatMap(nodes)
        }
        let refreshButtons = nodes(hosting).filter {
            let label = ($0.accessibilityLabel?() ?? nil) ?? ""
            return ($0.accessibilityRole?() ?? nil) == .button && label.localizedCaseInsensitiveContains("refresh")
        }
        XCTAssertEqual(refreshButtons.count, 0, "Watching must not expose a Refresh button")
    }
}
