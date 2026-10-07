import XCTest

@testable import TCDesign

/// Every text token clears 4.5:1 on the pane base.
final class TextContrastTests: XCTestCase {
    private let base = GlassRGBA(GlassTokens.Color.paneBase.rgb)

    func test_everyTextTokenClearsTextContrastOnThePane() {
        let text: [(String, GlassRGBA)] = [
            ("textPrimary", GlassTokens.Color.textPrimary),
            ("textSecondary", GlassTokens.Color.textSecondary),
            ("textTertiary", GlassTokens.Color.textTertiary),
            ("purpleText", GlassTokens.Color.purpleText),
        ]
        for (name, token) in text {
            let ratio = SwitchContrastTests.contrast(token, base)
            XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(name): \(ratio)")
        }
    }

    /// An off row's sub-line keeps the tertiary ink and fades with the
    /// row, as #1146 does (owner ruling, 2026-10-07).
    func test_anOffRowsSubLineIsTertiary() {
        XCTAssertEqual(GlassListRow.plainSubInk(off: false), GlassTokens.Color.textTertiary)
        XCTAssertEqual(GlassListRow.plainSubInk(off: true), GlassTokens.Color.textTertiary)
    }
}
