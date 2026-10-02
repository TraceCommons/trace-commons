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

    /// The opaque base is the pane base at full opacity: text contrast on it
    /// is what the token's contrast notes were measured against.
    func test_theOpaqueBaseIsThePaneBase() {
        XCTAssertEqual(GlassTokens.Color.paneBase.rgb, 0x161A22)
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
