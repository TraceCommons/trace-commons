// Assert that an .icns actually contains the mark at every representation.
//
// Usage: verify-icns.swift <AppIcon.icns> <expected-representation-count>
//
// An icon that is the right size, the right filename and entirely blank passes
// every check anyone thought to write; that is how three solid squares of
// #315FBA shipped as the Windows tiles for as long as they did. So this looks
// at pixels: every representation must decode, must contain more than one
// colour, and must contain both bracket inks.
//
// It reads through CGImageSource deliberately. `iconutil --convert iconset`
// looks like the obvious way to inspect an .icns and is not: it stores the 16
// and 32 rungs as raw ARGB RLE (ic04/ic05) and reads them back with the blue
// channel zeroed, which looks precisely like a real corruption of the two
// smallest sizes. CGImageSource is what macOS uses to draw the icon, so it is
// what the check has to agree with.

import CoreGraphics
import Foundation
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FATAL: \(message)\n".utf8))
    exit(1)
}

let args = CommandLine.arguments
guard args.count == 3, let expected = Int(args[2]) else {
    fail("usage: verify-icns.swift <AppIcon.icns> <expected-representation-count>")
}
let path = args[1]

guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else {
    fail("cannot open \(path)")
}
let count = CGImageSourceGetCount(source)
guard count == expected else {
    fail("\(path) has \(count) representations, expected \(expected)")
}

// The two bracket inks, light scheme. If neither appears the icon is a blank
// card, which is the failure mode that matters.
//
// The names are historical: since the mark converged on the site palette the
// opening bracket is the accent teal and the closing one is plain ink. Ink is
// still worth probing for -- a blank card is white, so black pixels in the
// bottom-right are exactly what distinguishes a drawn mark from nothing.
let green: (UInt8, UInt8, UInt8) = (0x00, 0xD4, 0xAA)
let blue: (UInt8, UInt8, UInt8) = (0x00, 0x00, 0x00)

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
    var sawGreen = false
    var sawBlue = false
    // The small rungs are heavily antialiased, so an exact ink match is too
    // strict there; "closer to this ink than to anything else in the palette"
    // is what actually distinguishes a drawn bracket from a blank card.
    func near(_ r: UInt8, _ g: UInt8, _ b: UInt8, _ target: (UInt8, UInt8, UInt8)) -> Bool {
        let dr = Int(r) - Int(target.0)
        let dg = Int(g) - Int(target.1)
        let db = Int(b) - Int(target.2)
        return dr * dr + dg * dg + db * db < 2000
    }
    for pixel in 0..<(w * h) {
        let r = bytes[pixel * 4]
        let g = bytes[pixel * 4 + 1]
        let b = bytes[pixel * 4 + 2]
        colors.insert(UInt32(r) << 16 | UInt32(g) << 8 | UInt32(b))
        if near(r, g, b, green) { sawGreen = true }
        if near(r, g, b, blue) { sawBlue = true }
    }

    guard colors.count > 1 else {
        fail("representation \(index) (\(w)x\(h)) is a single flat colour")
    }
    guard sawGreen else { fail("representation \(index) (\(w)x\(h)) has no green bracket") }
    guard sawBlue else { fail("representation \(index) (\(w)x\(h)) has no blue bracket") }
    print("  \(w)x\(h): \(colors.count) colours, both brackets present")
}

print("\(path) carries the mark at all \(count) representations")
