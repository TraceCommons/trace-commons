import CoreGraphics
import SwiftUI

// The NEAR AI primary lockup, the "near AI" wordmark: path data from the
// brand kit's `near-ai-primary-white-color.svg` (copied unchanged into the
// NEAR AI OS design system's `assets/logos`), in its own 730 by 138
// coordinates. "near" is four paths and "AI" two. Spaces are written as
// commas, as in GlassLogoPaths. GlassBrandMarkTests parses it.

/// The NEAR AI wordmark (owner, 2026-10-22: the menu-bar popover's top
/// left). The kit's inks: "near" white on a dark ground and black on a
/// light one (`menuItemMark`, the kit's single inks), "AI" the brand blue
/// in both. Never recoloured or redrawn (the kit's rules).
public struct GlassNearWordmark: View {
    public init() {}

    /// The artwork's bounds, and so its aspect.
    nonisolated public static let viewBox = CGRect(x: 0, y: 0, width: 730, height: 138)

    /// The kit's brand blue, "AI" in every colouring.
    static let brandBlue = GlassRGBA(0x0091FD)

    static let near = [
        "M74.3943,0.533711C59.8836,0.533711,49.3315,3.95853,40.3658,11.8567L24.5351,25.5477C23.215,26.6046,20.5749,27.3939,18.7322,25.8086C16.8844,24.2299,16.6214,22.1245,18.2028,20.0175L26.6458,7.37669C27.9642,5.53049,26.9122,3.16253,24.5351,3.16253H4.22152C1.84943,3.16253,0,5.00874,0,7.37669V130.624C0,132.992,1.84943,134.838,4.22152,134.838H25.3258C27.6979,134.838,29.5456,132.992,29.5456,130.624V61.102C29.5456,29.2384,56.1915,24.2366,66.2142,24.2366C87.5798,24.2366,95.2322,39.5112,95.2322,51.0951V130.624C95.2322,132.992,97.0799,134.838,99.452,134.838H120.556C122.928,134.838,124.778,132.992,124.778,130.624V48.4613C124.778,18.9656,105.516,0.533711,74.3892,0.533711H74.3943Z",
        "M210.771,0.00509583C169.882,0.00509583,143.766,25.0241,143.766,58.9948V77.6942C143.766,113.508,169.882,138,210.771,138C246.91,138,272.236,119.301,274.874,94.019C275.142,91.3852,273.293,89.544,270.654,89.544H250.078C248.23,89.544,246.648,90.6009,246.119,92.4404C243.479,100.867,231.084,113.508,210.771,113.508C190.457,113.508,171.463,98.7616,171.725,77.6942L171.993,54.2572C172.254,36.6147,190.723,24.5023,210.771,24.5023C228.973,24.5023,246.648,34.7752,248.491,51.6284C248.644,53.5766,247.302,55.3358,245.377,55.7054L186.23,67.1655C183.858,67.694,182.011,69.7994,182.011,72.4299V72.6908C182.011,75.0587,184.383,77.1658,187.812,77.1658H272.758C275.092,77.1658,276.98,75.2761,276.98,72.9516V56.3643C276.98,25.0308,249.806,0.0101099,210.766,0.0101099L210.771,0.00509583Z",
        "M357.965,0.00483262C324.989,0.00483262,296.5,19.226,296.5,44.5076C296.5,46.6146,298.35,48.1933,300.722,48.1933H322.087C324.198,48.1933,325.779,46.6146,326.048,44.5076C328.158,32.9237,342.14,24.4971,357.175,24.4971C375.111,24.4971,387.25,35.5575,387.25,54.5195V77.4314C387.25,100.867,369.837,112.718,348.204,112.718C331.321,112.718,321.56,106.399,321.56,96.1258C321.56,87.1708,326.309,79.5335,345.827,75.0585L374.052,67.4211C376.954,66.6318,378.012,64.2572,377.483,61.6284C377.222,59.5213,374.848,58.4661,372.732,58.4661H343.455C318.658,58.4661,293.594,74.2641,293.594,97.4369V101.123C293.594,124.826,316.018,137.199,341.605,137.199C357.96,137.199,371.941,130.879,380.646,123.508L393.575,112.446C395.686,110.601,397.797,110.601,399.639,112.446C401.221,114.024,400.691,116.399,399.373,118.238L391.459,130.618C390.139,132.463,391.193,134.831,393.57,134.831H412.564C414.936,134.831,416.785,132.986,416.785,130.618V50.8221C416.785,20.2779,394.89,-0.00018147,357.955,-0.00018147L357.965,0.00483262Z",
        "M530.748,3.16809H501.203C490.917,3.16809,480.889,9.48764,473.766,15.548L462.162,25.5532C460.842,26.6085,458.465,27.3978,456.883,26.0817C455.034,24.7639,454.243,22.1284,455.83,20.0213L464.273,7.38223C465.591,5.53604,464.539,3.16809,462.162,3.16809H442.378C440.006,3.16809,438.156,5.01428,438.156,7.38223V130.629C438.156,132.997,440.006,134.842,442.378,134.842H464.01C466.383,134.842,468.231,132.997,468.231,130.629V67.4271C468.231,40.2994,479.313,28.1871,503.318,28.1871H530.753C533.125,28.1871,534.975,26.3426,534.975,23.9746V7.38223C534.975,5.01428,533.125,3.16809,530.753,3.16809H530.748Z",
    ]

