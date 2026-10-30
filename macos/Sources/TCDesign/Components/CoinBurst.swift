import SwiftUI

/// Gold coins bursting out of a card's centre like a firework (owner,
/// 2026-10-27: NEAR AI credit arriving on Home's Balance tile, in place of
/// the green edge that lit): a brief gold flash, then small coins fly out
/// every way, slow as they spread, tumble end over end as they droop and
/// fade, each trailing a few sparks while it is fast. The coins may fly
/// above and below the card (`spill`), and to its left, where they fall over
/// the cards beside it; to its right they stop at the card's side
/// (`sideways`), where the card is the last in its row and what holds it
/// ends: Home's scroll view runs flush to it, and would cut them off.
/// Under Reduce Motion the coins are drawn still, spread round the centre,
/// and nothing moves. Give it with `glassCoinBurst` over the card, and draw
/// the card above what follows it, so the coins fall over the cards below.
public struct GlassCoinBurst: View {
    /// When the burst began.
    private let start: Date
    /// Draws the coins still, at `stillAt`, for Reduce Motion.
    private let still: Bool

    public init(start: Date, still: Bool = false) {
        self.start = start
        self.still = still
    }

    /// How long a burst lasts, start to the last coin gone.
    public static let duration: Double = 1.7
    /// How many coins one burst throws.
    public static let count = 44
    /// How far past the card's edges the coins may fly.
    public static let spill: CGFloat = 120
    /// The moment drawn under Reduce Motion: spread out, not yet falling.
    static let stillAt: Double = 0.45

    /// How fast a coin slows as it spreads (per second), and how hard it
    /// droops once it has (points per second squared).
    static let drag: Double = 3.4
    static let gravity: Double = 50
    /// How much shorter a coin's reach is up and down than side to side, so
    /// the burst fills a wide card without leaving it far above and below.
    static let squash: Double = 0.72
    /// When the coins start to fade, as a share of `duration`.
    static let fadeFrom: Double = 0.55
    /// The gold flash at the centre as the burst begins.
    static let flashDuration: Double = 0.28
    static let flashRadius: CGFloat = 46

    /// The coin's golds: its lit face, its body, its shade and its rim, and
    /// the sparks it trails.
    static let goldLight = GlassRGBA(0xFFE58A)
    static let gold = GlassRGBA(0xF5B921)
    static let goldDeep = GlassRGBA(0xC58A12)
    static let goldRim = GlassRGBA(0x8F5E0A)
    static let spark = GlassRGBA(0xFFD45C)

    /// One coin: the way it flies, how fast it leaves, its size, how it
    /// turns, and how late it leaves the centre.
    struct Coin: Equatable {
        /// Radians, clockwise from the right (y runs down).
        let angle: Double
        /// Points per second as it leaves the centre.
        let speed: Double
        /// Its face's width, in points: very small.
        let diameter: CGFloat
        /// Turns end over end per second, and where in a turn it starts.
        let spin: Double
        let phase: Double
        /// The lean of the axis it turns on, in radians.
        let tilt: Double
        /// Seconds after the burst it leaves.
        let delay: Double
    }

    /// The burst's coins: the same every time, spaced round the circle with
    /// a little jitter so the burst is full but not a ring.
    static let coins: [Coin] = (0..<count).map { coin(at: $0) }

    /// A repeatable number in 0..<1 for `index` and `salt`.
    static func noise(_ index: Int, _ salt: Double) -> Double {
        let x = sin(Double(index) * 12.9898 + salt * 78.233) * 43758.5453
        return x - x.rounded(.down)
    }

    static func coin(at index: Int) -> Coin {
        let slot = Double(index) / Double(count)
        return Coin(
            angle: (slot + (noise(index, 1) - 0.5) / Double(count)) * 2 * .pi,
            speed: 220 + noise(index, 2) * 200,
            diameter: 3.5 + CGFloat(noise(index, 3)) * 2.5,
            spin: 1.4 + noise(index, 4) * 2.2,
            phase: noise(index, 5),
            tilt: (noise(index, 6) - 0.5) * 1.2,
            delay: noise(index, 7) * 0.08)
    }

    /// Where a coin is `t` seconds into the burst, from the centre: out
    /// along its way, slowed by drag, pulled down. `sideways` shortens its
    /// reach to the right (the side that is cut off), and only that.
    static func offset(_ coin: Coin, at t: Double, sideways: Double = 1) -> CGVector {
        let cosine = cos(coin.angle)
        let flown = max(0, t - coin.delay)
        let reach = coin.speed * (1 - exp(-drag * flown)) / drag
        let fall = 0.5 * gravity * flown * flown
        return CGVector(dx: cosine * reach * (cosine > 0 ? sideways : 1), dy: sin(coin.angle) * reach * squash + fall)
    }

    /// The farthest any coin flies side to side: its speed, spent against
    /// drag.
    static let farthest = (coins.map(\.speed).max() ?? 0) / drag
    /// The widest coin.
    static let widest = coins.map(\.diameter).max() ?? 0

