import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

final class SettingsAccessoryTests: XCTestCase {
    /// Discovery's refresh is its own named control beside the heading.
    /// An unlabeled accessory closure used to bind to the card's optional
    /// action instead, discarding the refresh view and making the card a button.
    @MainActor
    func testWatchingExposesTheRefreshAccessoryAsAButton() async throws {
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
            ($0.accessibilityRole?() ?? nil) == .button
                && ($0.accessibilityLabel?() ?? nil) == SettingsLegacyWords.refresh
        }
        XCTAssertEqual(refreshButtons.count, 1, "Discovery must expose its Refresh button beside the heading")
    }
}
