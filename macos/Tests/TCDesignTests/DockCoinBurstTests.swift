import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

@MainActor
final class DockCoinBurstTests: XCTestCase {
    func testDockCoinsAreDeterministicAndStayInsideTileWhileVisible() {
        XCTAssertEqual(GlassDockCoinBurst.coins.count, GlassDockCoinBurst.count)
        XCTAssertGreaterThanOrEqual(GlassDockCoinBurst.count, 20)
        XCTAssertEqual(
            GlassDockCoinBurst.coins,
            (0..<GlassDockCoinBurst.count).map { GlassDockCoinBurst.coin(at: $0) }
        )

        let tile = GlassDockCoinBurst.defaultSize
        let halfW = tile.width / 2
        let halfH = tile.height / 2
        var t = 0.0
        while t < GlassDockCoinBurst.duration {
            if GlassCoinBurst.opacity(at: t) >= 0.2 {
                for coin in GlassDockCoinBurst.coins {
                    let at = GlassDockCoinBurst.offset(coin, at: t, tileSize: tile)
                    XCTAssertLessThanOrEqual(abs(at.dx), halfW, "coin left tile horizontally at t=\(t)")
                    XCTAssertLessThanOrEqual(abs(at.dy), halfH + 10, "coin fell far below tile at t=\(t)")
                }
            }
            t += 0.05
        }
    }

    func testDockIconScaleRespectsReduceMotion() {
        XCTAssertEqual(GlassDockCoinBurst.iconScale(at: 0, flash: true), 1)
        XCTAssertGreaterThan(GlassDockCoinBurst.iconScale(at: 0.15, flash: true), 1)
        XCTAssertEqual(GlassDockCoinBurst.iconScale(at: 0.15, flash: false), 1)
        XCTAssertEqual(GlassDockCoinBurst.iconScale(at: GlassDockCoinBurst.pulseDuration, flash: true), 1)
    }

    @MainActor
    func testDockRenderAddsGoldPixelsOverBaseIcon() throws {
        let base = NSImage(size: GlassDockCoinBurst.defaultSize, flipped: false) { rect in
            NSColor(srgbRed: 0.12, green: 0.12, blue: 0.16, alpha: 1).setFill()
            NSBezierPath(roundedRect: rect.insetBy(dx: 12, dy: 12), xRadius: 24, yRadius: 24).fill()
            return true
        }
        let burst = GlassDockCoinBurst.render(
            baseIcon: base,
            at: GlassCoinBurst.stillAt,
            flash: false,
            tileSize: GlassDockCoinBurst.defaultSize,
            scale: 2
        )
        XCTAssertEqual(burst.size, GlassDockCoinBurst.defaultSize)
        let rep = try XCTUnwrap(burst.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:)))
        var goldPixels = 0
        for y in stride(from: 0, to: rep.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: rep.pixelsWide, by: 2) {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                if c.redComponent > 0.7 && c.greenComponent > 0.5 && c.blueComponent < 0.45 {
                    goldPixels += 1
                }
            }
        }
        XCTAssertGreaterThan(goldPixels, 40, "burst frame should paint visible gold coins on the Dock tile")
    }

    func testMenuBarStripGleamAndHopEnvelope() {
        XCTAssertEqual(GlassMenuBarStrip.creditGleam(at: -0.1), 0)
        XCTAssertEqual(GlassMenuBarStrip.creditGleam(at: 0), 0)
        XCTAssertEqual(GlassMenuBarStrip.creditGleam(at: 0.45), 1)
        XCTAssertEqual(GlassMenuBarStrip.creditGleam(at: GlassCoinBurst.duration), 0)

        XCTAssertLessThan(GlassMenuBarStrip.markHop(at: 0.17), 0, "hops upward early")
        XCTAssertEqual(GlassMenuBarStrip.markHop(at: GlassCoinBurst.stillAt), 0, "still under Reduce Motion")
        XCTAssertEqual(GlassMenuBarStrip.markHop(at: GlassMenuBarStrip.markHopDuration), 0)
    }

    @MainActor
    func testMenuBarStripCreditBurstKeepsExactSizeAndAddsGold() throws {
        let columns = (0..<6).map { GlassDayColumn(id: "s\($0)", up: $0 % 3, down: 1) }
        func render(_ creditBurst: Double?) throws -> NSBitmapImageRep {
            let renderer = ImageRenderer(
                content: GlassMenuBarStrip(
                    traces: columns, privateAI: nil, condition: .live, badge: nil,
                    mark: .none, creditBurst: creditBurst
                )
                .environment(\.colorScheme, .dark)
            )
            renderer.scale = 2
            let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation)
            return try XCTUnwrap(NSBitmapImageRep(data: tiff))
        }
        let idle = try render(nil)
        let burst = try render(GlassCoinBurst.stillAt)
        XCTAssertEqual(burst.pixelsWide, idle.pixelsWide, "menu-bar strip width must not jitter")
        XCTAssertEqual(burst.pixelsHigh, idle.pixelsHigh, "menu-bar strip height must not jitter")

        var goldPixels = 0
        for y in 0..<burst.pixelsHigh {
            for x in 0..<burst.pixelsWide {
                guard let c = burst.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                if c.redComponent > 0.7 && c.greenComponent > 0.5 && c.blueComponent < 0.45 {
                    goldPixels += 1
                }
            }
        }
        XCTAssertGreaterThan(goldPixels, 20, "menu-bar strip burst should paint gold gleam and coins")
    }
}