    /// How much of its reach a coin flies to the right when the edge that
    /// cuts it off, the card's right side, is `room` from the centre: all of
    /// it when there is room, less when there is not, so that no coin
    /// passes that edge. The left is not limited: those coins fall over the
    /// cards beside, which cut nothing off.
    static func sideways(room: CGFloat) -> Double {
        max(0, min(1, Double(room - widest) / farthest))
    }

    /// How much of the coins shows `t` seconds in: whole while they fly,
    /// fading out from `fadeFrom` of the way, gone at `duration`.
    static func opacity(at t: Double) -> Double {
        let from = duration * fadeFrom
        guard t > from else { return 1 }
        return max(0, 1 - (t - from) / (duration - from))
    }

    public var body: some View {
        GlassAnimationTimeline(paused: still) { date in
            Canvas { context, size in
                let t = still ? Self.stillAt : date.timeIntervalSince(start)
                guard t >= 0, t < Self.duration else { return }
                Self.draw(in: &context, size: size, at: t, flash: !still)
            }
        }
        .padding(-Self.spill)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// The burst `t` seconds in, round the centre of a field of `size`: the
    /// card and `spill` past it.
    static func draw(in context: inout GraphicsContext, size: CGSize, at t: Double, flash: Bool) {
        let centre = CGPoint(x: size.width / 2, y: size.height / 2)
        let sideways = Self.sideways(room: size.width / 2 - spill)
        if flash, t < flashDuration {
            let left = 1 - t / flashDuration
            let radius = flashRadius * CGFloat(0.4 + 0.6 * (t / flashDuration))
            context.fill(
                Path(ellipseIn: CGRect(x: centre.x - radius, y: centre.y - radius, width: radius * 2, height: radius * 2)),
                with: .radialGradient(
                    // To clear gold, not clear black, which greys the edge.
                    Gradient(colors: [goldLight.color.opacity(0.75 * left), gold.color.opacity(0.3 * left),
                                      gold.color.opacity(0)]),
                    center: centre, startRadius: 0, endRadius: radius))
        }
        context.opacity = opacity(at: t)
        for coin in coins {
            let flown = t - coin.delay
            guard flown > 0 else { continue }
            let at = offset(coin, at: t, sideways: sideways)
            let point = CGPoint(x: centre.x + at.dx, y: centre.y + at.dy)
            // Sparks behind it while it is still fast.
            if flown < 0.55 {
                for step in 1...3 {
                    let back = offset(coin, at: t - Double(step) * 0.035, sideways: sideways)
                    let dot = coin.diameter * (0.42 - CGFloat(step) * 0.08)
                    context.fill(
                        Path(ellipseIn: CGRect(x: centre.x + back.dx - dot / 2, y: centre.y + back.dy - dot / 2,
                                               width: dot, height: dot)),
                        with: .color(spark.color.opacity((0.75 - Double(step) * 0.2) * (1 - flown / 0.55))))
                }
            }
            drawCoin(coin, at: point, turn: coin.phase + coin.spin * flown, in: &context)
        }
    }

    /// One coin turning end over end: a full disc face on, a sliver edge
    /// on, its lit face brighter than its back.
    static func drawCoin(_ coin: Coin, at point: CGPoint, turn: Double, in context: inout GraphicsContext) {
        let facing = cos(turn * 2 * .pi)
        let width = coin.diameter * CGFloat(max(0.2, abs(facing)))
        let face = CGRect(x: -width / 2, y: -coin.diameter / 2, width: width, height: coin.diameter)
        var local = context
        local.translateBy(x: point.x, y: point.y)
        local.rotate(by: .radians(coin.tilt))
        let disc = Path(ellipseIn: face)
        let lit = facing > 0
        local.fill(disc, with: .linearGradient(
            Gradient(colors: lit ? [goldLight.color, gold.color, goldDeep.color] : [gold.color, goldDeep.color]),
            startPoint: CGPoint(x: face.minX, y: face.minY), endPoint: CGPoint(x: face.maxX, y: face.maxY)))
        local.stroke(disc, with: .color(goldRim.color), lineWidth: 0.6)
        // A glint as it turns face on.
        if abs(facing) > 0.92 {
            let glint = coin.diameter * 0.32
            local.fill(
                Path(ellipseIn: CGRect(x: -coin.diameter * 0.28, y: -coin.diameter * 0.3, width: glint, height: glint)),
                with: .color(.white.opacity(0.9)))
        }
    }
}

public extension View {
    /// Bursts gold coins out of this view's centre (`GlassCoinBurst`) from
    /// `start`. Nil draws nothing, and adds no view: almost always, there
    /// is no burst.
    func glassCoinBurst(start: Date?, still: Bool = false) -> some View {
        overlay {
            if let start {
                GlassCoinBurst(start: start, still: still)
            }
        }
    }
}
