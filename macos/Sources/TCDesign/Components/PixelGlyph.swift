import AppKit
import SwiftUI

/// A symbol or image drawn as lit cells on a visible grid, the cards' pixel
/// field's language (`GlassPixelField`; Ron, 2026-11-05, after future-retro's
/// "show the grid"): every cell the same size on a whole-point pitch, an
/// ordered Bayer dither, no shadow, and the field's shading: neighbouring
/// cells share one of the palette's colours, brighter toward the glyph's
/// bottom trailing corner.
///
/// A cell is lit where the source covers it, thinning out by the same dither
/// toward its top leading corner, and it moves as the field does: a slow
/// ripple and drifting colours (`GlassPixelField.framesPerSecond`, on the
/// timeline that stops while its window cannot be seen; still under Reduce
/// Motion). A symbol in `drawnCells` is placed by hand, so its edges are
/// square; any other is sampled. VoiceOver never reads it.
public struct GlassPixelGlyph: View {
    /// What the glyph samples.
    public enum Source {
        case symbol(String, weight: NSFont.Weight)
        case image(NSImage)
    }

    private let source: Source
    private let size: CGFloat
    private let palette: [GlassRGBA]
    private let dimmed: [GlassRGBA]
    private let cell: CGFloat
    private let angle: Double
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `palette` holds the brighter cells' colours and `dimmed` the others',
    /// one for one, all opaque. `size` is the source's own point size. The canvas runs a little past
    /// it, so a tilted glyph keeps its corners, without changing the layout.
    public init(
        _ source: Source, size: CGFloat, palette: [GlassRGBA], dimmed: [GlassRGBA], cell: CGFloat,
        angle: Double = 0
    ) {
        self.source = source
        self.size = size
        self.palette = palette
        self.dimmed = dimmed
        self.cell = cell
        self.angle = angle
    }

    /// How far the canvas runs past `size`, as a fraction of it.
    static let overscan: CGFloat = 0.3

    public var body: some View {
        let side = (size * (1 + Self.overscan) / cell).rounded(.up) * cell
        let mask = Self.mask(source, size: size, canvas: side, cell: cell, angle: angle)
        let palette = palette
        let dimmed = dimmed
        let cell = cell
        let still = reduceMotion
        GlassAnimationTimeline(minimumInterval: 1 / GlassPixelField.framesPerSecond, paused: still) { date in
            Canvas { context, _ in
                Self.draw(
                    mask, in: &context, palette: palette, dimmed: dimmed, cell: cell,
                    time: still ? 0 : date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 3600))
            }
        }
        .frame(width: side, height: side)
        .padding(-(side - size) / 2)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    // MARK: The pure rules

    /// How much of each cell the source covers, 0 to 1, row by row from the
    /// top.
    struct Mask {
        let columns: Int
        let rows: Int
        let coverage: [Float]
    }

    /// How much of a cell the source must cover for the cell to light.
    static let minimumCoverage: Float = 0.5

    /// How much of the glyph shows at a cell, 0 to 1: all of it toward the
    /// bottom trailing corner, thinning toward the top leading one. `across`
    /// and `down` run 0 to 1 over the canvas.
    static func presence(across: Double, down: Double) -> Double {
        let far = 1 - (across + down) / 2
        let into = min(max((far - fadeStart) / (1 - fadeStart), 0), 1)
        return 1 - fadeDepth * into * into * (3 - 2 * into)
    }

    /// Where along the diagonal the fade starts, and how much it takes away.
    static let fadeStart: Double = 0.55
    static let fadeDepth: Double = 0.92

    /// Whether a cell lights: covered, and present past its dither threshold.
    static func isLit(coverage: Float, column: Int, row: Int, columns: Int, rows: Int) -> Bool {
        guard coverage >= minimumCoverage, columns > 1, rows > 1 else { return false }
        let across = Double(column) / Double(columns - 1)
        let down = Double(row) / Double(rows - 1)
        return presence(across: across, down: down) > GlassPixelField.threshold(column: column, row: row)
    }

