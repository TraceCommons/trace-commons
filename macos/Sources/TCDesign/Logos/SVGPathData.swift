import CoreGraphics
import Foundation

/// Parses SVG path data (the `d` attribute) into a `CGPath`.
///
/// It covers the whole path grammar -- M L H V C S Q T A Z, absolute and
/// relative, implicit repeats -- including the compact forms minified
/// artwork uses: numbers run together (`.08-.23`, `1.5.5`) and arc flags
/// written as bare digits (`0 00-.975 0`). Arcs become cubic Béziers.
///
/// Returns nil on malformed data rather than a partial path, so a logo
/// that would draw half a mark draws nothing and its test fails.
enum SVGPathData {
    static func parse(_ data: String) -> CGPath? {
        var scanner = Scanner(Array(data.utf8))
        let path = CGMutablePath()
        var current = CGPoint.zero
        var subpathStart = CGPoint.zero
        // The previous segment's second control point, for S and T.
        var lastCubic: CGPoint?
        var lastQuad: CGPoint?
        var command: UInt8?

        while true {
            scanner.skipSeparators()
            guard let byte = scanner.peek() else { break }
            if Scanner.isCommand(byte) {
                command = byte
                scanner.advance()
            } else if command == nil {
                return nil
            }
            guard let cmd = command else { return nil }
            let relative = cmd >= UInt8(ascii: "a")
            let origin = relative ? current : .zero

            switch cmd | 0x20 {
            case UInt8(ascii: "m"):
                guard let p = scanner.point() else { return nil }
                current = p + origin
                subpathStart = current
                path.move(to: current)
                // Further pairs after a moveto are implicit linetos.
                command = relative ? UInt8(ascii: "l") : UInt8(ascii: "L")
                lastCubic = nil; lastQuad = nil
            case UInt8(ascii: "l"):
                guard let p = scanner.point() else { return nil }
                current = p + origin
                path.addLine(to: current)
                lastCubic = nil; lastQuad = nil
            case UInt8(ascii: "h"):
                guard let x = scanner.number() else { return nil }
                current.x = relative ? current.x + x : x
                path.addLine(to: current)
                lastCubic = nil; lastQuad = nil
            case UInt8(ascii: "v"):
                guard let y = scanner.number() else { return nil }
                current.y = relative ? current.y + y : y
                path.addLine(to: current)
                lastCubic = nil; lastQuad = nil
            case UInt8(ascii: "c"):
                guard let c1 = scanner.point(), let c2 = scanner.point(), let p = scanner.point() else { return nil }
                path.addCurve(to: p + origin, control1: c1 + origin, control2: c2 + origin)
                lastCubic = c2 + origin
                current = p + origin
                lastQuad = nil
            case UInt8(ascii: "s"):
                guard let c2 = scanner.point(), let p = scanner.point() else { return nil }
                let c1 = lastCubic.map { current * 2 - $0 } ?? current
                path.addCurve(to: p + origin, control1: c1, control2: c2 + origin)
                lastCubic = c2 + origin
                current = p + origin
                lastQuad = nil
            case UInt8(ascii: "q"):
                guard let c = scanner.point(), let p = scanner.point() else { return nil }
                path.addQuadCurve(to: p + origin, control: c + origin)
                lastQuad = c + origin
                current = p + origin
                lastCubic = nil
            case UInt8(ascii: "t"):
                guard let p = scanner.point() else { return nil }
                let c = lastQuad.map { current * 2 - $0 } ?? current
                path.addQuadCurve(to: p + origin, control: c)
                lastQuad = c
                current = p + origin
                lastCubic = nil
            case UInt8(ascii: "a"):
                guard let rx = scanner.number(), let ry = scanner.number(),
                      let rotation = scanner.number(),
                      let largeArc = scanner.flag(), let sweep = scanner.flag(),
                      let p = scanner.point() else { return nil }
                let end = p + origin
                addArc(to: path, from: current, to: end, rx: rx, ry: ry,
                       rotation: rotation, largeArc: largeArc, sweep: sweep)
                current = end
                lastCubic = nil; lastQuad = nil
            case UInt8(ascii: "z"):
                path.closeSubpath()
                current = subpathStart
                lastCubic = nil; lastQuad = nil
                // Z takes no arguments; what follows must be a new command.
                command = nil
            default:
                return nil
            }
        }
        return path.isEmpty ? nil : path
    }

