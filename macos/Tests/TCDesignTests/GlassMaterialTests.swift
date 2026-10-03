import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// R3, D4: glass on every supported macOS, and none under Reduce
/// Transparency.
final class GlassMaterialTests: XCTestCase {
    func test_reduceTransparencyAlwaysGetsTheOpaqueBase() {
        XCTAssertEqual(GlassMaterial.current(reduceTransparency: true), .opaque)
    }

    func test_eachMacOSGetsItsMaterial() {
        let expected: GlassMaterial
        if #available(macOS 26.0, *) {
            expected = .liquidGlass
        } else {
            expected = .vibrancy
        }
        XCTAssertEqual(GlassMaterial.current(reduceTransparency: false), expected)
    }

    /// Liquid Glass draws its own rim; a pane on it must not draw a second.
    /// Every other tier, and every pane on the fallbacks, keeps its edge.
    func test_onlyAPaneOnLiquidGlassSkipsItsOwnEdge() {
        XCTAssertFalse(GlassTier.pane.drawsOwnEdge(on: .liquidGlass))
        XCTAssertTrue(GlassTier.pane.drawsOwnEdge(on: .vibrancy))
        XCTAssertTrue(GlassTier.pane.drawsOwnEdge(on: .opaque))
        for tier in [GlassTier.card, .cardQuiet, .well, .control, .controlSelected, .popover, .menu, .nodeCard] {
            XCTAssertTrue(tier.drawsOwnEdge(on: .liquidGlass), "\(tier)")
        }
    }

    /// The Reduce Transparency base is solid.
    func test_theOpaqueBaseIsSolid() {
        XCTAssertEqual(GlassTokens.Color.paneOpaque.alpha, 1)
    }

    @MainActor
    func test_theBackdropBuildsTheNativeViewForTheMaterial() {
        let glass = GlassBackdrop(material: .liquidGlass, cornerRadius: 16)
        let vibrancy = GlassBackdrop(material: .vibrancy, cornerRadius: 16)
        let host = NSHostingView(rootView: HStack { glass; vibrancy }.frame(width: 200, height: 100))
        host.layoutSubtreeIfNeeded()
        let classes = Self.descendants(of: host).map { String(describing: type(of: $0)) }
        XCTAssertTrue(classes.contains("NSVisualEffectView"), "\(classes)")
        if #available(macOS 26.0, *) {
            XCTAssertTrue(classes.contains("NSGlassEffectView"), "\(classes)")
        }
    }

    private static func descendants(of view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap { descendants(of: $0) }
    }
}
