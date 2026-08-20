// Render the mark to PNG at a given size, with CoreGraphics.
//
// Usage: render-mark.swift <geometry.json> <scheme:light|dark> <variant:framed|template> <size> <out.png>
//
// # Why this exists rather than a call to a rasterizer
//
// sips can rasterize assets/mark/mark-light.svg, and it does so correctly at
// every size the icon ladder needs -- that was measured, not assumed. It is
// still not what this build depends on: sips' SVG handling is not a documented
// interface, it goes through whatever Quick Look generator the OS ships, and
// every alternative rasterizer (rsvg-convert, cairosvg, ImageMagick, Inkscape)
// is a build dependency that a hosted macos runner does not have.
//
// CoreGraphics is on every Mac, needs nothing installed, and draws the
// geometry exactly rather than interpreting a document.
//
// The numbers are NOT written out here. They are read from the JSON that
// crates/trace-commons-mark generates, so this renderer is a consumer of the
// one description rather than a fifth copy of it.

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct SchemeColors: Decodable {
    let surface: String
    let line: String
    let bracketOpen: String
    let bracketClose: String
    let ink: String
}

struct Geometry: Decodable {
    let view: Int
    let frame: [Int]
    let strokeFrame: Int
    let strokeFramed: Int
    let strokeTemplate: Int
    let bracketOpen: [[Int]]
    let bracketClose: [[Int]]
    let schemes: [String: SchemeColors]
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FATAL: \(message)\n".utf8))
    exit(1)
}

/// `#rrggbb` to a CoreGraphics colour. A malformed literal is a typo in
/// generated JSON, so it stops the build rather than drawing something else.
func color(_ hex: String) -> CGColor {
    var text = hex
    if text.hasPrefix("#") { text.removeFirst() }
    guard text.count == 6, let value = UInt32(text, radix: 16) else {
        fail("malformed colour \(hex)")
    }
    return CGColor(
        red: CGFloat((value >> 16) & 0xFF) / 255,
        green: CGFloat((value >> 8) & 0xFF) / 255,
        blue: CGFloat(value & 0xFF) / 255,
        alpha: 1
    )
}

let args = CommandLine.arguments
guard args.count >= 5 else {
    fail(
        "usage: render-mark.swift <geometry.json> <light|dark> <framed|template> <size>:<out.png> ..."
    )
}
let geometryPath = args[1]
let schemeName = args[2]
let variant = args[3]

// Every requested size in one process. `swift file.swift` compiles before it
// runs, so ten invocations would put ten compiles into every bundle build.
let requests: [(size: Int, path: String)] = args.dropFirst(4).map { argument in
    let parts = argument.split(separator: ":", maxSplits: 1)
    guard parts.count == 2, let size = Int(parts[0]), size > 0 else {
        fail("expected <size>:<out.png>, got \(argument)")
    }
    return (size, String(parts[1]))
}

guard let data = FileManager.default.contents(atPath: geometryPath) else {
    fail("cannot read \(geometryPath)")
}
let geometry: Geometry
do {
    geometry = try JSONDecoder().decode(Geometry.self, from: data)
} catch {
    fail("cannot decode \(geometryPath): \(error)")
}
guard let palette = geometry.schemes[schemeName] else { fail("no scheme \(schemeName)") }
guard geometry.frame.count == 4 else { fail("frame must have four components") }

/// Draw the mark at one size and write it out.
///
/// Each size is drawn at its own resolution rather than resampled from a larger
/// bitmap. The geometry is vector, so a native draw is sharper at the small
/// rungs and costs nothing extra.
func render(size: Int, to outPath: String) {
    guard
        let context = CGContext(
            data: nil,
            width: size,
            height: size,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )
    else { fail("cannot create a \(size)x\(size) context") }

    // The mark's coordinate space is `view` units square and CoreGraphics'
    // origin is bottom-left where SVG's is top-left, so the transform both
    // scales and flips. Everything below is in view units.
    let unit = CGFloat(size) / CGFloat(geometry.view)
    context.translateBy(x: 0, y: CGFloat(size))
    context.scaleBy(x: unit, y: -unit)
    context.setLineCap(.butt)
    context.setLineJoin(.miter)

    func stroke(_ vertices: [[Int]], _ ink: CGColor, _ width: Int) {
        guard let first = vertices.first, first.count == 2 else { return }
        context.setStrokeColor(ink)
        context.setLineWidth(CGFloat(width))
        context.beginPath()
        context.move(to: CGPoint(x: CGFloat(first[0]), y: CGFloat(first[1])))
        for point in vertices.dropFirst() where point.count == 2 {
            context.addLine(to: CGPoint(x: CGFloat(point[0]), y: CGFloat(point[1])))
        }
        context.strokePath()
    }

    switch variant {
    case "framed":
        let rect = CGRect(
            x: CGFloat(geometry.frame[0]),
            y: CGFloat(geometry.frame[1]),
            width: CGFloat(geometry.frame[2]),
            height: CGFloat(geometry.frame[3])
        )
        context.setFillColor(color(palette.surface))
        context.fill(rect)
        context.setStrokeColor(color(palette.line))
        context.setLineWidth(CGFloat(geometry.strokeFrame))
        context.stroke(rect)
        stroke(geometry.bracketOpen, color(palette.bracketOpen), geometry.strokeFramed)
        stroke(geometry.bracketClose, color(palette.bracketClose), geometry.strokeFramed)
    case "template":
        stroke(geometry.bracketOpen, color(palette.ink), geometry.strokeTemplate)
        stroke(geometry.bracketClose, color(palette.ink), geometry.strokeTemplate)
    default:
        fail("unknown variant \(variant)")
    }

    guard let image = context.makeImage() else { fail("no image from context") }
    let url = URL(fileURLWithPath: outPath)
    guard
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fail("cannot write \(outPath)") }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { fail("cannot finalize \(outPath)") }
}

for request in requests {
    render(size: request.size, to: request.path)
}