    /// A run of cells on a drawn symbol's grid.
    struct CellRun {
        let x: Int, y: Int, width: Int, height: Int

        init(_ x: Int, _ y: Int, _ width: Int, _ height: Int) {
            self.x = x
            self.y = y
            self.width = width
            self.height = height
        }
    }

    /// The symbols promotions use most, by hand: the grid's side, how many
    /// cells it sits right of centre to clear a card's words, then each layer
    /// in turn, lighting or cutting back out its runs.
    static let drawnCells: [String: (grid: Int, shift: Int, layers: [(lit: Bool, runs: [CellRun])])] = [
        "cpu": (grid: 38, shift: 2, layers: [
            // The pins, three a side, and the body with its corners off.
            (true, [
                CellRun(11, 0, 2, 7), CellRun(18, 0, 2, 7), CellRun(25, 0, 2, 7),
                CellRun(11, 31, 2, 7), CellRun(18, 31, 2, 7), CellRun(25, 31, 2, 7),
                CellRun(0, 11, 7, 2), CellRun(0, 18, 7, 2), CellRun(0, 25, 7, 2),
                CellRun(31, 11, 7, 2), CellRun(31, 18, 7, 2), CellRun(31, 25, 7, 2),
                CellRun(6, 6, 26, 26),
            ]),
            (false, [CellRun(6, 6, 1, 1), CellRun(31, 6, 1, 1), CellRun(6, 31, 1, 1), CellRun(31, 31, 1, 1),
                     CellRun(9, 9, 20, 20)]),
            // The die, a ring inside the body.
            (true, [CellRun(13, 13, 12, 12)]),
            (false, [CellRun(15, 15, 8, 8)]),
        ]),
        "chart.bar.xaxis": (grid: 32, shift: 0, layers: [
            (true, [CellRun(-2, 12, 7, 20), CellRun(8, 0, 7, 32), CellRun(18, 7, 7, 25)]),
        ]),
    ]

    /// A drawn symbol centred on a canvas of `columns` cells square.
    static func drawnMask(_ symbol: String, columns: Int) -> Mask? {
        guard let drawn = drawnCells[symbol] else { return nil }
        let inset = (columns - drawn.grid) / 2
        var coverage = [Float](repeating: 0, count: columns * columns)
        for layer in drawn.layers {
            for run in layer.runs {
                for row in run.y..<(run.y + run.height) {
                    for column in run.x..<(run.x + run.width) {
                        let y = row + inset, x = column + inset + drawn.shift
                        guard (0..<columns).contains(y), (0..<columns).contains(x) else { continue }
                        coverage[y * columns + x] = layer.lit ? 1 : 0
                    }
                }
            }
        }
        return Mask(columns: columns, rows: columns, coverage: coverage)
    }

    /// One path per colour and brightness, so a glyph is a handful of fills.
    static func draw(
        _ mask: Mask, in context: inout GraphicsContext, palette: [GlassRGBA], dimmed: [GlassRGBA], cell: CGFloat,
        time: Double
    ) {
        guard !palette.isEmpty, dimmed.count == palette.count, mask.columns > 0 else { return }
        var paths = Array(repeating: Path(), count: palette.count * 2)
        let extent = CGFloat(mask.columns) * cell
        let area = CGSize(width: extent, height: extent)
        // A one-point gap, as the pixel field's.
        let side = max(cell - 1, 1)
        for row in 0..<mask.rows {
            for column in 0..<mask.columns {
                let coverage = mask.coverage[row * mask.columns + column]
                guard isLit(coverage: coverage, column: column, row: row, columns: mask.columns, rows: mask.rows)
                else { continue }
                let shade = GlassPixelField.shade(column: column, row: row, time: time, count: palette.count)
                let x = CGFloat(column) * cell, y = CGFloat(row) * cell
                let distance = GlassPixelField.distance(
                    x: x + cell / 2, y: y + cell / 2, size: area, reach: CGSize(width: 1, height: 1))
                let bright = GlassPixelField.level(distance: distance, time: time) > 0.28 ? 1 : 0
                paths[shade * 2 + bright].addRect(CGRect(x: x, y: y, width: side, height: side))
            }
        }
        for (index, path) in paths.enumerated() where !path.isEmpty {
            context.fill(path, with: .color((index % 2 == 1 ? palette : dimmed)[index / 2].color))
        }
    }

