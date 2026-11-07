import SwiftUI

/// A pixelated, dithered grid of light that grows from a card's bottom
/// trailing corner toward its top leading one (owner, 2026-11-02: tried on
/// the Private AI tab's getting-started card), or, as `.waves`, swells along
/// its bottom edge (Ron, 2026-11-05: Home's card, the same grid told apart).
///
/// Square cells on a fixed pitch with a one-point gap, so it reads as a
/// grid. Which cells are lit is an ordered dither (a 4x4 Bayer matrix) of a
/// falloff that is strongest in the corner and gone before it reaches the
/// far side, so it stays clear of a card's text. A slow ripple travels
/// outward through it, and each cell drifts between the palette's nearby
/// blues. It sits behind everything: it never takes the pointer and
/// VoiceOver never reads it.
///
/// It moves at `framesPerSecond`, on the shared timeline that stops while
/// its window cannot be seen (`GlassAnimationTimeline`), and stands still
/// under Reduce Motion. Every frame is a function of the time alone.
public struct GlassPixelField: View {
    /// The shape the light takes on the grid.
    public enum Pattern: Sendable {
        /// Strongest in the bottom trailing corner, a ripple running out of it.
        case corner
        /// Swells rolling along the bottom edge, gone toward the leading side.
        case waves
    }

    private let palette: [GlassRGBA]
    private let pattern: Pattern
    private let maxOpacity: Double
    private let cell: CGFloat
    private let reach: CGSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `palette` is the colours a cell drifts between; `maxOpacity` its
    /// brightest cell; `cell` the grid's pitch; `reach` how far across and
    /// up the card the field reaches, as a fraction of its width and height.
    public init(
        palette: [GlassRGBA], pattern: Pattern = .corner, maxOpacity: Double = 0.34, cell: CGFloat = 6,
        reach: CGSize = CGSize(width: 0.62, height: 1.05)
    ) {
        self.palette = palette
        self.pattern = pattern
        self.maxOpacity = maxOpacity
        self.cell = cell
        self.reach = reach
    }

    /// A pixel field reads as stepped anyway, and each frame is a SwiftUI
    /// graph update, so it runs slow.
    public static let framesPerSecond: Double = 10

    public var body: some View {
        GlassAnimationTimeline(minimumInterval: 1 / Self.framesPerSecond, paused: reduceMotion) { date in
            Canvas { context, size in
                Self.draw(
                    in: &context, size: size,
                    time: reduceMotion ? 0 : date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 3600),
                    palette: palette, pattern: pattern, maxOpacity: maxOpacity, cell: cell, reach: reach)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    // MARK: The pure rules

    /// The ordered dither's thresholds, 0 to 15 (Bayer, 4x4).
    static let bayer: [[Int]] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]

    /// A cell's threshold in (0, 1): lit when its level is above it.
    static func threshold(column: Int, row: Int) -> Double {
        (Double(bayer[row & 3][column & 3]) + 0.5) / 16
    }

    /// How far a point is from the bottom trailing corner, in units of the
    /// field's reach: 0 in the corner, 1 at the field's edge.
    static func distance(x: CGFloat, y: CGFloat, size: CGSize, reach: CGSize) -> Double {
        guard size.width > 0, size.height > 0, reach.width > 0, reach.height > 0 else { return .infinity }
        let u = Double((size.width - x) / (size.width * reach.width))
        let v = Double((size.height - y) / (size.height * reach.height))
        return (u * u + v * v).squareRoot()
    }

    /// The light at `distance` and `time`, 0 to 1: a smooth falloff from
    /// the corner, with a ripple that travels outward as time goes on.
    static func level(distance: Double, time: Double) -> Double {
        guard distance < 1 else { return 0 }
        let falloff = 1 - distance
        let smooth = falloff * falloff * (3 - 2 * falloff)
        let ripple = 0.5 + 0.5 * sin(distance * 14 - time * 1.6)
        return smooth * (0.72 + 0.28 * ripple)
    }

    /// The light at a point of the `.waves` pattern, 0 to 1: nothing above
    /// two rolling swells, rising with depth below them, and faded out toward
    /// the leading side so it stays clear of a card's text.
    static func waveLevel(x: CGFloat, y: CGFloat, size: CGSize, time: Double) -> Double {
        guard size.width > 0, size.height > 0 else { return 0 }
        let u = Double(x / size.width)
        let v = Double(y / size.height)
        let across = min(max((u - 0.34) / 0.5, 0), 1)
        let fade = across * across * (3 - 2 * across)
        let surface = 0.6 + 0.13 * sin(u * 7 - time * 0.9) + 0.06 * sin(u * 15 + time * 0.55)
        let depth = v - surface
        guard depth > 0 else { return 0 }
        return fade * (0.3 + 0.7 * min(depth / 0.3, 1))
    }

    /// Which of `count` colours a cell takes at `time`: a slow drift across
    /// the grid, so neighbouring cells share a colour and the colours move.
    static func shade(column: Int, row: Int, time: Double, count: Int) -> Int {
        guard count > 1 else { return 0 }
        let wave = sin(Double(column) * 0.31 + Double(row) * 0.23 + time * 0.45)
            + sin(Double(column - row) * 0.17 - time * 0.3)
        let unit = min(max((wave + 2) / 4, 0), 0.999_999)
        return Int(unit * Double(count))
    }

    /// The field, drawn as one path per colour and brightness, so a frame
    /// is a handful of fills however many cells are lit.
    static func draw(
        in context: inout GraphicsContext, size: CGSize, time: Double, palette: [GlassRGBA],
        pattern: Pattern = .corner, maxOpacity: Double, cell: CGFloat, reach: CGSize
    ) {
        guard !palette.isEmpty, cell >= 2, size.width > 0, size.height > 0 else { return }
        let columns = Int((size.width / cell).rounded(.up))
        let rows = Int((size.height / cell).rounded(.up))
        // Two brightnesses per colour: the field's inner and outer halves.
        var paths = Array(repeating: Path(), count: palette.count * 2)
        let side = cell - 1
        for row in 0..<rows {
            // The grid is anchored in the bottom trailing corner, so the
            // corner's cells are whole and the far side's are cut.
            let y = size.height - CGFloat(rows - row) * cell
            for column in 0..<columns {
                let x = size.width - CGFloat(columns - column) * cell
                let level: Double
                switch pattern {
                case .corner:
                    let distance = Self.distance(x: x + cell / 2, y: y + cell / 2, size: size, reach: reach)
                    level = Self.level(distance: distance, time: time)
                case .waves:
                    level = Self.waveLevel(x: x + cell / 2, y: y + cell / 2, size: size, time: time)
                }
                guard level > Self.threshold(column: column, row: row) else { continue }
                let colour = Self.shade(column: column, row: row, time: time, count: palette.count)
                let bright = level > 0.5 ? 1 : 0
                paths[colour * 2 + bright].addRect(CGRect(x: x, y: y, width: side, height: side))
            }
        }
        for (index, path) in paths.enumerated() where !path.isEmpty {
            let opacity = maxOpacity * (index % 2 == 1 ? 1 : 0.55)
            context.fill(path, with: .color(palette[index / 2].color.opacity(opacity)))
        }
    }
}
