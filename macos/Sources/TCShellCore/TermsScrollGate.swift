import Foundation

/// Whether the reader has had the end of the terms in view (terms
/// read-through, 2026-11-09): the accept checkbox stays disabled until then.
///
/// Reached once the bottom of the content is within `tolerance` points of
/// the bottom of the viewport, which also covers a document that fits
/// without scrolling (at any text size). Once reached it stays reached:
/// scrolling back up does not lock the checkbox again. Kept apart from the
/// view so it can be tested without one.
public struct TermsScrollGate: Equatable, Sendable {
    /// How close, in points, the content's bottom must come to the
    /// viewport's.
    public static let tolerance: CGFloat = 8

    public private(set) var reachedEnd = false

    public init() {}

    /// One scroll position: the content's full height, the visible height,
    /// and how far the content is scrolled from its top.
    public mutating func observe(contentHeight: CGFloat, viewportHeight: CGFloat, offset: CGFloat) {
        guard !reachedEnd, contentHeight > 0, viewportHeight > 0 else { return }
        if offset + viewportHeight >= contentHeight - Self.tolerance {
            reachedEnd = true
        }
    }

    /// The last block came into view another way (VoiceOver moving through
    /// the content): that counts as the end.
    public mutating func reachedLastBlock() {
        reachedEnd = true
    }
}
