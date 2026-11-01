import CoreGraphics
import Foundation
import simd
import SwiftUI

/// The library on the Traces map (owner, 2026-10-16): a space station built
/// round the NEAR AI mark, drawn as a real mesh. The mark is extruded from
/// the brand kit's path into a slab and lies in the station's plane with a
/// torus ring, four spokes, four modules and two solar arrays round it.
/// Each frame the mesh turns about its own axis, is tilted toward the
/// viewer and leaned, projected in perspective, sorted far to near and
/// flat-shaded from one light. One turn takes `turnSeconds`; under Reduce
/// Motion it is drawn once and holds still.
///
/// `lit` is false while nothing has been contributed. The station itself is
/// drawn as bright either way (owner, 2026-10-19: dimmed, it read too dark
/// beside the launch pad); only its glow is softer.
///
/// `date`, when given, is the moment to draw: a caller that already runs a
/// timeline (the flow map) passes its own, so one timeline drives the
/// whole map. With none the station runs its own, at `frameInterval`, and
/// only while its window can be seen (`GlassAnimationTimeline`).
public struct GlassLibraryStation: View {
    private let lit: Bool
    private let date: Date?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(lit: Bool = true, date: Date? = nil) {
        self.lit = lit
        self.date = date
    }

    /// One turn of the station.
    public static let turnSeconds: Double = 26
    /// The angle the station rests at, and starts from.
    static let restAngle: Double = 0.6
    /// The station's own frame rate: one turn in 26 seconds moves the
    /// solar arrays' tips about a point a frame at 20 frames a second.
    public static let frameInterval: Double = 1 / 20

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
        .shadow(color: Self.brandBlue.opacity(Self.glow(lit: lit)), radius: 6)
        .accessibilityHidden(true)
    }

    /// The glow's strength: full once something has been contributed, a
    /// little softer before.
    static func glow(lit: Bool) -> Double { lit ? 0.35 : 0.25 }

    /// The station's angle at `seconds` past the reference date.
    static func angle(at seconds: Double, reduceMotion: Bool) -> Double {
        reduceMotion
            ? restAngle
            : restAngle + seconds.truncatingRemainder(dividingBy: turnSeconds) / turnSeconds * 2 * .pi
    }

    private func frame(at date: Date) -> some View {
        let angle = Self.angle(at: date.timeIntervalSinceReferenceDate, reduceMotion: reduceMotion)
        return Canvas { context, size in
            Self.draw(in: &context, size: size, angle: angle)
        }
    }

    // MARK: Mesh

    /// One flat face: its corners, its colour (0...1), and whether it is
    /// one of the mark's two faces or part of the mark at all.
    struct Face {
        let points: [SIMD3<Double>]
        let color: SIMD3<Double>
        var markCap = false
        var mark = false
    }

    /// The NEAR AI brand blue, #0091fd: the campaign gradient's light stop.
    static let brandBlue = GlassTokens.Gradient.heroCampaign.stops.last?.color.color ?? GlassTokens.Color.blue.color

    static let mesh: [Face] = build()

    /// The mark's outline from the brand kit path (`GlassBrandMark`),
    /// curves flattened, centred, scaled to `height` units and turned y-up.
    static func markOutline(height: Double) -> [SIMD2<Double>] {
        guard let path = SVGPathData.parse(GlassBrandMark.data) else { return [] }
        // Each step typed and small: the macOS 15 toolchain gives up on
        // one long expression of point arithmetic here.
        var points: [SIMD2<Double>] = []
        var current = SIMD2<Double>.zero
        path.applyWithBlock { element in
            let e: CGPathElement = element.pointee
            var curve: [SIMD2<Double>] = []
            switch e.type {
            case .moveToPoint, .addLineToPoint:
                current = vector(e.points[0])
                points.append(current)
            case .addQuadCurveToPoint:
                curve = [current, vector(e.points[0]), vector(e.points[1])]
            case .addCurveToPoint:
                curve = [current, vector(e.points[0]), vector(e.points[1]), vector(e.points[2])]
            case .closeSubpath:
                break
            @unknown default:
                break
            }
            guard let last = curve.last else { return }
            for step in 1...5 {
                points.append(bezier(curve, at: Double(step) / 5))
            }
            current = last
        }
        let box = GlassBrandMark.viewBox
        let k: Double = height / Double(box.height)
        let centre = SIMD2<Double>(Double(box.midX), Double(box.midY))
        var outline: [SIMD2<Double>] = []
        for point in points {
            let offset: SIMD2<Double> = point - centre
            let q = SIMD2<Double>(offset.x * k, -offset.y * k)
            if let last = outline.last, simd_distance(q, last) <= 0.05 { continue }
            outline.append(q)
        }
        if let first = outline.first, let last = outline.last, outline.count > 1, simd_distance(first, last) < 0.05 {
            outline.removeLast()
        }
        return outline
    }

    static func vector(_ point: CGPoint) -> SIMD2<Double> {
        SIMD2<Double>(Double(point.x), Double(point.y))
    }

    /// The point `t` of the way along a Bezier curve given by its control
    /// points (de Casteljau).
    static func bezier(_ controls: [SIMD2<Double>], at t: Double) -> SIMD2<Double> {
        var level = controls
        while level.count > 1 {
            var next: [SIMD2<Double>] = []
            for index in 0 ..< level.count - 1 {
                let a: SIMD2<Double> = level[index]
                let b: SIMD2<Double> = level[index + 1]
                next.append(a + (b - a) * t)
            }
            level = next
        }
        return level.first ?? .zero
    }

    static func build() -> [Face] {
        var faces: [Face] = []
        func box(_ c: SIMD3<Double>, _ s: SIMD3<Double>, _ color: SIMD3<Double>) {
            let a = c - s / 2, b = c + s / 2
            let p = { (x: Double, y: Double, z: Double) in SIMD3<Double>(x, y, z) }
            faces.append(Face(points: [p(a.x, a.y, b.z), p(b.x, a.y, b.z), p(b.x, b.y, b.z), p(a.x, b.y, b.z)], color: color))
            faces.append(Face(points: [p(b.x, a.y, a.z), p(a.x, a.y, a.z), p(a.x, b.y, a.z), p(b.x, b.y, a.z)], color: color))
            faces.append(Face(points: [p(a.x, b.y, a.z), p(a.x, b.y, b.z), p(b.x, b.y, b.z), p(b.x, b.y, a.z)], color: color))
            faces.append(Face(points: [p(a.x, a.y, b.z), p(a.x, a.y, a.z), p(b.x, a.y, a.z), p(b.x, a.y, b.z)], color: color))
            faces.append(Face(points: [p(b.x, a.y, b.z), p(b.x, a.y, a.z), p(b.x, b.y, a.z), p(b.x, b.y, b.z)], color: color))
            faces.append(Face(points: [p(a.x, a.y, a.z), p(a.x, a.y, b.z), p(a.x, b.y, b.z), p(a.x, b.y, a.z)], color: color))
        }
        func rgb(_ r: Double, _ g: Double, _ b: Double) -> SIMD3<Double> { SIMD3(r, g, b) / 255 }

        // The mark, extruded, lying in the station's plane (owner,
        // 2026-10-16): its top toward the plane's far side, so it reads
        // upright once the station is tilted.
        let mark = markOutline(height: 30)
        let h = 3.2
        let blue = rgb(0, 145, 253)
        faces.append(Face(points: mark.map { SIMD3($0.x, h, -$0.y) }, color: blue, markCap: true, mark: true))
        faces.append(Face(points: mark.reversed().map { SIMD3($0.x, -h, -$0.y) }, color: blue, markCap: true, mark: true))
        for (j, p) in mark.enumerated() {
            let q = mark[(j + 1) % mark.count]
            faces.append(Face(points: [SIMD3(p.x, h, -p.y), SIMD3(q.x, h, -q.y), SIMD3(q.x, -h, -q.y), SIMD3(p.x, -h, -p.y)],
                              color: rgb(0, 98, 176), mark: true))
        }

        // The ring: a torus round the mark, in the same plane; metal with a
        // little of the brand blue in it.
        let ringRadius = 36.0, tube = 3.6, around = 40, across = 8
        func torus(_ u: Int, _ v: Int) -> SIMD3<Double> {
            let a = Double(u) / Double(around) * 2 * .pi, b = Double(v) / Double(across) * 2 * .pi
            let r = ringRadius + tube * cos(b)
            return SIMD3(r * cos(a), tube * sin(b), r * sin(a))
        }
        for u in 0..<around {
            for v in 0..<across {
                faces.append(Face(points: [torus(u, v), torus(u + 1, v), torus(u + 1, v + 1), torus(u, v + 1)],
                                  color: rgb(172, 196, 230)))
            }
        }

        // Spokes from beside the mark to the ring, and modules on the ring.
        let spoke = rgb(140, 164, 200), hub = 17.0, length = ringRadius - hub, mid = (hub + ringRadius) / 2
        box(SIMD3(0, 0, mid), SIMD3(1.8, 1.8, length), spoke)
        box(SIMD3(0, 0, -mid), SIMD3(1.8, 1.8, length), spoke)
        box(SIMD3(mid, 0, 0), SIMD3(length, 1.8, 1.8), spoke)
        box(SIMD3(-mid, 0, 0), SIMD3(length, 1.8, 1.8), spoke)
        for degrees in [45.0, 135, 225, 315] {
            let a = degrees * .pi / 180
            box(SIMD3(ringRadius * cos(a), 0, ringRadius * sin(a)), SIMD3(6, 5, 6), rgb(226, 236, 248))
        }

        // Solar arrays on trusses beyond the ring, lying in the station's
        // plane like the mark.
        let truss = rgb(120, 134, 156), panel = rgb(44, 96, 176)
        for side in [1.0, -1.0] {
            box(SIMD3(side * (ringRadius + 9), 0, 0), SIMD3(14, 1.6, 1.6), truss)
            box(SIMD3(side * (ringRadius + 20), 0, 0), SIMD3(16, 0.9, 20), panel)
            box(SIMD3(side * (ringRadius + 20), 0, 0), SIMD3(0.9, 1.1, 20.4), truss)
        }
        return faces
    }

    // MARK: Drawing

    /// The station's tilt toward the viewer and its lean, and the camera.
    static let tilt = 42 * Double.pi / 180
    static let lean = -14 * Double.pi / 180
    static let cameraDistance = 240.0
    /// The light, from above left and in front.
    static let light = simd_normalize(SIMD3<Double>(-0.42, 0.62, 0.66))

    /// A point of the mesh turned by `angle` about the station's axis, then
    /// tilted and leaned.
    static func turned(_ p: SIMD3<Double>, angle: Double) -> SIMD3<Double> {
        turned(p, cy: cos(angle), sy: sin(angle))
    }

    /// The tilt's and the lean's cosines and sines, worked out once rather
    /// than for every corner of every face on every frame.
    static let cx = cos(tilt), sx = sin(tilt)
    static let cz = cos(lean), sz = sin(lean)

    /// `turned(_:angle:)` with the angle's cosine and sine given, so a
    /// frame works them out once for the whole mesh. The same arithmetic in
    /// the same order: the result is identical.
    static func turned(_ p: SIMD3<Double>, cy: Double, sy: Double) -> SIMD3<Double> {
        let x = p.x * cy + p.z * sy, z = -p.x * sy + p.z * cy, y = p.y
        let y2 = y * cx - z * sx, z2 = y * sx + z * cx
        return SIMD3(x * cz - y2 * sz, x * sz + y2 * cz, z2)
    }

    static func draw(in context: inout GraphicsContext, size: CGSize, angle: Double) {
        let scale = min(Double(size.width) / 140, Double(size.height) / 120)
        func project(_ q: SIMD3<Double>) -> CGPoint {
            let s = scale * cameraDistance / (cameraDistance - q.z)
            return CGPoint(x: Double(size.width) / 2 + q.x * s, y: Double(size.height) / 2 - q.y * s)
        }
        struct Drawn {
            let points: [SIMD3<Double>]
            let color: SIMD3<Double>
            let shade: Double
            let spec: Double
            var depth: Double
            let cap: Bool
        }
        var drawn: [Drawn] = []
        drawn.reserveCapacity(mesh.count)
        var markNear = -Double.infinity, markFar = Double.infinity
        let cy = cos(angle), sy = sin(angle)
        for face in mesh {
            let q = face.points.map { turned($0, cy: cy, sy: sy) }
            guard q.count >= 3 else { continue }
            var n = simd_cross(q[1] - q[0], q[2] - q[0])
            let length = simd_length(n)
            n = length > 0 ? n / length : SIMD3(0, 0, 1)
            // Two-sided: light the side that faces the viewer.
            if n.z < 0 { n = -n }
            let lambert = max(0, simd_dot(n, light))
            let spec = pow(max(0, n.z * 0.5 + lambert * 0.5), 18) * 0.35
            let depth = q.reduce(0) { $0 + $1.z } / Double(q.count)
            if face.mark {
                for p in q {
                    markNear = max(markNear, p.z)
                    markFar = min(markFar, p.z)
                }
            }
            drawn.append(Drawn(points: q, color: face.color, shade: 0.3 + 0.7 * lambert, spec: spec, depth: depth, cap: face.markCap))
        }
        // The mark's face toward the viewer is drawn over its own wall, the
        // other under it.
        let markMiddle = (markNear + markFar) / 2
        for index in drawn.indices where drawn[index].cap {
            drawn[index].depth = drawn[index].depth > markMiddle ? markNear + 0.01 : markFar - 0.01
        }
        drawn.sort { $0.depth < $1.depth }

        for face in drawn {
            var path = Path()
            for (index, point) in face.points.enumerated() {
                let s = project(point)
                if index == 0 { path.move(to: s) } else { path.addLine(to: s) }
            }
            path.closeSubpath()
            let shaded: SIMD3<Double> = face.color * face.shade + SIMD3<Double>(repeating: face.spec)
            let c: SIMD3<Double> = shaded
            let color = Color(red: min(1, c.x), green: min(1, c.y), blue: min(1, c.z))
            context.fill(path, with: .color(color))
            // The same colour round the edge closes the seams between faces.
            context.stroke(path, with: .color(color), style: StrokeStyle(lineWidth: 0.6, lineJoin: .round))
        }
    }
}
