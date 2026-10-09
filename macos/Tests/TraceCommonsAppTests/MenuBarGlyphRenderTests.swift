import AppKit
import SwiftUI
import XCTest

@testable import TCDesign
@testable import TraceCommonsApp

/// The status item can only be checked by looking at it, so this draws it
/// the way the screenshot hook does -- `ImageRenderer`, CPU-side, no window
/// -- and asserts the one thing a pixel buffer can prove: that a count
/// changes what is drawn. The item is the glass strip (`GlassMenuBarStrip`),
/// drawn under each `Condition` with and without a badge. The PNGs it writes
/// are for a person to look at; set `TRACE_COMMONS_MENUBAR_RENDER_DIR` to
/// keep them somewhere other than the temporary directory.
final class MenuBarGlyphRenderTests: XCTestCase {
    private struct Rendered {
        let png: Data
        /// Pixels that are not the background.
        let inked: Int
        /// How far, summed, those pixels are from the background: tells grey
        /// bars from coloured ones where the pixel count cannot.
        let weight: Int
    }

    /// Seven days with something on most of them, so the bars draw.
    private static let columns = (0..<7).map { GlassDayColumn(id: "d\($0)", up: $0 % 3, down: ($0 + 1) % 2) }

    @MainActor
    private func render(
        _ condition: GlassMenuBarStrip.Condition, badge: Int?, scheme: ColorScheme, scale: CGFloat
    ) -> Rendered? {
        // Padded past the badge's offset, so the capture holds the whole
        // item at the menu bar's 22pt.
        let renderer = ImageRenderer(
            content: GlassMenuBarStrip(columns: Self.columns, condition: condition, badge: badge)
                .padding(.horizontal, GlassTokens.Space.s4)
                .padding(.vertical, GlassTokens.Space.s1)
                .background(scheme == .dark ? Color.black : Color.white)
                .environment(\.colorScheme, scheme)
        )
        renderer.scale = scale
        guard let image = renderer.nsImage,
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else { return nil }
        // Ink is anything that is not the background: the bars are colour,
        // not black or white, so a brightness threshold would miss them.
        let background: CGFloat = scheme == .dark ? 0 : 1
        var inked = 0
        var weight: CGFloat = 0
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                guard let colour = rep.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                let distance = abs(colour.redComponent - background) + abs(colour.greenComponent - background)
                    + abs(colour.blueComponent - background)
                if distance > 0.15 {
                    inked += 1
                    weight += distance
                }
            }
        }
        return Rendered(png: png, inked: inked, weight: Int(weight * 100))
    }

    private var outputDirectory: URL {
        let env = ProcessInfo.processInfo.environment["TRACE_COMMONS_MENUBAR_RENDER_DIR"]
        if let env, !env.isEmpty { return URL(fileURLWithPath: env) }
        return FileManager.default.temporaryDirectory
    }

    @MainActor
    func testEveryStateRendersAndACountAddsInk() throws {
        let cases: [(String, GlassMenuBarStrip.Condition, Int?)] = [
            ("live", .live, nil),
            ("3", .live, 3),
            ("12", .live, 12),
            ("120", .live, 120),
            ("paused", .paused, nil),
            ("3-paused", .paused, 3),
            ("attention", .attention, nil),
            ("3-attention", .attention, 3),
            ("unavailable", .unavailable, nil),
            ("3-unavailable", .unavailable, 3),
        ]
        for scale in [CGFloat(1), 2] {
            for scheme in [ColorScheme.light, .dark] {
                var inked: [String: Int] = [:]
                var drawn: [String: Int] = [:]
                for (name, condition, badge) in cases {
                    let rendered = try XCTUnwrap(
                        render(condition, badge: badge, scheme: scheme, scale: scale), "\(name) did not render")
                    let suffix = scheme == .dark ? "-dark" : ""
                    let path = outputDirectory.appendingPathComponent("menubar-strip-\(name)\(suffix)-\(Int(scale))x.png")
                    try rendered.png.write(to: path)
                    XCTAssertGreaterThan(rendered.inked, 0, "\(name) draws nothing")
                    inked[name] = rendered.inked
                    drawn[name] = rendered.weight
                }
                let live = try XCTUnwrap(drawn["live"])
                for name in ["3", "12", "120", "paused", "attention", "unavailable"] {
                    XCTAssertNotEqual(drawn[name], live, "\(name) must draw something the live strip does not")
                }
                XCTAssertGreaterThan(try XCTUnwrap(inked["120"]), try XCTUnwrap(inked["12"]))
                XCTAssertNotEqual(drawn["attention"], drawn["paused"])
                XCTAssertNotEqual(drawn["3"], drawn["3-paused"])
                XCTAssertNotEqual(drawn["3-paused"], drawn["paused"], "a paused strip still shows the count")
                // Fail closed: an unavailable strip never draws a number.
                XCTAssertEqual(drawn["3-unavailable"], drawn["unavailable"], "an unavailable strip drew a count")
            }
        }
    }
}
