import CoreGraphics
import SwiftUI

// The NEAR AI "N" mark, without the tile's rounded square: path data from
// the brand kit's `Tertiary/02_White & Color/tile-white-color.svg` (the
// blue path), in the tile's own 400 by 396 coordinates. Spaces are written
// as commas, as in GlassLogoPaths. GlassBrandMarkTests parses it.

/// The NEAR AI mark.
enum GlassBrandMark {
    /// The mark's own bounds inside the tile's coordinates.
    static let viewBox = CGRect(x: 82.96, y: 82.13, width: 234.08, height: 231.72)

    static let data =
        "M292.106,82.1357C283.445,82.1357,275.379,86.5848,270.847,93.9047L221.912,165.811C220.318,168.188,220.971,171.382,223.353,172.96C225.292,174.234,227.866,174.082,229.633,172.58L277.799,131.227C278.605,130.504,279.834,130.581,280.545,131.379C280.872,131.74,281.044,132.216,281.044,132.691V262.169C281.044,263.233,280.18,264.089,279.085,264.089C278.509,264.089,277.952,263.842,277.587,263.404L132.014,90.8817C127.271,85.3489,120.318,82.1548,112.982,82.1357H107.893C94.1229,82.1357,82.9648,93.1822,82.9648,106.814V289.167C82.9648,302.799,94.1229,313.846,107.893,313.846C116.554,313.846,124.62,309.397,129.153,302.077L178.087,230.17C179.681,227.793,179.028,224.599,176.646,223.021C174.707,221.747,172.133,221.899,170.366,223.401L122.2,264.754C121.394,265.477,120.165,265.401,119.454,264.602C119.128,264.241,118.955,263.766,118.955,263.29V133.775C118.955,132.71,119.819,131.854,120.914,131.854C121.49,131.854,122.047,132.102,122.412,132.539L267.966,305.1C272.709,310.632,279.662,313.827,286.998,313.846H292.087C305.857,313.846,317.034,302.799,317.034,289.167V106.833C317.034,93.2012,305.876,82.1548,292.106,82.1548V82.1357Z"

    /// Parsed once per process, moved so the mark's top-left is the origin
    /// and scaled to a unit square (aspect kept). Nil only if the data is
    /// malformed, which GlassBrandMarkTests rules out.
    static let unit: Path? = {
        guard let parsed = SVGPathData.parse(data) else { return nil }
        let scale = 1 / max(viewBox.width, viewBox.height)
        let transform = CGAffineTransform(scaleX: scale, y: scale)
            .translatedBy(x: -viewBox.minX, y: -viewBox.minY)
        return Path(parsed).applying(transform)
    }()

    /// The mark `size` points across with its top-left at `origin`.
    static func path(at origin: CGPoint, size: CGFloat) -> Path? {
        unit?.applying(CGAffineTransform(translationX: origin.x, y: origin.y).scaledBy(x: size, y: size))
    }
}
