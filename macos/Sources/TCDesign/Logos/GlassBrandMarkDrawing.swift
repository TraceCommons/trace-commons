import CoreGraphics
import SwiftUI

extension GlassBrandMark {
    /// The marker's centre line through the mark, in the tile's coordinates
    /// (Ron's 2026-10-21 canvas): from inside the lower-left hook, down and
    /// up the left stem, along the diagonal, up the right stem and into the
    /// upper-right hook. Starting a little inside the lower hook keeps the
    /// pen's round end off the diagonal beside it.
    static let markerLine: [CGPoint] = [
        CGPoint(x: 167.5, y: 234.6), CGPoint(x: 101, y: 312), CGPoint(x: 101, y: 86),
        CGPoint(x: 299, y: 320), CGPoint(x: 299, y: 84), CGPoint(x: 226, y: 170),
    ]

    /// The marker's width in the tile's coordinates: wide enough to cover
    /// every stroke of the mark.
    static let markerWidth: CGFloat = 44

    /// The marker line's length in the tile's coordinates.
    static let markerLength: CGFloat = zip(markerLine, markerLine.dropFirst())
        .reduce(0) { $0 + hypot($1.1.x - $1.0.x, $1.1.y - $1.0.y) }

    /// The point `distance` along the marker line, clamped to its ends.
    static func markerPoint(at distance: CGFloat) -> CGPoint {
        var left = max(0, distance)
        for (from, to) in zip(markerLine, markerLine.dropFirst()) {
            let length = hypot(to.x - from.x, to.y - from.y)
            if left <= length, length > 0 {
                let t = left / length
                return CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
            }
            left -= length
        }
        return markerLine[markerLine.count - 1]
    }

    /// The light band that crosses the mark, in the tile's coordinates: a
    /// 110-wide strip from y 60 to 340, leaning forward 18 degrees, its left
    /// edge at `x`. It rests fully clear of the mark (x 83 to 317) at both
    /// ends of its travel, -240 and 470.
    ///
    /// `from` and `to` run across the band square to its lean, so the light
    /// inside it leans with it, as the canvas's skewed gradient does (Ron,
    /// 2026-10-22: the shimmer read upright).
    static func shineBand(at x: CGFloat) -> (band: Path, from: CGPoint, to: CGPoint) {
        let angle = 18 * CGFloat.pi / 180
        let lean = tan(angle)
        func skewed(_ px: CGFloat, _ py: CGFloat) -> CGPoint { CGPoint(x: px - lean * py, y: py) }
        var band = Path()
        band.move(to: skewed(x, 60))
        band.addLine(to: skewed(x + 110, 60))
        band.addLine(to: skewed(x + 110, 340))
        band.addLine(to: skewed(x, 340))
        band.closeSubpath()
        let from = skewed(x, 200)
        let across = 110 * cos(angle)
        return (band, from, CGPoint(x: from.x + across * cos(angle), y: from.y + across * sin(angle)))
    }

    /// Where the shine's band starts and ends.
    static let shineTravel: ClosedRange<CGFloat> = -240...470
}

/// The NEAR AI mark as the animated first run draws it: filled in `fill`,
/// inked in as far as `drawn` along the marker line (1 is the whole mark),
/// with the pen tip at the stroke's front while it draws, and a light band
/// across it at `shine` (0 to 1 of its travel; nil draws none).
///
/// A drawing only: it carries no accessibility words. The caller hides it
/// or names it with the core's.
public struct GlassBrandMarkDrawing: View {
    private let fill: Color
    private let drawn: Double
    private let shine: Double?
    private let tip: Color?
    private let tipGlow: Color

    public init(fill: Color, drawn: Double = 1, shine: Double? = nil, tip: Color? = nil, tipGlow: Color = .clear) {
        self.fill = fill
        self.drawn = drawn
        self.shine = shine
        self.tip = tip
        self.tipGlow = tipGlow
    }

    public var body: some View {
        Canvas { context, size in
            guard let unit = GlassBrandMark.unit, drawn > 0 else { return }
            let box = GlassBrandMark.viewBox
            let side = min(size.width, size.height * box.width / box.height)
            let scale = side / box.width
            let origin = CGPoint(x: (size.width - side) / 2, y: (size.height - box.height * scale) / 2)
            let place = CGAffineTransform(translationX: origin.x, y: origin.y).scaledBy(x: scale, y: scale)
                .translatedBy(x: -box.minX, y: -box.minY)
            let mark = unit.applying(CGAffineTransform(scaleX: box.width, y: box.width)).applying(
                CGAffineTransform(translationX: origin.x, y: origin.y).scaledBy(x: scale, y: scale))

            var ink = context
            if drawn < 1 {
                var line = Path()
                line.addLines(GlassBrandMark.markerLine)
                let stroked = line.applying(place).trimmedPath(from: 0, to: drawn).strokedPath(
                    StrokeStyle(lineWidth: GlassBrandMark.markerWidth * scale, lineCap: .round, lineJoin: .round))
                ink.clip(to: stroked)
            }
            ink.fill(mark, with: .color(fill))

            if let shine {
                var band = context
                band.clip(to: mark)
                let travel = GlassBrandMark.shineTravel
                let x = travel.lowerBound + (travel.upperBound - travel.lowerBound) * CGFloat(shine)
                let strip = GlassBrandMark.shineBand(at: x)
                band.fill(
                    strip.band.applying(place),
                    with: .linearGradient(
                        Gradient(stops: [
                            .init(color: .white.opacity(0), location: 0),
                            .init(color: .white.opacity(0.7), location: 0.5),
                            .init(color: .white.opacity(0), location: 1),
                        ]),
                        startPoint: strip.from.applying(place), endPoint: strip.to.applying(place)))
            }

            if let tip, drawn < 1 {
                // The tip rides just inside the front of the pen's round end,
                // and fades in and out at the ends of the line.
                let front = CGFloat(drawn) * GlassBrandMark.markerLength + GlassBrandMark.markerWidth / 2 - 8
                let point = GlassBrandMark.markerPoint(at: front).applying(place)
                let radius = 10.5 * scale
                let fade = min(1, drawn / 0.05, (1 - drawn) / 0.08)
                var pen = context
                pen.opacity = max(0, fade)
                pen.addFilter(.shadow(color: tipGlow, radius: radius * 1.6))
                pen.fill(
                    Path(ellipseIn: CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2)),
                    with: .color(tip))
            }
        }
    }
}
