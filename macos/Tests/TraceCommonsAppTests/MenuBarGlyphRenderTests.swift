import AppKit
import SwiftUI
import XCTest

@testable import TCShellCore
@testable import TraceCommonsApp

/// The status item can only be checked by looking at it, so this draws it
/// the way the screenshot hook does -- `ImageRenderer`, CPU-side, no window
/// -- and asserts the one thing a pixel buffer can prove: that a count
/// changes what is drawn. The PNGs it writes are for a person to look at;
/// set `TRACE_COMMONS_MENUBAR_RENDER_DIR` to keep them somewhere other than
/// the temporary directory.
final class MenuBarGlyphRenderTests: XCTestCase {
    private struct Rendered {
        let png: Data
        let inked: Int
    }

    @MainActor
    private func render(_ state: MenuBarState, scheme: ColorScheme, scale: CGFloat) -> Rendered? {
        // A 22pt band, the menu bar's own height, so the capture shows the
        // item at the vertical room it actually gets.
        let renderer = ImageRenderer(
            content: MenuBarGlyph(state: state)
                .frame(height: 22)
                .padding(.horizontal, TC.Space.xs)
                .background(scheme == .dark ? Color.black : Color.white)
                .environment(\.colorScheme, scheme)
        )
        renderer.scale = scale
        guard let image = renderer.nsImage,
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else { return nil }
        var inked = 0
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                guard let colour = rep.colorAt(x: x, y: y) else { continue }
                let luminance = colour.usingColorSpace(.deviceRGB)?.brightnessComponent ?? 0
                if scheme == .dark ? luminance > 0.5 : luminance < 0.5 { inked += 1 }
            }
        }
        return Rendered(png: png, inked: inked)
    }

    private var outputDirectory: URL {
        let env = ProcessInfo.processInfo.environment["TRACE_COMMONS_MENUBAR_RENDER_DIR"]
        if let env, !env.isEmpty { return URL(fileURLWithPath: env) }
        return FileManager.default.temporaryDirectory
    }

    @MainActor
    func testEveryStateRendersAndACountAddsInk() throws {
        let cases: [(String, MenuBarState)] = [
            ("0", .idle),
            ("3", .count("3")),
            ("12", .count("12")),
            ("3-paused", .count("3", paused: true)),
            ("120", .count(try XCTUnwrap(MenuBarStatus.badgeText(decisionsOwed: 120)))),
            ("paused", .paused),
            ("health", .attention),
        ]
        var inked: [String: Int] = [:]
        for scale in [CGFloat(1), 2] {
            for scheme in [ColorScheme.light, .dark] {
                for (name, state) in cases {
                    let rendered = try XCTUnwrap(render(state, scheme: scheme, scale: scale), "\(name) did not render")
                    let suffix = scheme == .dark ? "-dark" : ""
                    let path = outputDirectory.appendingPathComponent("menubar-label-\(name)\(suffix)-\(Int(scale))x.png")
                    try rendered.png.write(to: path)
                    inked[name] = rendered.inked
                }
                let idle = try XCTUnwrap(inked["0"])
                XCTAssertGreaterThan(idle, 0, "the mark itself must draw")
                for name in ["3", "12", "120", "paused", "health"] {
                    XCTAssertNotEqual(inked[name], idle, "\(name) must draw something the idle mark does not")
                }
                XCTAssertGreaterThan(try XCTUnwrap(inked["120"]), try XCTUnwrap(inked["12"]))
                XCTAssertNotEqual(inked["health"], inked["paused"])
                XCTAssertNotEqual(inked["3"], inked["3-paused"])
            }
        }
    }
}
