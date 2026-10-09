import SwiftUI
import XCTest

@testable import TCDesign

/// The info button (owner, 2026-10-08): hover shows its text, a click pins
/// it until an outside click, VoiceOver reads the caller's label and the
/// text, it is a button (a keyboard stop with a pressed state), and it adds
/// no animation of its own, so Reduce Motion is the system popover's.
@MainActor
final class InfoButtonTests: XCTestCase {
    private func source() throws -> String {
        try XCTUnwrap(try DesignSources.components().first { $0.0 == "InfoButton.swift" }?.1)
    }

    func test_hoverOrAPinShowsTheText() {
        XCTAssertFalse(GlassInfoButton.isShown(hovering: false, pinned: false))
        XCTAssertTrue(GlassInfoButton.isShown(hovering: true, pinned: false))
        XCTAssertTrue(GlassInfoButton.isShown(hovering: false, pinned: true))
        XCTAssertTrue(GlassInfoButton.isShown(hovering: true, pinned: true))
    }

    func test_itIsAnAccessibleButtonWithNoAnimationOfItsOwn() throws {
        let text = try source()
        XCTAssertTrue(text.contains("Button {\n            pinned.toggle()"))
        XCTAssertTrue(text.contains(".buttonStyle(GlassPressStyle())"))
        XCTAssertTrue(text.contains(".onHover { hovering = $0 }"))
        XCTAssertTrue(text.contains(".popover(isPresented: shown"))
        XCTAssertTrue(text.contains(".accessibilityLabel(label)"))
        XCTAssertTrue(text.contains(".accessibilityValue(text)"))
        // An outside click or Escape closes the popover and unpins it.
        XCTAssertTrue(text.contains("pinned = false"))
        XCTAssertFalse(text.contains(".animation("))
        XCTAssertFalse(text.contains("withAnimation"))
        XCTAssertFalse(text.contains(".help("), "the popover is the text; no second tooltip")
    }
}
