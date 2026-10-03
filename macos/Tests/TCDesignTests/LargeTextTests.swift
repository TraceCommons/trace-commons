import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// Controls hold a minimum height, never a fixed one, so a label that is
/// taller than the design (a larger system text size, a second line) grows
/// its control instead of being clipped. A two-line label stands in for the
/// larger text: SwiftUI's `dynamicTypeSize` does not scale macOS text styles,
/// which follow the system text size setting instead.
@MainActor
final class LargeTextTests: XCTestCase {
    private func height<V: View>(_ view: V) -> CGFloat {
        NSHostingView(rootView: view.fixedSize()).fittingSize.height
    }

    func test_aTallerLabelGrowsItsControl() {
        let kinds: [(String, GlassButtonKind)] = [
            ("primary", .primary), ("secondary", .secondary), ("glass", .glass), ("submit", .submit(done: false)),
        ]
        for (name, kind) in kinds {
            let one = height(Button {} label: { Text("A") }.buttonStyle(GlassButtonStyle(kind)))
            let two = height(Button {} label: { Text("A\nB\nC") }.buttonStyle(GlassButtonStyle(kind)))
            XCTAssertGreaterThan(two, one, "\(name) clipped its label")
        }
        let row = height(GlassListRow(depth: .session, tile: .session, title: "A", sub: "B").frame(width: 360))
        XCTAssertGreaterThanOrEqual(row, GlassTokens.Size.listRow)
    }
}
