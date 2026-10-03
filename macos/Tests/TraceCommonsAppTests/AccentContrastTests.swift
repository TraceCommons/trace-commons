import AppKit
import SwiftUI
import XCTest

@testable import TraceCommonsApp

/// The purple accent is a fill and tint colour; the lavender is text only.
/// White glyphs on the accent (a checked box, a prominent button, the
/// consent read-gate) must clear 3:1 in both schemes, and accent text must
/// clear 4.5:1 on the ground.
final class AccentContrastTests: XCTestCase {
    func test_whiteOnTheAccentClearsTheGlyphFloorInBothSchemes() {
        for scheme in [NSAppearance.Name.aqua, .darkAqua] {
            let ratio = Self.contrast(TC.onAccent, on: TC.accent, in: scheme)
            XCTAssertGreaterThanOrEqual(ratio, 3, "\(scheme.rawValue): \(ratio)")
        }
    }

    func test_thePrimaryLabelOnThePrimaryFillClearsTextContrast() {
        for scheme in [NSAppearance.Name.aqua, .darkAqua] {
            let ratio = Self.contrast(TC.primaryLabel, on: TC.primaryFill, in: scheme)
            XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(scheme.rawValue): \(ratio)")
        }
    }

    func test_accentTextClearsTextContrastOnTheGround() {
        for scheme in [NSAppearance.Name.aqua, .darkAqua] {
            let ratio = Self.contrast(TC.accentText, on: TC.ground, in: scheme)
            XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(scheme.rawValue): \(ratio)")
        }
    }

    // MARK: WCAG relative luminance

    static func contrast(_ foreground: Color, on background: Color, in scheme: NSAppearance.Name) -> Double {
        let a = luminance(foreground, in: scheme)
        let b = luminance(background, in: scheme)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    static func luminance(_ color: Color, in scheme: NSAppearance.Name) -> Double {
        var rgb = (0.0, 0.0, 0.0)
        NSAppearance(named: scheme)!.performAsCurrentDrawingAppearance {
            let resolved = NSColor(color).usingColorSpace(.sRGB)!
            rgb = (Double(resolved.redComponent), Double(resolved.greenComponent), Double(resolved.blueComponent))
        }
        func linear(_ c: Double) -> Double { c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        return 0.2126 * linear(rgb.0) + 0.7152 * linear(rgb.1) + 0.0722 * linear(rgb.2)
    }
}
