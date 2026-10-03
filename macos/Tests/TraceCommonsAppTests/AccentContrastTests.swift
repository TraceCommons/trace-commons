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

    /// The accent is also a fill drawn straight on the ground: an on-switch
    /// track, the checked read-gate box, a prominent button's edge. As a UI
    /// component it must clear 3:1 against the grounds it sits on, in both
    /// schemes. The brand purple #6D14F3 is 2.25:1 on the dark ground.
    func test_theAccentClearsTheFillFloorOnTheGround() {
        for scheme in [NSAppearance.Name.aqua, .darkAqua] {
            for (name, ground) in [("ground", TC.ground), ("surface", TC.surface)] {
                let ratio = Self.contrast(TC.accent, on: ground, in: scheme)
                XCTAssertGreaterThanOrEqual(ratio, 3, "\(scheme.rawValue) on \(name): \(ratio)")
            }
        }
    }

    /// Text drawn in the window tint (a `Link`, a `.borderless` button)
    /// would take the accent; the accent is not a text colour, which is why
    /// those sites take `accentText`. This pins that the accent alone does
    /// not clear text contrast in dark, so the override stays necessary.
    func test_theAccentIsNotATextColourInDark() {
        let ratio = Self.contrast(TC.accent, on: TC.ground, in: .darkAqua)
        XCTAssertLessThan(ratio, 4.5, "dark accent on ground: \(ratio)")
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
