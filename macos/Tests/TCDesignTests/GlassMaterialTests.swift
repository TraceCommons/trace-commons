import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// R3, D4: glass on every supported macOS for the navigation layer, and
/// the opaque base for the content layer (R14: Reduce Transparency is the
/// system's to apply to native glass, not ours).
final class GlassMaterialTests: XCTestCase {
    /// The map's pane is content: never Liquid Glass.
    func test_aContentPaneGetsTheOpaqueBase() {
        XCTAssertEqual(GlassMaterial.current(content: true), .opaque)
    }

    func test_eachMacOSGetsItsMaterial() {
        let expected: GlassMaterial
        if #available(macOS 26.0, *) {
            expected = .liquidGlass
        } else {
            expected = .vibrancy
        }
        XCTAssertEqual(GlassMaterial.current(), expected)
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

    /// The content-layer base is solid.
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

    /// `.opaque` is no material: the backdrop must not quietly build a
    /// vibrancy view for it.
    @MainActor
    func test_theOpaqueBackdropIsASolidViewNotVibrancy() {
        let view = GlassBackdrop.makeView(.opaque, cornerRadius: 16)
        XCTAssertFalse(view is NSVisualEffectView)
        let fill = view.layer?.backgroundColor.flatMap { NSColor(cgColor: $0)?.usingColorSpace(.sRGB) }
        let opaque = NSColor(GlassTokens.Color.paneOpaque.color).usingColorSpace(.sRGB)
        XCTAssertEqual(fill?.alphaComponent, 1)
        XCTAssertEqual(fill?.redComponent ?? -1, opaque?.redComponent ?? -2, accuracy: 0.002)
        XCTAssertEqual(fill?.blueComponent ?? -1, opaque?.blueComponent ?? -2, accuracy: 0.002)
    }

    private static func descendants(of view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap { descendants(of: $0) }
    }
}
