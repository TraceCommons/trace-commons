import SwiftUI

/// #1146's own small glyphs, drawn from its SVG path data at its sizes and
/// stroke widths (owner ruling, 2026-10-07: #1146 wins for components):
/// the toolbar's view, graph, map and inspector icons, the Settings gear,
/// the picker chevron, the folder button's folder and the kebab's dots.
/// Decorative: the control around a glyph carries its name.
public enum GlassGlyph: String, CaseIterable, Sendable {
    case viewMenu, graph, map, inspector, gear, chevronDown, folder, kebab

    /// The artwork: its path data, its viewBox, how wide its stroke is in
    /// viewBox units (nil for a filled glyph), and the size #1146 draws it.
    /// The path data is #1146's, separated by commas rather than spaces,
    /// which SVG reads the same and the shell's wording scan does not take
    /// for a sentence.
    struct Artwork {
        let data: String
        let viewBox: CGRect
        let stroke: CGFloat?
        let size: CGSize
    }

    /// A rounded 13×11 frame at (0.5, 0.5) in a 14×12 box: the graph and
    /// inspector icons' `<rect rx="2">`.
    private static let frame =
        "M2.5,0.5H11.5A2,2,0,0,1,13.5,2.5V9.5A2,2,0,0,1,11.5,11.5H2.5A2,2,0,0,1,0.5,9.5V2.5A2,2,0,0,1,2.5,0.5Z"

    var artwork: Artwork {
        let toolbar = CGRect(x: 0, y: 0, width: 14, height: 12)
        switch self {
        case .viewMenu:
            return Artwork(data: "M1,2h12M1,6h12M1,10h12", viewBox: toolbar, stroke: 1.4, size: toolbar.size)
        case .graph:
            return Artwork(data: Self.frame + "M0.5,7.5h13M4,7.5v4M8,7.5v4", viewBox: toolbar, stroke: 1.3, size: toolbar.size)
        case .map:
            return Artwork(
                data: "M0.5,2.5l4-2,5,2,4-2v9l-4,2-5-2-4,2zM4.5,0.5v9M9.5,2.5v9", viewBox: toolbar, stroke: 1.3,
                size: toolbar.size)
        case .inspector:
            return Artwork(data: Self.frame + "M9,0.5v11", viewBox: toolbar, stroke: 1.3, size: toolbar.size)
        case .gear:
            return Artwork(
                data: "M19.47,10.14L21.9,10.59L21.9,13.41L19.47,13.86L18.6,15.97L20,18L18,20L15.97,18.6L13.86,19.47"
                    + "L13.41,21.9L10.59,21.9L10.14,19.47L8.03,18.6L6,20L4,18L5.4,15.97L4.53,13.86L2.1,13.41L2.1,10.59"
                    + "L4.53,10.14L5.4,8.03L4,6L6,4L8.03,5.4L10.14,4.53L10.59,2.1L13.41,2.1L13.86,4.53L15.97,5.4L18,4"
                    + "L20,6L18.6,8.03Z"
                    + "M15.2,12A3.2,3.2,0,1,1,8.8,12A3.2,3.2,0,1,1,15.2,12Z",
                viewBox: CGRect(x: 0, y: 0, width: 24, height: 24), stroke: 1.7, size: CGSize(width: 15, height: 15))
        case .chevronDown:
            return Artwork(
                data: "M4,6.5l4,4,4-4", viewBox: CGRect(x: 0, y: 0, width: 16, height: 16), stroke: 1.8,
                size: CGSize(width: 10, height: 10))
        case .folder:
            return Artwork(
                data: "M3,7a2,2,0,0,1,2-2h4l2,2h8a2,2,0,0,1,2,2v9a2,2,0,0,1-2,2H5a2,2,0,0,1-2-2z",
                viewBox: CGRect(x: 0, y: 0, width: 24, height: 24), stroke: 1.8, size: CGSize(width: 13, height: 13))
        case .kebab:
            return Artwork(
                data: "M3.6,2A1.6,1.6,0,1,1,0.4,2A1.6,1.6,0,1,1,3.6,2Z"
                    + "M3.6,7A1.6,1.6,0,1,1,0.4,7A1.6,1.6,0,1,1,3.6,7Z"
                    + "M3.6,12A1.6,1.6,0,1,1,0.4,12A1.6,1.6,0,1,1,3.6,12Z",
                viewBox: CGRect(x: 0, y: 0, width: 4, height: 14), stroke: nil, size: CGSize(width: 4, height: 14))
        }
    }

    /// The parsed path in viewBox units, once per process. Nil only for
    /// malformed data, which GlyphTests rules out.
    var path: Path? { Self.parsed[self] }

    private static let parsed: [GlassGlyph: Path] = allCases.reduce(into: [:]) { result, glyph in
        if let path = SVGPathData.parse(glyph.artwork.data) { result[glyph] = Path(path) }
    }
}

/// A glyph's outline scaled into its frame, aspect kept and centred.
struct GlassGlyphShape: Shape {
    let glyph: GlassGlyph

    func path(in rect: CGRect) -> Path {
        guard let path = glyph.path else { return Path() }
        return path.applying(Self.transform(glyph.artwork.viewBox, into: rect))
    }

    /// viewBox units to `rect`: the larger scale that still fits, centred.
    static func transform(_ box: CGRect, into rect: CGRect) -> CGAffineTransform {
        let scale = min(rect.width / box.width, rect.height / box.height)
        return CGAffineTransform(translationX: rect.midX, y: rect.midY)
            .scaledBy(x: scale, y: scale)
            .translatedBy(x: -box.midX, y: -box.midY)
    }
}

/// A `GlassGlyph` at #1146's size, stroked or filled in the foreground
/// style, hidden from assistive tech.
public struct GlassGlyphView: View {
    private let glyph: GlassGlyph

    public init(_ glyph: GlassGlyph) {
        self.glyph = glyph
    }

    /// The stroke in points at the glyph's drawn size.
    static func lineWidth(_ glyph: GlassGlyph) -> CGFloat? {
        let artwork = glyph.artwork
        guard let stroke = artwork.stroke else { return nil }
        return stroke * min(artwork.size.width / artwork.viewBox.width, artwork.size.height / artwork.viewBox.height)
    }

    public var body: some View {
        let shape = GlassGlyphShape(glyph: glyph)
        Group {
            if let width = Self.lineWidth(glyph) {
                shape.stroke(style: StrokeStyle(lineWidth: width, lineCap: .butt, lineJoin: .round))
            } else {
                shape.fill()
            }
        }
        .frame(width: glyph.artwork.size.width, height: glyph.artwork.size.height)
        .accessibilityHidden(true)
    }
}
