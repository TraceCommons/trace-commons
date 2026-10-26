import CoreGraphics
import Foundation
import ImageIO

// Assert that a compiled Icon Composer icon carries the NEAR AI "N" on the
// tile, and that only a staging icon carries the staging ribbon.
//
// Usage: verify-icon.swift <icns> <expected-representations> <production|staging>
//
// # Why classes of colour, not exact inks
//
// macOS composites a .icon as glass: it lightens, adds a specular rim and lets
// the tile show through the N, so #F3F3F2 arrives as anything from white to a
// pale blue and no exact-ink match survives. So each pixel is sorted into a
// class the glass treatment cannot erase -- blue-dominant tile, bright paper,
// amber ribbon -- and the classes are checked where the mark puts them.
//
// This fails the things that matter: a blank or flat icon, a missing N, an N
// with half of it gone, an N in the tile's colour, a production icon that
// ships the staging ribbon, and a staging icon that lost it.

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FATAL: \(message)\n".utf8))
    exit(1)
}

let args = CommandLine.arguments
guard args.count == 4, let expected = Int(args[2]), ["production", "staging"].contains(args[3]) else {
    fail("usage: verify-icon.swift <icns> <expected-representations> <production|staging>")
}
let path = args[1]
let staging = args[3] == "staging"

guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else {
    fail("\(path) does not open as an image source")
}
let count = CGImageSourceGetCount(source)
guard count == expected else {
    fail("\(path) has \(count) representations, expected \(expected)")
}

for index in 0..<count {
    guard let image = CGImageSourceCreateImageAtIndex(source, index, nil) else {
        fail("representation \(index) does not decode")
    }
    let w = image.width
    let h = image.height
    guard
        let context = CGContext(
            data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { fail("cannot rasterize representation \(index)") }
    context.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    guard let data = context.data else { fail("no pixels for representation \(index)") }
    let bytes = data.bindMemory(to: UInt8.self, capacity: w * h * 4)

    var colors = Set<UInt32>()
    var opaque = 0
    var tile = 0
    var paper = 0
    var paperLeft = 0
    var paperRight = 0
    var opaqueLeft = 0
    var opaqueRight = 0
    var paperCentre = 0
    var centre = 0
    var amber = 0
    var amberTopRight = 0

    // Row 0 of a CGBitmapContext's buffer is the TOP row of the image, even
    // though CoreGraphics' drawing coordinates start at the bottom left.
    // Getting this backwards puts the ribbon in the wrong quadrant.
    for y in 0..<h {
        for x in 0..<w {
            let p = (y * w + x) * 4
            let a = Int(bytes[p + 3])
            // The legacy renditions carry a transparent margin and a soft
            // shadow around the tile. Neither is the icon.
            guard a >= 192 else { continue }
            // Un-premultiply, so a pixel at the tile's antialiased edge is
            // judged by its colour, not darkened by its coverage.
            let r = Int(bytes[p]) * 255 / a
            let g = Int(bytes[p + 1]) * 255 / a
            let b = Int(bytes[p + 2]) * 255 / a
            colors.insert(UInt32(r) << 16 | UInt32(g) << 8 | UInt32(b))
            opaque += 1

            let isLeft = x < w / 2
            let isTop = y < h / 2
            if isLeft { opaqueLeft += 1 } else { opaqueRight += 1 }
            // The diagonal of the N crosses the middle of the canvas.
            let inCentre = abs(2 * x - w) < w / 5 && abs(2 * y - h) < h / 5
            if inCentre { centre += 1 }

            if b > r + 60 && b > g + 20 && r < 140 {
                tile += 1
            } else if min(r, min(g, b)) >= 185 && b - r <= 70 {
                paper += 1
                if isLeft { paperLeft += 1 } else { paperRight += 1 }
                if inCentre { paperCentre += 1 }
            } else if r > 200 && g > 110 && g < 215 && b < 100 {
                amber += 1
                if isTop && !isLeft { amberTopRight += 1 }
            }
        }
    }

    let label = "representation \(index) (\(w)x\(h))"
    guard colors.count > 1 else { fail("\(label) is a single flat colour") }
    guard opaque > 0 else { fail("\(label) is fully transparent") }
    let share = { (n: Int, of: Int) in of == 0 ? 0 : Double(n) / Double(of) }
    let tileShare = share(tile, opaque)
    let paperShare = share(paper, opaque)
    let leftShare = share(paperLeft, opaqueLeft)
    let rightShare = share(paperRight, opaqueRight)
    let centreShare = share(paperCentre, centre)
    let amberShare = share(amber, opaque)

    // At 16px a stroke of the N is about one pixel wide and blends into the
    // tile, so most of it no longer reads as paper. Measured on the shipped
    // icon: paper is 15-18% of the opaque pixels from 32px up and 6% at 16px,
    // and the ribbon's amber is 1-3% from 32px up and gone at 16px. The floors
    // sit well under those numbers and well over a missing mark's zero.
    let small = w < 32

    // The tile is most of the icon; the N is a real share of it but not all
    // of it. An N scaled to nothing, or blown up to cover the tile, fails.
    guard tileShare >= 0.40 else {
        fail("\(label) has no blue tile (\(pct(tileShare)) of opaque pixels are tile blue)")
    }
    guard paperShare >= (small ? 0.03 : 0.10) && paperShare <= 0.45 else {
        fail("\(label) has no N of the right size (\(pct(paperShare)) of opaque pixels are paper)")
    }
    // The N is symmetric under a half turn, so each half carries a stem and
    // half of the diagonal. One half bare means half of the mark is gone.
    let halfFloor = small ? 0.03 : 0.08
    guard leftShare >= halfFloor && rightShare >= halfFloor else {
        fail("\(label) has half an N (left \(pct(leftShare)), right \(pct(rightShare)) paper)")
    }
    guard centreShare >= 0.10 else {
        fail("\(label) has no diagonal through the middle (\(pct(centreShare)) paper)")
    }
    if staging && !small {
        guard share(amberTopRight, opaque) >= 0.01 else {
            fail("\(label) is a staging icon without the amber ribbon in the top-right corner")
        }
        guard amberTopRight * 10 >= amber * 9 else {
            fail("\(label) has amber outside the top-right corner")
        }
    } else if !staging {
        guard amberShare < 0.002 else {
            fail("\(label) is a production icon carrying staging amber (\(pct(amberShare)))")
        }
    }
    print(
        "  \(w)x\(h): tile \(pct(tileShare)), paper \(pct(paperShare)) "
            + "(left \(pct(leftShare)), right \(pct(rightShare)), centre \(pct(centreShare))), "
            + "amber \(pct(amberShare))")
}

func pct(_ value: Double) -> String { String(format: "%.1f%%", value * 100) }

print("\(path) carries the \(args[3]) icon at all \(count) representations")
