import SwiftUI

/// Where a slanted band of light is as it crosses a field: the map's
/// shimmer (`GlassMapShimmer`) and the card's (`GlassCardShine`) both draw
/// one, from the same geometry.
enum GlassShimmerBand {
    /// The band at `progress` (0 wholly before the field, 1 wholly past
    /// it): the gradient's start and end. The band leans `slant` degrees
    /// from upright, its top to the right, and travels left to right, from
    /// wholly before the field's first corner to wholly past its last, so a
    /// pass starts and ends off the field whatever the field's shape.
    /// `halfWidth` is half the band, as a share of the field's diagonal.
    static func band(progress: Double, size: CGSize, slant: Double, halfWidth: CGFloat) -> (start: CGPoint, end: CGPoint) {
        let angle = slant * .pi / 180
        // Square to the band: rightward, tipped down by the slant (y runs
        // down), so the band itself leans with its top to the right.
        let unit = CGVector(dx: CGFloat(cos(angle)), dy: CGFloat(sin(angle)))
        let corners = [CGPoint.zero, CGPoint(x: size.width, y: 0), CGPoint(x: 0, y: size.height),
                       CGPoint(x: size.width, y: size.height)]
        let along = corners.map { $0.x * unit.dx + $0.y * unit.dy }
        let half = halfWidth * max(hypot(size.width, size.height), 1)
        let from = (along.min() ?? 0) - half
        let to = (along.max() ?? 0) + half
        let centre = from + (to - from) * CGFloat(progress)
        return (CGPoint(x: unit.dx * (centre - half), y: unit.dy * (centre - half)),
                CGPoint(x: unit.dx * (centre + half), y: unit.dy * (centre + half)))
    }
}
