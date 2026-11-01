import SwiftUI

/// The Traces map's library on the standard map (owner, 2026-10-19): a
/// glass orb with the NEAR AI mark at its centre, its meridians turning
/// about a leaning axis. The station (`GlassLibraryStation`) is the same
/// library on the rocket map.
///
/// The sphere fills the frame less `inset` on each side, so the glow has
/// room; Reduce Motion holds it still. `lit` is false while nothing has
/// been contributed, and the orb is drawn dimmer.
///
/// `date`, when given, is the moment to draw, from a caller's own timeline
/// (the flow map); with none the orb runs its own at `frameInterval`, only
/// while its window can be seen (`GlassAnimationTimeline`).
public struct GlassLibraryOrb: View {
    private let lit: Bool
    private let date: Date?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(lit: Bool = true, date: Date? = nil) {
        self.lit = lit
        self.date = date
    }

    /// One turn of the meridians.
    public static let turnSeconds: Double = 18
    /// The angle the orb rests at, and starts from.
    static let restAngle: Double = 0.4
    /// The share of the frame's half-size left round the sphere.
    nonisolated public static let inset: CGFloat = 0.29
    /// The meridians drawn, and the axis's lean.
    static let meridians = 6
    static let lean: Double = -14 * Double.pi / 180
    /// The orb's own frame rate, as the station's.
    public static let frameInterval: Double = GlassLibraryStation.frameInterval

    public var body: some View {
        Group {
            if let date {
                frame(at: date)
            } else {
                GlassAnimationTimeline(minimumInterval: Self.frameInterval, paused: reduceMotion) { date in
                    frame(at: date)
                }
            }
        }
        .shadow(color: GlassLibraryStation.brandBlue.opacity(lit ? 0.5 : 0.15), radius: 10)
        .accessibilityHidden(true)
    }

    private func frame(at date: Date) -> some View {
        let seconds: Double = date.timeIntervalSinceReferenceDate
        let turn: Double = seconds.truncatingRemainder(dividingBy: Self.turnSeconds) / Self.turnSeconds
        let angle: Double = reduceMotion ? Self.restAngle : Self.restAngle + turn * 2 * .pi
        let lit = lit
        return Canvas { context, size in
            Self.draw(in: &context, size: size, angle: angle, lit: lit)
        }
    }

    /// Each meridian's half-width as a share of the radius at `angle`, and
    /// whether that meridian is on the near side.
    static func meridianWidths(angle: Double) -> [(width: Double, near: Bool)] {
        (0 ..< meridians).map { index in
            let phase: Double = angle + Double(index) * Double.pi / Double(meridians)
            return (width: abs(cos(phase)), near: sin(phase) >= 0)
        }
    }

    /// The sphere's radius in a frame of `size`.
    public static func radius(in size: CGSize) -> CGFloat {
        let half: CGFloat = min(size.width, size.height) / 2
        return half * (1 - inset)
    }

    static func draw(in context: inout GraphicsContext, size: CGSize, angle: Double, lit: Bool) {
        let r: CGFloat = radius(in: size)
        let centre = CGPoint(x: size.width / 2, y: size.height / 2)
        let blue: Color = GlassLibraryStation.brandBlue
        let deep: Color = GlassTokens.Gradient.heroCampaign.stops.first?.color.color ?? blue
        let sphere = Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2))
        var orb = context
        orb.opacity = lit ? 1 : 0.6

        // The glass: deep at the rim, bright toward the light above left.
        let light = CGPoint(x: centre.x - r * 0.35, y: centre.y - r * 0.4)
        orb.fill(sphere, with: .radialGradient(
            Gradient(colors: [blue.opacity(0.95), deep.opacity(0.95), deep]),
            center: light, startRadius: 0, endRadius: r * 1.5))

        // The lines, inside the sphere and turned about the leaning axis.
        orb.drawLayer { lines in
            lines.clip(to: sphere)
            lines.translateBy(x: centre.x, y: centre.y)
            lines.rotate(by: .radians(lean))
            let ink = Color.white
            for latitude in [-0.5, 0.0, 0.5] {
                let y: CGFloat = r * CGFloat(latitude)
                let half: CGFloat = r * CGFloat((1 - latitude * latitude).squareRoot())
                let ring = Path(ellipseIn: CGRect(x: -half, y: y - half * 0.22, width: half * 2, height: half * 0.44))
                lines.stroke(ring, with: .color(ink.opacity(0.18)), lineWidth: 1)
            }
            for meridian in meridianWidths(angle: angle) {
                let w: CGFloat = r * CGFloat(meridian.width)
                let line = Path(ellipseIn: CGRect(x: -w, y: -r, width: w * 2, height: r * 2))
                lines.stroke(line, with: .color(ink.opacity(meridian.near ? 0.32 : 0.12)), lineWidth: meridian.near ? 1.2 : 0.8)
            }
        }

        // The mark at the centre, then the highlight and the rim.
        let side: CGFloat = r * 0.95
        if let mark = GlassBrandMark.path(at: CGPoint(x: centre.x - side / 2, y: centre.y - side / 2), size: side) {
            orb.fill(mark, with: .color(.white))
        }
        let shine = Path(ellipseIn: CGRect(x: centre.x - r * 0.62, y: centre.y - r * 0.78, width: r * 0.7, height: r * 0.38))
        orb.fill(shine, with: .linearGradient(
            Gradient(colors: [Color.white.opacity(0.45), Color.white.opacity(0)]),
            startPoint: CGPoint(x: centre.x, y: centre.y - r * 0.78), endPoint: CGPoint(x: centre.x, y: centre.y - r * 0.4)))
        orb.stroke(sphere, with: .color(Color.white.opacity(0.35)), lineWidth: 1)
    }
}
