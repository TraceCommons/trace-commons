import CoreGraphics
import SwiftUI

// The Trace mark's "1c Live trace" variant, from the owner's Claude Design
// project (Trace Logo, 1c), in its own 200 by 200 coordinates. Spaces are
// written as commas, as in GlassLogoPaths. GlassTraceMarkTests parses it.

/// The Trace mark, live: a trace across a 3 by 3 diagonal lattice of nodes.
/// The visited joins and nodes run a violet, purple, hot-pink gradient
/// (`traceLive*`), ending in a soft glow on the current step; the unvisited
/// nodes, the possible next steps, take the brand purple.
///
/// `ink` draws it in one colour instead, the unvisited nodes faded, for a
/// ground the gradient would not read on (a selected rail tile).
public struct GlassTraceMark: View {
    private let ink: Color?

    public init(ink: Color? = nil) {
        self.ink = ink
    }

    /// The mark's coordinate space.
    static let side: CGFloat = 200

    /// The necked joins between visited nodes.
    static let joins = [
        "M90.12,47.44C88.22,59.44,84.94,62.72,72.94,64.62L84.38,76.06C86.28,64.06,89.56,60.78,101.56,58.88Z",
        "M64.62,72.94C62.72,84.94,59.44,88.22,47.44,90.12L58.88,101.56C60.78,89.56,64.06,86.28,76.06,84.38Z",
        "M72.94,84.38C84.94,86.28,88.22,89.56,90.12,101.56L101.56,90.12C89.56,88.22,86.28,84.94,84.38,72.94Z",
        "M98.44,109.88C110.44,111.78,113.72,115.06,115.62,127.06L127.06,115.62C115.06,113.72,111.78,110.44,109.88,98.44Z",
    ]

    /// Visited nodes, the current step last.
    static let visited = [
        CGPoint(x: 100, y: 49), CGPoint(x: 74.5, y: 74.5), CGPoint(x: 49, y: 100), CGPoint(x: 100, y: 100),
        CGPoint(x: 125.5, y: 125.5),
    ]

    /// Unvisited nodes: the possible next steps.
    static let unvisited = [
        CGPoint(x: 125.5, y: 74.5), CGPoint(x: 151, y: 100), CGPoint(x: 74.5, y: 125.5), CGPoint(x: 100, y: 151),
    ]

    static let nodeRadius: CGFloat = 10
    static let glowRadius: CGFloat = 30

    /// The joins, parsed once. Nil only if the data is malformed, which
    /// GlassTraceMarkTests rules out.
    static let joinPaths: [Path]? = {
        let parsed = joins.compactMap(SVGPathData.parse)
        return parsed.count == joins.count ? parsed.map { Path($0) } : nil
    }()

    /// The visited joins and nodes as one path, in the mark's coordinates.
    static var trace: Path {
        var path = Path()
        joinPaths?.forEach { path.addPath($0) }
        visited.forEach { path.addEllipse(in: circle($0, nodeRadius)) }
        return path
    }

    static func circle(_ centre: CGPoint, _ radius: CGFloat) -> CGRect {
        CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2)
    }

    public var body: some View {
        Canvas { context, size in
            let scale = min(size.width, size.height) / Self.side
            context.translateBy(
                x: (size.width - Self.side * scale) / 2, y: (size.height - Self.side * scale) / 2)
            context.scaleBy(x: scale, y: scale)
            let current = Self.visited[Self.visited.count - 1]
            var rest = Path()
            Self.unvisited.forEach { rest.addEllipse(in: Self.circle($0, Self.nodeRadius)) }
            if let ink {
                context.fill(rest, with: .color(ink.opacity(0.45)))
                context.fill(Self.trace, with: .color(ink))
                return
            }
            context.fill(
                Path(ellipseIn: Self.circle(current, Self.glowRadius)),
                with: .radialGradient(
                    Gradient(stops: [
                        .init(color: GlassTokens.Color.traceLiveEnd.color.opacity(0.55), location: 0.3),
                        .init(color: GlassTokens.Color.traceLiveEnd.color.opacity(0), location: 1),
                    ]),
                    center: current, startRadius: 0, endRadius: Self.glowRadius))
            context.fill(
                Self.trace,
                with: .linearGradient(
                    Gradient(colors: [
                        GlassTokens.Color.traceLiveStart.color, GlassTokens.Color.traceLiveMid.color,
                        GlassTokens.Color.traceLiveEnd.color,
                    ]),
                    startPoint: CGPoint(x: 68, y: 68), endPoint: CGPoint(x: 134, y: 134)))
            context.fill(rest, with: .color(GlassTokens.Color.purple.color))
        }
        .accessibilityHidden(true)
    }
}
