import SwiftUI

/// Traces as four connected nodes in a T (owner, 2026-10-22: the edge
/// rail's Traces icon, simpler than the Trace mark): three across the top
/// joined by a line, and one under the middle joined to it. Drawn in
/// `ink`, square, filling its frame.
public struct GlassTraceNodesGlyph: View {
    private let ink: Color

    public init(ink: Color) {
        self.ink = ink
    }

    /// The nodes in a unit square: left, middle and right across the top,
    /// then the one under the middle.
    static let nodes: [CGPoint] = [
        CGPoint(x: 0.16, y: 0.3), CGPoint(x: 0.5, y: 0.3), CGPoint(x: 0.84, y: 0.3), CGPoint(x: 0.5, y: 0.84),
    ]
    /// A node's radius and the lines' width, as shares of the square.
    static let nodeRadius: CGFloat = 0.12
    static let lineWidth: CGFloat = 0.08

    /// The two lines: across the top row, and down from its middle node.
    static let links: [(Int, Int)] = [(0, 2), (1, 3)]

    public var body: some View {
        Canvas { context, size in
            let side = min(size.width, size.height)
            let origin = CGPoint(x: (size.width - side) / 2, y: (size.height - side) / 2)
            func point(_ unit: CGPoint) -> CGPoint {
                CGPoint(x: origin.x + unit.x * side, y: origin.y + unit.y * side)
            }
            var lines = Path()
            for (from, to) in Self.links {
                lines.move(to: point(Self.nodes[from]))
                lines.addLine(to: point(Self.nodes[to]))
            }
            context.stroke(lines, with: .color(ink), style: StrokeStyle(lineWidth: Self.lineWidth * side, lineCap: .round))
            let radius = Self.nodeRadius * side
            for node in Self.nodes {
                let centre = point(node)
                context.fill(
                    Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2)),
                    with: .color(ink))
            }
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}