    /// SVG's endpoint arc, converted to centre form (SVG 1.1 appendix F.6)
    /// and drawn as cubics of at most a quarter turn each.
    private static func addArc(
        to path: CGMutablePath, from p0: CGPoint, to p1: CGPoint,
        rx: CGFloat, ry: CGFloat, rotation: CGFloat, largeArc: Bool, sweep: Bool
    ) {
        var rx = abs(rx), ry = abs(ry)
        if p0 == p1 { return }
        if rx == 0 || ry == 0 { path.addLine(to: p1); return }

        let phi = rotation * .pi / 180
        let cosPhi = cos(phi), sinPhi = sin(phi)
        let dx = (p0.x - p1.x) / 2, dy = (p0.y - p1.y) / 2
        let x1 = cosPhi * dx + sinPhi * dy
        let y1 = -sinPhi * dx + cosPhi * dy

        // Radii too small to span the endpoints are scaled up to fit.
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
        if lambda > 1 { rx *= lambda.squareRoot(); ry *= lambda.squareRoot() }

        let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
        let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1
        var coefficient = (max(0, numerator) / denominator).squareRoot()
        if largeArc == sweep { coefficient = -coefficient }
        let cx1 = coefficient * rx * y1 / ry
        let cy1 = -coefficient * ry * x1 / rx
        let cx = cosPhi * cx1 - sinPhi * cy1 + (p0.x + p1.x) / 2
        let cy = sinPhi * cx1 + cosPhi * cy1 + (p0.y + p1.y) / 2

        func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
            let sign: CGFloat = ux * vy - uy * vx < 0 ? -1 : 1
            let dot = ux * vx + uy * vy
            let length = (ux * ux + uy * uy).squareRoot() * (vx * vx + vy * vy).squareRoot()
            return sign * acos(min(1, max(-1, dot / length)))
        }
        let theta1 = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry)
        var delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry)
        if !sweep && delta > 0 { delta -= 2 * .pi }
        if sweep && delta < 0 { delta += 2 * .pi }

        let segments = max(1, Int((abs(delta) / (.pi / 2)).rounded(.up)))
        let step = delta / CGFloat(segments)
        let k = 4 / 3 * tan(step / 4)

        func point(_ t: CGFloat) -> CGPoint {
            CGPoint(x: cx + rx * cos(t) * cosPhi - ry * sin(t) * sinPhi,
                    y: cy + rx * cos(t) * sinPhi + ry * sin(t) * cosPhi)
        }
        func derivative(_ t: CGFloat) -> CGPoint {
            CGPoint(x: -rx * sin(t) * cosPhi - ry * cos(t) * sinPhi,
                    y: -rx * sin(t) * sinPhi + ry * cos(t) * cosPhi)
        }
        var t = theta1
        for index in 0 ..< segments {
            let next = t + step
            let start = point(t)
            let end = index == segments - 1 ? p1 : point(next)
            path.addCurve(to: end,
                          control1: start + derivative(t) * k,
                          control2: point(next) - derivative(next) * k)
            t = next
        }
    }
}

private struct Scanner {
    private let bytes: [UInt8]
    private var index = 0

    init(_ bytes: [UInt8]) { self.bytes = bytes }

    static func isCommand(_ byte: UInt8) -> Bool {
        switch byte | 0x20 {
        case UInt8(ascii: "m"), UInt8(ascii: "l"), UInt8(ascii: "h"), UInt8(ascii: "v"),
             UInt8(ascii: "c"), UInt8(ascii: "s"), UInt8(ascii: "q"), UInt8(ascii: "t"),
             UInt8(ascii: "a"), UInt8(ascii: "z"):
            // `e` is an exponent, not a command, and is not in this set.
            return true
        default:
            return false
        }
    }

    func peek() -> UInt8? { index < bytes.count ? bytes[index] : nil }
    mutating func advance() { index += 1 }

    mutating func skipSeparators() {
        while let b = peek(), b == UInt8(ascii: " ") || b == UInt8(ascii: ",")
            || b == UInt8(ascii: "\n") || b == UInt8(ascii: "\t") || b == UInt8(ascii: "\r") {
            index += 1
        }
    }

    /// An arc flag: a single 0 or 1, which may run straight into the next
    /// number (`00-.975` is two flags and then -.975).
    mutating func flag() -> Bool? {
        skipSeparators()
        guard let b = peek(), b == UInt8(ascii: "0") || b == UInt8(ascii: "1") else { return nil }
        index += 1
        return b == UInt8(ascii: "1")
    }

    mutating func point() -> CGPoint? {
        guard let x = number(), let y = number() else { return nil }
        return CGPoint(x: x, y: y)
    }

    /// One number. A second `.` or a sign ends it, which is how `.08-.23`
    /// reads as two numbers.
    mutating func number() -> CGFloat? {
        skipSeparators()
        let start = index
        if let b = peek(), b == UInt8(ascii: "-") || b == UInt8(ascii: "+") { index += 1 }
        var digits = 0
        while let b = peek(), b >= UInt8(ascii: "0") && b <= UInt8(ascii: "9") { index += 1; digits += 1 }
        if peek() == UInt8(ascii: ".") {
            index += 1
            while let b = peek(), b >= UInt8(ascii: "0") && b <= UInt8(ascii: "9") { index += 1; digits += 1 }
        }
        guard digits > 0 else { index = start; return nil }
        if let b = peek(), b == UInt8(ascii: "e") || b == UInt8(ascii: "E") {
            let mark = index
            index += 1
            if let s = peek(), s == UInt8(ascii: "-") || s == UInt8(ascii: "+") { index += 1 }
            var exponentDigits = 0
            while let d = peek(), d >= UInt8(ascii: "0") && d <= UInt8(ascii: "9") { index += 1; exponentDigits += 1 }
            if exponentDigits == 0 { index = mark }
        }
        guard let text = String(bytes: bytes[start ..< index], encoding: .ascii),
              let value = Double(text) else { return nil }
        return CGFloat(value)
    }
}

private func + (a: CGPoint, b: CGPoint) -> CGPoint { CGPoint(x: a.x + b.x, y: a.y + b.y) }
private func - (a: CGPoint, b: CGPoint) -> CGPoint { CGPoint(x: a.x - b.x, y: a.y - b.y) }
private func * (a: CGPoint, k: CGFloat) -> CGPoint { CGPoint(x: a.x * k, y: a.y * k) }
