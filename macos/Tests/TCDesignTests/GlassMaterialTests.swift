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

    /// Every pane keeps #1146's pane gradient, veil and edge over its
    /// native material, Liquid Glass included (owner ruling, 2026-10-07).
    func test_everyPaneKeepsTheReferenceGradientVeilAndEdge() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign/Styling")
        let backdrop = try String(contentsOf: url.appendingPathComponent("GlassBackdrop.swift"), encoding: .utf8)
        let native = try XCTUnwrap(backdrop.range(of: "case .liquidGlass, .vibrancy:"))
        let tail = backdrop[native.upperBound...].prefix(400)
        XCTAssertTrue(tail.contains("glassVeil") && tail.contains("paneFill"))
        let style = try String(contentsOf: url.appendingPathComponent("GlassStyle.swift"), encoding: .utf8)
        XCTAssertTrue(style.contains(".glassEdge(edge ?? tier.edge, in: shape)"))
        XCTAssertFalse(style.contains("drawsOwnEdge"))
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

    /// The opaque backdrop follows the appearance: the light pane base
    /// under the light appearance and the dark one under the dark, never
    /// one baked-in value (#1206 review).
    @MainActor
    func test_theOpaqueBackdropFollowsTheAppearance() throws {
        let view = GlassBackdrop.makeView(.opaque, cornerRadius: 16)
        let base = GlassTokens.Color.paneOpaque
        for (name, expected) in [(NSAppearance.Name.aqua, base.light), (.darkAqua, base.dark)] {
            view.appearance = try XCTUnwrap(NSAppearance(named: name))
            let fill = view.layer?.backgroundColor.flatMap { NSColor(cgColor: $0)?.usingColorSpace(.sRGB) }
            XCTAssertEqual(fill?.redComponent ?? -1, expected.red, accuracy: 0.002, "\(name)")
            XCTAssertEqual(fill?.blueComponent ?? -1, expected.blue, accuracy: 0.002, "\(name)")
        }
    }

    private static func descendants(of view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap { descendants(of: $0) }
    }
}
