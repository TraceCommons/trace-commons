import XCTest
@testable import TraceCommonsApp

/// The Private AI tab's guide card takes Home's 3-steps card's compact form
/// and its X (owner, 2026-11-07): compact when the full card would push
/// what is under it out of view, closed by the person's choice, kept under
/// its own key.
@MainActor
final class PrivateAIGuideCompactTests: XCTestCase {
    typealias Guide = PrivateAIGuideCard

    func test_aNarrowCardIsCompactAtHomesWidth() {
        XCTAssertEqual(Guide.compactWidth, HomeTracesHero.compactWidth)
        XCTAssertTrue(Guide.isCompact(width: Guide.compactWidth - 1, visibleHeight: 2000, fullHeight: 400))
        XCTAssertFalse(Guide.isCompact(width: Guide.compactWidth, visibleHeight: 2000, fullHeight: 400))
    }

    func test_aShortTabIsCompactWhenWhatIsUnderItWouldNotShow() {
        let fits = 400 + Guide.contentRoom
        XCTAssertFalse(Guide.isCompact(width: 900, visibleHeight: fits, fullHeight: 400))
        XCTAssertTrue(Guide.isCompact(width: 900, visibleHeight: fits - 1, fullHeight: 400))
    }

    func test_beforeAnythingIsMeasuredTheCardIsFull() {
        XCTAssertFalse(Guide.isCompact(width: 0, visibleHeight: 0, fullHeight: 0))
        XCTAssertFalse(Guide.isCompact(width: 900, visibleHeight: 300, fullHeight: 0), "no full height yet")
    }

    /// The full height is measured from a hidden full copy at every width,
    /// so a card that started compact still learns whether the full card
    /// would fit, and goes back to full when it does.
    func test_theFullHeightIsMeasuredEvenWhileCompact() throws {
        let card = try Self.guideSource()
        XCTAssertTrue(card.contains(".hidden()"))
        XCTAssertTrue(card.contains(".onGeometryChange(for: CGFloat.self, of: \\.size.height) { fullHeight = $0 }"))
        XCTAssertFalse(card.contains("if !compact { fullHeight"), "measured only while full can stay compact")
    }

    func test_closingIsKeptUnderItsOwnKey() throws {
        XCTAssertEqual(Guide.closedKey, "privateAI.guideCardClosed")
        XCTAssertNotEqual(Guide.closedKey, HomeTracesHero.closedKey)
        let card = try Self.guideSource()
        XCTAssertTrue(card.contains("@AppStorage(Self.closedKey) private var closed = false"))
        XCTAssertTrue(card.contains("if !closed {"))
        XCTAssertTrue(card.contains("close: .init(label: MonitorWords.close, action: { closed = true })"))
        XCTAssertTrue(card.contains("if let close { GetStartedCardClose(close: close) }"), "the light card's X")
    }

    /// Compact rows still give VoiceOver each step's line.
    func test_aCompactRowReadsItsLine() throws {
        let card = try Self.guideSource()
        XCTAssertTrue(card.contains(".accessibilityLabel(\"\\(title), \\(line)\")"))
    }

    static func guideSource() throws -> String {
        let views = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor/InferenceViews.swift"),
            encoding: .utf8)
        let start = try XCTUnwrap(views.range(of: "struct PrivateAIGuideCard: View {"))
        let end = try XCTUnwrap(views.range(of: "struct PrivateAIInspectorView: View {"))
        return String(views[start.lowerBound..<end.lowerBound])
    }
}
