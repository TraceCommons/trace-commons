import CoreGraphics
import Foundation
import ImageIO

// Assert that a compiled Icon Composer icon carries the mark.
//
// # Why this is not verify-icns.swift
//
// verify-icns.swift matches pixels against the exact light-scheme inks with a
// small tolerance. That works for the flat .icns the CoreGraphics renderer
// produces, where the ink lands on the canvas unmodified.
//
// It does not work here, and weakening its tolerance until it passed would
// have thrown away the check. macOS 26 composites a .icon as glass: it
// lightens, desaturates and adds a specular highlight, so #315FBA arrives as
// a pale periwinkle far outside any tolerance that still means anything.
// Measured against a correct icon, the exact-ink check reports "no blue
// bracket" on every representation.
//
// So this asks the question the glass treatment cannot erase: is there a
// green-dominant mass where the green bracket belongs, and a blue-dominant
// mass where the blue one belongs. Channel dominance survives lightening;
// absolute colour does not.
//
// This still fails the things that matter -- a blank icon, a flat one, one
// bracket missing, both brackets the same colour, or the mark drawn in the
// wrong corner.

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FATAL: \(message)\n".utf8))
    exit(1)
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: verify-icon.swift <icns> [expected-representations]") }
let path = args[1]
let expected = args.count >= 3 ? Int(args[2]) : nil

guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else {
    fail("\(path) does not open as an image source")
}
let count = CGImageSourceGetCount(source)
guard count > 0 else { fail("\(path) has no representations") }
if let expected, count != expected {
    fail("\(path) has \(count) representations, expected \(expected)")
}

// A bracket has to occupy a real share of its quadrant. One stray antialiased
// pixel is not a bracket, and without a floor this check would pass on noise.
let minimumShare = 0.005

for index in 0..<count {
    guard let image = CGImageSourceCreateImageAtIndex(source, index, nil) else {
        fail("representation \(index) does not decode")
    }
    let w = image.width
    let h = image.height
    guard
        let context = CGContext(
            data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot rasterize representation \(index)") }
    context.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    guard let data = context.data else { fail("no pixels for representation \(index)") }
    let bytes = data.bindMemory(to: UInt8.self, capacity: w * h * 4)

    var colors = Set<UInt32>()
    var greenTopLeft = 0
    var blueBottomRight = 0

    // Row 0 of a CGBitmapContext's buffer is the TOP row of the image, even
    // though CoreGraphics' drawing coordinate system has its origin at the
    // bottom left. Getting this backwards inverts the quadrants and reports
    // "no green bracket" on a correct icon -- checked against the same image
    // decoded by an independent decoder, which puts 1176 green-dominant
    // pixels in the top-left quadrant and 1308 blue-dominant in the
    // bottom-right.
    for y in 0..<h {
        for x in 0..<w {
            let p = (y * w + x) * 4
            let r = Int(bytes[p])
            let g = Int(bytes[p + 1])
            let b = Int(bytes[p + 2])
            colors.insert(UInt32(r) << 16 | UInt32(g) << 8 | UInt32(b))

            let isTop = y < h / 2
            let isLeft = x < w / 2
            // A margin of 12 is comfortably above channel noise in the glass
            // gradient and comfortably below a drawn bracket, which clears it
            // by an order of magnitude even after compositing.
            if g > r + 12 && g > b + 12 && isTop && isLeft { greenTopLeft += 1 }
            if b > r + 12 && b > g + 12 && !isTop && !isLeft { blueBottomRight += 1 }
        }
    }

    guard colors.count > 1 else {
        fail("representation \(index) (\(w)x\(h)) is a single flat colour")
    }
    let quadrant = Double((w / 2) * (h / 2))
    let greenShare = Double(greenTopLeft) / quadrant
    let blueShare = Double(blueBottomRight) / quadrant
    guard greenShare >= minimumShare else {
        fail(
            "representation \(index) (\(w)x\(h)) has no green bracket "
                + "(\(greenTopLeft) green-dominant pixels in the top-left quadrant)")
    }
    guard blueShare >= minimumShare else {
        fail(
            "representation \(index) (\(w)x\(h)) has no blue bracket "
                + "(\(blueBottomRight) blue-dominant pixels in the bottom-right quadrant)")
    }
    let greenPct = String(format: "%.1f", greenShare * 100)
    let bluePct = String(format: "%.1f", blueShare * 100)
    print("  \(w)x\(h): \(colors.count) colours, green \(greenPct)%, blue \(bluePct)%")
}

print("\(path) carries the mark at all \(count) representations")
