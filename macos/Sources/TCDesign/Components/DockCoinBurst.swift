import AppKit
import SwiftUI

/// Gold coins bursting out of the app's Dock icon when NEAR AI credits arrive.
///
/// Shares `GlassCoinBurst`'s gold palette, drag/gravity flight and tumbling
/// coin drawing, scaled to the 128pt Dock tile so the burst stays inside the
/// tile without clipping. Drawn over the unmodified `baseIcon` and rendered
/// frame-by-frame into `NSApp.dockTile.contentView`; under Reduce Motion
/// `flash` is false and a single still frame at `GlassCoinBurst.stillAt` is
/// held briefly before restoring the system icon.
public struct GlassDockCoinBurst: View {
    private let baseIcon: NSImage
    private let elapsed: Double
    private let flash: Bool
    private let tileSize: CGSize

    public init(
        baseIcon: NSImage,
        elapsed: Double,
        flash: Bool = true,
        tileSize: CGSize = Self.defaultSize
    ) {
        self.baseIcon = baseIcon
        self.elapsed = elapsed
        self.flash = flash
        self.tileSize = tileSize
    }

    public static let defaultSize = CGSize(width: 128, height: 128)
    public static let duration: Double = GlassCoinBurst.duration
    public static let count = 24

    static let drag: Double = 3.8
    static let gravity: Double = 32
    static let flashDuration: Double = 0.32
    static let flashRadius: CGFloat = 56
    static let pulseDuration: Double = 0.36
    static let pulsePeak: CGFloat = 0.045

    /// Deterministic set of coins radiating from the Dock icon's centre.
    static let coins: [GlassCoinBurst.Coin] = (0..<count).map { coin(at: $0) }

    static func coin(at index: Int) -> GlassCoinBurst.Coin {
        let slot = Double(index) / Double(count)
        return GlassCoinBurst.Coin(
            angle: (slot + (GlassCoinBurst.noise(index, 11) - 0.5) / Double(count)) * 2 * .pi,
            speed: 120 + GlassCoinBurst.noise(index, 12) * 90,
            diameter: 6.0 + CGFloat(GlassCoinBurst.noise(index, 13)) * 4.5,
            spin: 1.5 + GlassCoinBurst.noise(index, 14) * 2.0,
            phase: GlassCoinBurst.noise(index, 15),
            tilt: (GlassCoinBurst.noise(index, 16) - 0.5) * 1.2,
            delay: GlassCoinBurst.noise(index, 17) * 0.08
        )
    }

    /// Subtle scale pulse on the app icon itself as the burst begins.
    /// Always 1.0 under Reduce Motion (`flash == false`) or once settled.
    static func iconScale(at t: Double, flash: Bool) -> CGFloat {
        guard flash, t > 0, t < pulseDuration else { return 1 }
        return 1 + pulsePeak * CGFloat(sin(.pi * t / pulseDuration))
    }

    /// Where a coin is `t` seconds into the burst, relative to the tile's
    /// centre, scaled to `tileSize` relative to `defaultSize`.
    static func offset(_ coin: GlassCoinBurst.Coin, at t: Double, tileSize: CGSize = defaultSize) -> CGVector {
        let flown = max(0, t - coin.delay)
        let reach = coin.speed * (1 - exp(-drag * flown)) / drag
        let fall = 0.5 * gravity * flown * flown
        let scaleX = tileSize.width / defaultSize.width
        let scaleY = tileSize.height / defaultSize.height
        return CGVector(
            dx: cos(coin.angle) * reach * scaleX,
            dy: (sin(coin.angle) * reach + fall) * scaleY
        )
    }

    public var body: some View {
        ZStack {
            Image(nsImage: baseIcon)
                .resizable()
                .interpolation(.high)
                .aspectRatio(contentMode: .fit)
                .scaleEffect(Self.iconScale(at: elapsed, flash: flash))
            Canvas { context, size in
                guard elapsed >= 0, elapsed < Self.duration else { return }
                Self.drawOverlay(in: &context, size: size, at: elapsed, flash: flash)
            }
        }
        .frame(width: tileSize.width, height: tileSize.height)
    }

    static func drawOverlay(in context: inout GraphicsContext, size: CGSize, at t: Double, flash: Bool) {
        let centre = CGPoint(x: size.width / 2, y: size.height / 2)
        let unit = min(size.width, size.height) / min(defaultSize.width, defaultSize.height)
        if flash, t < flashDuration {
            let left = 1 - t / flashDuration
            let radius = flashRadius * unit * CGFloat(0.35 + 0.65 * (t / flashDuration))
            context.fill(
                Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2)),
                with: .radialGradient(
                    Gradient(colors: [
                        GlassCoinBurst.goldLight.color.opacity(0.8 * left),
                        GlassCoinBurst.gold.color.opacity(0.35 * left),
                        GlassCoinBurst.gold.color.opacity(0),
                    ]),
                    center: centre, startRadius: 0, endRadius: radius
                )
            )
        }
        context.opacity = GlassCoinBurst.opacity(at: t)
        for coin in coins {
            let flown = t - coin.delay
            guard flown > 0 else { continue }
            let at = offset(coin, at: t, tileSize: size)
            let point = CGPoint(x: centre.x + at.dx, y: centre.y + at.dy)
            if flown < 0.55 {
                for step in 1...3 {
                    let back = offset(coin, at: t - Double(step) * 0.035, tileSize: size)
                    let dot = coin.diameter * unit * (0.42 - CGFloat(step) * 0.08)
                    context.fill(
                        Path(ellipseIn: CGRect(
                            x: centre.x + back.dx - dot / 2,
                            y: centre.y + back.dy - dot / 2,
                            width: dot, height: dot
                        )),
                        with: .color(GlassCoinBurst.spark.color.opacity((0.75 - Double(step) * 0.2) * (1 - flown / 0.55)))
                    )
                }
            }
            let scaledCoin = GlassCoinBurst.Coin(
                angle: coin.angle, speed: coin.speed, diameter: coin.diameter * unit,
                spin: coin.spin, phase: coin.phase, tilt: coin.tilt, delay: coin.delay
            )
            GlassCoinBurst.drawCoin(scaledCoin, at: point, turn: coin.phase + coin.spin * flown, in: &context)
        }
    }

    /// Renders one Dock tile frame as a non-template `NSImage`.
    @MainActor
    public static func render(
        baseIcon: NSImage,
        at elapsed: Double,
        flash: Bool = true,
        tileSize: CGSize = defaultSize,
        scale: CGFloat = 2
    ) -> NSImage {
        let renderer = ImageRenderer(
            content: GlassDockCoinBurst(
                baseIcon: baseIcon,
                elapsed: elapsed,
                flash: flash,
                tileSize: tileSize
            )
        )
        renderer.scale = scale
        let image = renderer.nsImage ?? baseIcon
        image.size = tileSize
        return image
    }
}
