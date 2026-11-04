import SwiftUI
import XCTest

@testable import TCDesign

/// A hero's title runs the card's width only when asked to, and then stops
/// short of the close button's whole target, so a click on the title never
/// closes the card (owner, 2026-11-02: Home's Traces hero, condensed).
@MainActor
final class HeroCardTitleTests: XCTestCase {
    typealias Card = GlassHeroCard<EmptyView, EmptyView>

    func test_theTitleKeepsClearOfTheGraphicUnlessToldToSpanIt() {
        XCTAssertEqual(Card.titleTrailing(contentSpansGraphic: false, titleSpansGraphic: false, closes: true), 0,
                       "the whole of the words already stops short of the graphic")
        XCTAssertEqual(Card.titleTrailing(contentSpansGraphic: true, titleSpansGraphic: false, closes: true),
                       Card.graphicReserve)
        XCTAssertEqual(Card.titleTrailing(contentSpansGraphic: true, titleSpansGraphic: true, closes: false), 0)
        XCTAssertEqual(Card.titleTrailing(contentSpansGraphic: true, titleSpansGraphic: true, closes: true),
                       Card.closeReserve)
    }

    /// The X's target is its glyph with its inset all round, in the card's
    /// top trailing corner; a full-width title ends the card's padding plus
    /// the reserve in from that edge, clear of it.
    func test_aFullWidthTitleClearsTheCloseButtonsTarget() {
        let target = GlassHeroClose.glyph + GlassHeroClose.inset * 2
        XCTAssertGreaterThanOrEqual(GlassTokens.Space.cardPaddingHorizontal + Card.closeReserve, target)
        XCTAssertLessThan(Card.closeReserve, Card.graphicReserve, "far less room than the graphic took")
    }
}
