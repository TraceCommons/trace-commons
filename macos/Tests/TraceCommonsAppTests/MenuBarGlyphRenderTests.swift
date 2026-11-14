import AppKit
import SwiftUI
import XCTest

@testable import TCDesign
@testable import TraceCommonsApp

/// The status item can only be checked by looking at it, so this draws it
/// the way the screenshot hook does -- `ImageRenderer`, CPU-side, no window
/// -- and checks its sizing, ink, and transparent badge figures. The item
/// is the glass strip (`GlassMenuBarStrip`):
/// the NEAR AI mark and stacked up-and-down bars, drawn under each
/// `Condition` with and without a badge and the Private AI segments. The
/// PNGs it writes
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

    /// Six steps with something on most of them, so the trace bars draw.
    private static let columns = (0..<6).map { GlassDayColumn(id: "s\($0)", up: $0 % 3, down: ($0 + 1) % 2) }
    /// Private AI steps: verified calls up, all calls down.
    private static let privateAI = (0..<6).map { GlassDayColumn(id: "s\($0)", up: $0 % 4, down: $0 % 4 + 2) }

    @MainActor
    private func render(
        _ condition: GlassMenuBarStrip.Condition, badge: Int?, mark: GlassMenuBarStrip.Mark = .none,
        traces: [GlassDayColumn] = columns, privateAI: [GlassDayColumn]? = privateAI,
        scheme: ColorScheme, scale: CGFloat
    ) -> Rendered? {
        // Padded past the badge's offset, so the capture holds the whole
        // item at the menu bar's 22pt.
        let renderer = ImageRenderer(
            content: GlassMenuBarStrip(
                traces: traces, privateAI: privateAI, condition: condition, badge: badge, mark: mark)
                .padding(.horizontal, GlassTokens.Space.s4)
                .padding(.vertical, GlassTokens.Space.s1)
                .background(scheme == .dark ? Color.black : Color.white)
                .environment(\.colorScheme, scheme)
        )
        guard let rep = Self.draw(renderer, scale: scale),
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

    /// Draws into a bitmap this test owns (sRGB, 8 bits a channel), rather
    /// than through `nsImage`, whose shared drawing state another test's
    /// `ImageRenderer` drawing leaves behind.
    @MainActor
    private static func draw<Content: View>(_ renderer: ImageRenderer<Content>, scale: CGFloat) -> NSBitmapImageRep? {
        var bitmap: NSBitmapImageRep?
        renderer.render(rasterizationScale: scale) { size, paint in
            let width = Int((size.width * scale).rounded(.up)), height = Int((size.height * scale).rounded(.up))
            guard width > 0, height > 0,
                  let space = CGColorSpace(name: CGColorSpace.sRGB),
                  let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                          space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return }
            context.scaleBy(x: scale, y: scale)
            paint(context)
            bitmap = context.makeImage().map(NSBitmapImageRep.init(cgImage:))
        }
        return bitmap
    }

    private var outputDirectory: URL {
        let env = ProcessInfo.processInfo.environment["TRACE_COMMONS_MENUBAR_RENDER_DIR"]
        if let env, !env.isEmpty { return URL(fileURLWithPath: env) }
        return FileManager.default.temporaryDirectory
    }

    /// The item alone, unpadded, on a ground of grey level `ground`
    /// (black or white, the bar's own, by default), as a bitmap.
    @MainActor
    private func bitmap(
        traces: [GlassDayColumn], privateAI: [GlassDayColumn]?, scheme: ColorScheme, ground: CGFloat? = nil
    ) throws -> NSBitmapImageRep {
        let level = ground ?? (scheme == .dark ? 0 : 1)
        let renderer = ImageRenderer(
            content: GlassMenuBarStrip(traces: traces, privateAI: privateAI, condition: .live, badge: nil)
                .background(Color(.sRGB, red: level, green: level, blue: level))
                .environment(\.colorScheme, scheme)
        )
        renderer.scale = 2
        let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation)
        return try XCTUnwrap(NSBitmapImageRep(data: tiff))
    }

    /// The Private AI segments add ink while they are known, also while
    /// paused, and none while unknown or on an unavailable strip.
    @MainActor
    func testThePrivateAISegmentsDrawOnlyWhenKnown() throws {
        for scheme in [ColorScheme.light, .dark] {
            func weight(_ condition: GlassMenuBarStrip.Condition, _ privateAI: [GlassDayColumn]?) throws -> Int {
                try XCTUnwrap(render(condition, badge: nil, privateAI: privateAI, scheme: scheme, scale: 2)).weight
            }
            XCTAssertNotEqual(try weight(.live, Self.privateAI), try weight(.live, nil), "the Private AI segments")
            XCTAssertNotEqual(try weight(.paused, Self.privateAI), try weight(.paused, nil), "pausing keeps them")
            XCTAssertEqual(
                try weight(.unavailable, Self.privateAI), try weight(.unavailable, nil),
                "an unavailable strip drew Private AI segments")
        }
    }

    /// A count past 99 draws as 99+, whatever the count (owner, 2026-10-21).
    /// The two drawings of 99+ match byte for byte.
    @MainActor
    func testACountPastNinetyNineDrawsNinetyNinePlus() throws {
        XCTAssertEqual(GlassMenuBarStrip.badgeText(100), "99+")
        XCTAssertEqual(GlassMenuBarStrip.badgeText(4_321), "99+")
        XCTAssertEqual(GlassMenuBarStrip.badgeText(99), "99")
        for scheme in [ColorScheme.light, .dark] {
            let hundred = try XCTUnwrap(render(.live, badge: 100, scheme: scheme, scale: 2))
            let thousands = try XCTUnwrap(render(.live, badge: 4_321, scheme: scheme, scale: 2))
            let ninetyNine = try XCTUnwrap(render(.live, badge: 99, scheme: scheme, scale: 2))
            XCTAssertEqual(hundred.weight, thousands.weight)
            XCTAssertEqual(hundred.png, thousands.png)
            XCTAssertNotEqual(hundred.weight, ninetyNine.weight)
        }
    }

    /// No capsule behind the bars (owner, 2026-10-21): with nothing to
    /// draw, the graph's upper reach is the ground it is drawn on, in both
    /// appearances. The ground is mid-grey, since a dark tint on black or a
    /// light one on white would leave the ground unchanged. The mark is the
    /// logo kit's single ink for the bar: white on a dark one, black on a
    /// light one.
    @MainActor
    func testNoCapsuleAndTheMarkTakesTheBarsInk() throws {
        let flat = (0..<6).map { GlassDayColumn(id: "s\($0)", up: 0, down: 0) }
        func distance(_ rep: NSBitmapImageRep, _ x: CGFloat, _ y: CGFloat, from value: CGFloat) throws -> CGFloat {
            let colour = try XCTUnwrap(rep.colorAt(x: Int(x * 2), y: Int(y * 2))?.usingColorSpace(.deviceRGB))
            return abs(colour.redComponent - value) + abs(colour.greenComponent - value)
                + abs(colour.blueComponent - value)
        }
        for scheme in [ColorScheme.light, .dark] {
            // Halfway along the graph, 3pt below its top edge.
            let graphMid = GlassMenuBarStrip.markSize + GlassMenuBarStrip.markGap
                + GlassMenuBarStrip.graphWidth(steps: 6) / 2
            let graphTop = GlassMenuBarStrip.graphTop
            let grey = try bitmap(traces: flat, privateAI: flat, scheme: scheme, ground: 0.5)
            let probe = try XCTUnwrap(grey.colorAt(x: Int(graphMid * 2), y: Int((graphTop + 3) * 2))?
                .usingColorSpace(.deviceRGB))
            let level = probe.redComponent
            XCTAssertLessThan(
                abs(probe.greenComponent - level) + abs(probe.blueComponent - level), 0.02, "\(scheme): a tint")
            let ground = try XCTUnwrap(grey.colorAt(x: 0, y: 0)?.usingColorSpace(.deviceRGB)).redComponent
            XCTAssertEqual(level, ground, accuracy: 0.01, "\(scheme): a fill behind the lines")

            let rep = try bitmap(traces: flat, privateAI: flat, scheme: scheme)
            let ink: CGFloat = scheme == .dark ? 1 : 0
            // Halfway up the N's left upright, which spans its first 15%.
            XCTAssertLessThan(
                try distance(rep, GlassMenuBarStrip.markSize * 0.077, 11, from: ink), 0.1, "\(scheme): the mark's ink")
        }
    }

    /// The news ring and the halo each change what is drawn, in both
    /// appearances; a mark with no slot to draw in changes nothing.
    @MainActor
    func testTheNudgeMarkAddsInkOnlyWhereItIsDrawn() throws {
        for scheme in [ColorScheme.light, .dark] {
            func weight(_ condition: GlassMenuBarStrip.Condition, _ badge: Int?, _ mark: GlassMenuBarStrip.Mark) throws -> Int {
                try XCTUnwrap(render(condition, badge: badge, mark: mark, scheme: scheme, scale: 2)).weight
            }
            XCTAssertNotEqual(try weight(.live, nil, .news), try weight(.live, nil, .none), "news ring")
            XCTAssertNotEqual(try weight(.live, 3, .ready), try weight(.live, 3, .none), "halo")
            XCTAssertEqual(try weight(.live, 3, .news), try weight(.live, 3, .none), "news under a badge")
            XCTAssertEqual(try weight(.unavailable, nil, .news), try weight(.unavailable, nil, .none))
        }
    }

    /// The item with a badge and every bar at full reach, padded on the
    /// trailing side as the app pads it, plus `outset` all round, on black.
    @MainActor
    private func badged(_ mark: GlassMenuBarStrip.Mark, outset: CGFloat) throws -> NSBitmapImageRep {
        let full = (0..<6).map { GlassDayColumn(id: "s\($0)", up: 4, down: 4) }
        let renderer = ImageRenderer(
            content: GlassMenuBarStrip(traces: full, privateAI: full, condition: .live, badge: 3, mark: mark)
                .padding(.trailing, GlassMenuBarStrip.haloReach)
                .padding(outset)
                .background(Color.black)
                .environment(\.colorScheme, .dark)
        )
        renderer.scale = 2
        let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation)
        return try XCTUnwrap(NSBitmapImageRep(data: tiff))
    }

    /// Summed distance from black over the pixels with `x` below `width`.
    private func ink(_ rep: NSBitmapImageRep, width: Int? = nil) -> CGFloat {
        var total: CGFloat = 0
        for y in 0..<rep.pixelsHigh {
            for x in 0..<min(width ?? rep.pixelsWide, rep.pixelsWide) {
                guard let colour = rep.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                total += colour.redComponent + colour.greenComponent + colour.blueComponent
            }
        }
        return total
    }

    /// The halo fits inside the item's image, with nothing cut off at its
    /// bottom or trailing edge: it draws the same with room all round.
    @MainActor
    func testTheHaloIsNotClipped() throws {
        let tight = try ink(badged(.ready, outset: 0))
        let roomy = try ink(badged(.ready, outset: 8))
        XCTAssertEqual(tight, roomy, accuracy: 0.5, "the halo is cut off at the item's edge")
    }

    /// The halo never hides the newest step: the graph draws the same with
    /// and without it.
    @MainActor
    func testTheHaloLeavesTheBarsUncovered() throws {
        let graph = Int(
            (GlassMenuBarStrip.markSize + GlassMenuBarStrip.markGap + GlassMenuBarStrip.graphWidth(steps: 6)) * 2)
        let halo = try badged(.ready, outset: 0)
        let plain = try badged(.none, outset: 0)
        XCTAssertEqual(ink(halo, width: graph), ink(plain, width: graph), accuracy: 0.01, "the halo covers the bars")
    }

    /// The item's image draws its dark rendering on a dark menu bar and its
    /// light one on a light bar, from the appearance it is drawn in, not
    /// the app's; one image serves both, as the item keeps one. It is not a
    /// template, so the bars keep their colours.
    @MainActor
    func testTheItemImageDrawsInTheBarsAppearance() throws {
        func solid(_ colour: NSColor) -> NSImage {
            NSImage(size: NSSize(width: 4, height: 4), flipped: false) { rect in
                colour.setFill()
                rect.fill()
                return true
            }
        }
        let image = MenuBarItemImage.adaptive(dark: solid(.red), light: solid(.blue))
        XCTAssertFalse(image.isTemplate)
        let cases: [(NSAppearance.Name, Bool)] = [
            (.darkAqua, true), (.aqua, false), (.vibrantDark, true), (.vibrantLight, false),
            (.accessibilityHighContrastDarkAqua, true), (.accessibilityHighContrastAqua, false),
            (.darkAqua, true),
        ]
        for (name, dark) in cases {
            let appearance = try XCTUnwrap(NSAppearance(named: name))
            XCTAssertEqual(MenuBarItemImage.isDark(appearance), dark, "\(name.rawValue)")
            let rep = try XCTUnwrap(NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: 4, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4,
                hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            appearance.performAsCurrentDrawingAppearance {
                image.draw(in: NSRect(x: 0, y: 0, width: 4, height: 4))
            }
            NSGraphicsContext.restoreGraphicsState()
            let colour = try XCTUnwrap(rep.colorAt(x: 2, y: 2)?.usingColorSpace(.deviceRGB))
            if dark {
                XCTAssertGreaterThan(colour.redComponent, colour.blueComponent + 0.5, "\(name.rawValue) drew the light image")
            } else {
                XCTAssertGreaterThan(colour.blueComponent, colour.redComponent + 0.5, "\(name.rawValue) drew the dark image")
            }
        }
    }

    /// The app's exact bounds, with optional extra room to expose clipping.
    /// Keep the background transparent unless testing the blend's isolation.
    @MainActor
    private func stripBitmap(
        condition: GlassMenuBarStrip.Condition = .live, badge: Int?, mark: GlassMenuBarStrip.Mark = .none,
        graph: Bool, scheme: ColorScheme, scale: CGFloat, empty: Bool = false,
        outset: CGFloat = 0, background: Color = .clear
    ) throws -> NSBitmapImageRep {
        let renderer = ImageRenderer(content: GlassMenuBarStrip(
            traces: empty ? [] : Self.columns, privateAI: empty ? nil : Self.privateAI,
            condition: condition, badge: badge, mark: mark, graph: graph)
            .padding(.trailing, GlassMenuBarStrip.haloReach)
            .padding(outset)
            .background(background)
            .environment(\.colorScheme, scheme))
        return try XCTUnwrap(Self.draw(renderer, scale: scale))
    }

    private func alpha(_ rep: NSBitmapImageRep, x: Int, y: Int) -> CGFloat {
        rep.colorAt(x: x, y: y)?.alphaComponent ?? 0
    }

    private func alphaSum(_ rep: NSBitmapImageRep) -> CGFloat {
        (0..<rep.pixelsHigh).reduce(CGFloat(0)) { total, y in
            total + (0..<rep.pixelsWide).reduce(CGFloat(0)) { $0 + alpha(rep, x: $1, y: y) }
        }
    }

    @MainActor
    func testMarkOnlyStatesFitWithoutCoveringTheLogoOrDrawingData() throws {
        let conditions: [GlassMenuBarStrip.Condition] = [.live, .paused, .attention, .unavailable, .loading]
        let slots: [(Int?, GlassMenuBarStrip.Mark)] = [
            (nil, .none), (0, .none), (3, .none), (12, .none), (120, .none), (nil, .news), (3, .ready),
        ]
        for scale in [CGFloat(1), 2] {
            for scheme in [ColorScheme.light, .dark] {
                let logo = try stripBitmap(badge: nil, graph: false, scheme: scheme, scale: scale)
                XCTAssertEqual(logo.pixelsWide, Int(((14 + 3.5) * scale).rounded(.up)))
                for condition in conditions {
                    let noBadge = try stripBitmap(
                        condition: condition, badge: nil, graph: false, scheme: scheme, scale: scale)
                    for (badge, mark) in slots {
                        let label = "\(condition), \(String(describing: badge)), \(mark), \(scheme), \(scale)x"
                        let tight = try stripBitmap(
                            condition: condition, badge: badge, mark: mark, graph: false, scheme: scheme, scale: scale)
                        let empty = try stripBitmap(
                            condition: condition, badge: badge, mark: mark, graph: false,
                            scheme: scheme, scale: scale, empty: true)
                        XCTAssertEqual(tight.pixelsHigh, Int(22 * scale), label)
                        XCTAssertEqual(
                            tight.representation(using: .png, properties: [:]),
                            empty.representation(using: .png, properties: [:]), "hidden data changed the strip: \(label)")
                        if condition == .unavailable || condition == .loading || (badge == 0 && mark == .none) {
                            XCTAssertEqual(
                                tight.representation(using: .png, properties: [:]),
                                noBadge.representation(using: .png, properties: [:]), "a hidden badge drew ink: \(label)")
                        }
                        let roomy = try stripBitmap(
                            condition: condition, badge: badge, mark: mark, graph: false,
                            scheme: scheme, scale: scale, outset: 8)
                        XCTAssertEqual(alphaSum(tight), alphaSum(roomy), accuracy: 0.5, "clipped slot: \(label)")
                        // Every slot, including the ready halo, leaves the
                        // mark's pixels intact and a clear point after it.
                        var logoDifference: CGFloat = 0
                        var gapInk: CGFloat = 0
                        for y in 0..<tight.pixelsHigh {
                            for x in 0..<Int(14 * scale) {
                                logoDifference += abs(alpha(tight, x: x, y: y) - alpha(logo, x: x, y: y))
                            }
                            gapInk += alpha(tight, x: Int(15 * scale), y: y)
                        }
                        XCTAssertEqual(logoDifference, 0, accuracy: 0.01, "slot covers the mark: \(label)")
                        XCTAssertEqual(gapInk, 0, accuracy: 0.01, "slot closes the mark's gap: \(label)")
                        let graphed = try stripBitmap(
                            condition: condition, badge: badge, mark: mark, graph: true, scheme: scheme, scale: scale)
                        // Six pairs of 1pt bars, five 1pt gaps, and the
                        // 4pt mark-to-graph gap; a slot has 2pt less gap.
                        let hasSlot = ((badge ?? 0) > 0 && condition != .unavailable && condition != .loading)
                            || condition == .attention || condition == .unavailable
                            || (mark == .news && condition == .live && (badge ?? 0) == 0)
                        XCTAssertEqual(graphed.pixelsWide - tight.pixelsWide, Int((hasSlot ? 19 : 21) * scale), label)
                    }
                }
            }
        }
    }

    /// Transparent pixels surrounded by opaque ink cannot be the capsule's
    /// rounded corners or the empty space around it: they are its figure.
    private func assertCutOutFigure(
        _ rep: NSBitmapImageRep, graph: Bool, scheme: ColorScheme, scale: CGFloat,
        file: StaticString = #filePath, line: UInt = #line
    ) throws {
        let start = Int((graph ? 35 : 16) * scale)
        let end = rep.pixelsWide - Int((3.5 * scale).rounded(.up))
        let ink: CGFloat = scheme == .dark ? 1 : 0
        var opaque = 0
        var holes = 0
        for y in 0..<rep.pixelsHigh {
            for x in start..<end {
                let colour = try XCTUnwrap(rep.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB), file: file, line: line)
                if colour.alphaComponent > 0.95 {
                    opaque += 1
                    XCTAssertEqual(colour.redComponent, ink, accuracy: 0.02, file: file, line: line)
                    XCTAssertEqual(colour.greenComponent, ink, accuracy: 0.02, file: file, line: line)
                    XCTAssertEqual(colour.blueComponent, ink, accuracy: 0.02, file: file, line: line)
                } else if colour.alphaComponent < 0.65,
                          (start..<x).contains(where: { alpha(rep, x: $0, y: y) > 0.95 }),
                          ((x + 1)..<end).contains(where: { alpha(rep, x: $0, y: y) > 0.95 }),
                          (0..<y).contains(where: { alpha(rep, x: x, y: $0) > 0.95 }),
                          ((y + 1)..<rep.pixelsHigh).contains(where: { alpha(rep, x: x, y: $0) > 0.95 }) {
                    holes += 1
                }
            }
        }
        XCTAssertGreaterThan(opaque, Int(20 * scale * scale), "the capsule lost its ink", file: file, line: line)
        XCTAssertGreaterThan(holes, 0, "the badge has no transparent figure", file: file, line: line)
    }

    @MainActor
    private func drawItem(_ image: NSImage, appearance: NSAppearance.Name, scale: CGFloat) throws -> NSBitmapImageRep {
        let rep = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int((image.size.width * scale).rounded(.up)),
            pixelsHigh: Int((image.size.height * scale).rounded(.up)), bitsPerSample: 8, samplesPerPixel: 4,
            hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: rep))
        let appearance = try XCTUnwrap(NSAppearance(named: appearance))
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = context
        context.cgContext.clear(CGRect(x: 0, y: 0, width: rep.pixelsWide, height: rep.pixelsHigh))
        context.cgContext.scaleBy(x: scale, y: scale)
        appearance.performAsCurrentDrawingAppearance {
            image.draw(in: NSRect(origin: .zero, size: image.size))
        }
        return rep
    }

    @MainActor
    func testBadgeFiguresStayTransparentThroughRenderingAndAdaptiveImages() throws {
        for graph in [false, true] {
            for scale in [CGFloat(1), 2] {
                for badge in [3, 12, 120] {
                    let image = MenuBarStripLabel.rendered(MenuBarStripInputs(
                        traces: Self.columns, privateAI: Self.privateAI, condition: .live,
                        badge: badge, mark: .none, scale: scale, graph: graph))
                    XCTAssertFalse(image.isTemplate)
                    for scheme in [ColorScheme.light, .dark] {
                        let direct = try stripBitmap(badge: badge, graph: graph, scheme: scheme, scale: scale)
                        try assertCutOutFigure(direct, graph: graph, scheme: scheme, scale: scale)
                        let adaptive = try drawItem(image, appearance: scheme == .dark ? .darkAqua : .aqua, scale: scale)
                        XCTAssertEqual(adaptive.pixelsWide, direct.pixelsWide)
                        XCTAssertEqual(adaptive.pixelsHigh, direct.pixelsHigh)
                        try assertCutOutFigure(adaptive, graph: graph, scheme: scheme, scale: scale)
                        // An ungrouped destinationOut also punches through
                        // the parent background. The badge must clear only
                        // itself, leaving this opaque ground intact.
                        let grounded = try stripBitmap(
                            badge: badge, graph: graph, scheme: scheme, scale: scale,
                            background: Color(.sRGB, red: 0.2, green: 0.5, blue: 0.7))
                        // The layout can end on a fractional pixel; exclude
                        // its outermost column, which is antialiased.
                        for y in 0..<grounded.pixelsHigh {
                            for x in 0..<(grounded.pixelsWide - 1) {
                                XCTAssertEqual(alpha(grounded, x: x, y: y), 1, accuracy: 0.01,
                                               "the cut-out escaped the badge's compositing group")
                            }
                        }
                    }
                }
            }
        }
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