    // MARK: Sampling

    /// Samples per cell along each side.
    private static let samples = 4
    @MainActor private static var cache: [String: Mask] = [:]

    @MainActor
    private static func mask(_ source: Source, size: CGFloat, canvas: CGFloat, cell: CGFloat, angle: Double) -> Mask {
        if case .symbol(let symbol, _) = source,
           let drawn = drawnMask(symbol, columns: Int((canvas / cell).rounded())) {
            return drawn
        }
        let name: String
        switch source {
        case .symbol(let symbol, let weight): name = "\(symbol)/\(weight.rawValue)"
        case .image(let image): name = "image/\(ObjectIdentifier(image).hashValue)"
        }
        let key = "\(name)/\(size)/\(canvas)/\(cell)/\(angle)"
        if let cached = cache[key] { return cached }
        let mask = sample(source, size: size, canvas: canvas, cell: cell, angle: angle)
        cache[key] = mask
        return mask
    }

    /// Draws the source, tilted, into a bitmap of `samples` per cell, then
    /// averages its alpha over each cell.
    @MainActor
    private static func sample(_ source: Source, size: CGFloat, canvas: CGFloat, cell: CGFloat, angle: Double) -> Mask {
        let columns = Int((canvas / cell).rounded())
        let empty = Mask(columns: columns, rows: columns, coverage: Array(repeating: 0, count: columns * columns))
        let pixels = columns * samples
        guard columns > 0, let bitmap = CGContext(
            data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: pixels * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return empty }
        let scale = CGFloat(pixels) / canvas
        let image: NSImage?
        switch source {
        case .symbol(let symbol, let weight):
            image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
                .withSymbolConfiguration(.init(pointSize: size * scale, weight: weight))
        case .image(let given):
            image = given
        }
        guard let image, image.size.width > 0, image.size.height > 0 else { return empty }
        // A symbol keeps its own proportions at its point size; an image
        // fits inside the box.
        let fit: CGFloat
        if case .image = source {
            fit = size * scale / max(image.size.width, image.size.height)
        } else {
            fit = 1
        }
        let drawn = CGSize(width: image.size.width * fit, height: image.size.height * fit)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: bitmap, flipped: false)
        bitmap.translateBy(x: CGFloat(pixels) / 2, y: CGFloat(pixels) / 2)
        // SwiftUI's angles turn clockwise on screen, Core Graphics' the other way.
        bitmap.rotate(by: -CGFloat(angle) * .pi / 180)
        image.draw(
            in: CGRect(x: -drawn.width / 2, y: -drawn.height / 2, width: drawn.width, height: drawn.height),
            from: .zero, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        guard let data = bitmap.data else { return empty }
        let bytes = data.bindMemory(to: UInt8.self, capacity: pixels * pixels * 4)
        var coverage = [Float](repeating: 0, count: columns * columns)
        for row in 0..<columns {
            for column in 0..<columns {
                var sum = 0
                for y in 0..<samples {
                    for x in 0..<samples {
                        sum += Int(bytes[((row * samples + y) * pixels + column * samples + x) * 4 + 3])
                    }
                }
                coverage[row * columns + column] = Float(sum) / Float(samples * samples * 255)
            }
        }
        return Mask(columns: columns, rows: columns, coverage: coverage)
    }
}