    static let ai = [
        "M612.823,84.5346H650.363C651.388,84.5346,651.854,83.976,651.574,82.9543L632.571,26.881C632.105,25.672,631.082,25.672,630.71,26.881L611.706,82.9543C611.333,83.976,611.798,84.5346,612.823,84.5346ZM649.06,6.3287L695.078,130.193C695.915,132.703,694.332,134.842,691.816,134.842H672.721C670.95,134.842,669.553,133.912,668.995,132.24L659.307,104.9C659.026,104.155,658.376,103.784,657.63,103.784H605.556C604.811,103.784,604.161,104.155,603.88,104.9L594.286,132.052C593.634,133.912,592.329,134.842,590.466,134.842H571.463C568.854,134.842,567.271,132.611,568.203,130.193L614.127,6.3287C614.966,4.19153,616.549,3.16809,618.786,3.16809H644.401C646.638,3.16809,648.314,4.19153,649.06,6.3287Z",
        "M708.016,3.16809H726.368C728.417,3.16809,730,4.7484,730,6.7936V131.216C730,133.261,728.417,134.842,726.368,134.842H708.016C705.968,134.842,704.383,133.261,704.383,131.216V6.7936C704.383,4.7484,705.968,3.16809,708.016,3.16809Z",
    ]

    /// One part of the wordmark parsed once, in the artwork's coordinates.
    /// Nil only if the data is malformed, which GlassBrandMarkTests rules
    /// out.
    static func parsed(_ parts: [String]) -> Path? {
        let path = CGMutablePath()
        for part in parts {
            guard let parsed = SVGPathData.parse(part) else { return nil }
            path.addPath(parsed)
        }
        return Path(path)
    }

    static let nearPath = parsed(near)
    static let aiPath = parsed(ai)

    public var body: some View {
        ZStack {
            Part(path: Self.nearPath).fill(GlassTokens.Color.menuItemMark.color)
            Part(path: Self.aiPath).fill(Self.brandBlue.color)
        }
        .aspectRatio(Self.viewBox.width / Self.viewBox.height, contentMode: .fit)
        .accessibilityHidden(true)
    }

    /// A part of the wordmark scaled into the frame the whole takes.
    private struct Part: Shape {
        let path: Path?

        func path(in rect: CGRect) -> Path {
            guard let path else { return Path() }
            let box = GlassNearWordmark.viewBox
            let scale = min(rect.width / box.width, rect.height / box.height)
            let transform = CGAffineTransform(translationX: rect.minX, y: rect.minY).scaledBy(x: scale, y: scale)
            return path.applying(transform)
        }
    }
}

/// The NEAR AI "N" mark alone in the brand blue, as the logo kit's color
/// tile draws its N (owner, 2026-10-22: the edge rail's way into the app,
/// in place of the app icon). Square; the mark fills it.
public struct GlassNearMark: View {
    public init() {}

    public var body: some View {
        Canvas { context, size in
            let side = min(size.width, size.height)
            let origin = CGPoint(x: (size.width - side) / 2, y: (size.height - side) / 2)
            guard let mark = GlassBrandMark.path(at: origin, size: side) else { return }
            context.fill(mark, with: .color(GlassNearWordmark.brandBlue.color))
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }
}
