import SwiftUI

/// Soft lights turning round a card's edge like a lighthouse's beams
/// (owner, 2026-10-22: Home's Not contributed card, in place of the band of
/// light that crossed it): two, half a turn apart, starting at the top and
/// the bottom. A pass fades them in on the edge, turns them once round the
/// card and fades them away. One comes when the card's tab is
/// shown or its window comes back into focus, then one every 15s. Each
/// pass is three SwiftUI animations, so nothing redraws between passes.
/// Under Reduce Motion there are no passes. Give it with `glassCardBeacon`
/// after the card's tier, so the light sits on the card's edge over its
/// fill.
public struct GlassCardBeacon: View {
    let radius: CGFloat
    /// How far round the card the beam is, in whole turns.
    @State private var turn: Double = 0
    /// How bright the beam is: 0 between passes.
    @State private var light: Double = 0
    /// Counts the times the window came back into focus; each starts the
    /// passes over.
    @State private var focus = 0
    /// When the turn under way ends.
    @State private var passEnds: ContinuousClock.Instant?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.controlActiveState) private var activeState

    public init(radius: CGFloat = GlassTier.card.radius) {
        self.radius = radius
    }

    /// How often a pass starts, and how soon the first comes once the card
    /// is shown or its window is back in focus.
    public static let every: Duration = .seconds(15)
    public static let first: Duration = .seconds(1)
    /// One turn round the card, and the fade in at its start and out at
    /// its end.
    public static let duration: Double = 4
    public static let fade: Double = 0.8
    /// The beam's width, as a share of a whole turn.
    public static let beamShare: Double = 0.16
    /// The edge the beam lights, and the soft glow round it.
    public static let lineWidth: CGFloat = 1.5
    public static let glowWidth: CGFloat = 6
    public static let glowBlur: CGFloat = 6
    /// How bright the beams are at their fullest, and how much of that the
    /// glow round them takes (owner, 2026-10-22: softened, dimmed a bit).
    public static let peak: Double = 0.7
    public static let glowShare: Double = 0.55

    /// How long to wait for the first pass: `first`, or until a turn still
    /// under way has ended, so it is not cut short.
    static func firstWait(now: ContinuousClock.Instant, passEnds: ContinuousClock.Instant?) -> Duration {
        guard let passEnds else { return first }
        return max(first, now.duration(to: passEnds))
    }

    /// Where the two beams sit round the turn: half a turn apart, so at the
    /// start of a pass one lights the card's bottom edge and the other its
    /// top (owner, 2026-10-22: two light sources, top and bottom).
    static let crests: [Double] = [0.25, 0.75]

    /// The beams round the card's centre: at each crest, clear rising to
    /// `crest` and clear again, with nothing on the rest of the turn.
    static func beam(_ crest: Color) -> Gradient {
        let half = beamShare / 2
        var stops: [Gradient.Stop] = [.init(color: .clear, location: 0)]
        for location in crests {
            stops += [
                .init(color: .clear, location: location - half),
                .init(color: crest, location: location),
                .init(color: .clear, location: location + half),
            ]
        }
        return Gradient(stops: stops + [.init(color: .clear, location: 1)])
    }

    /// What starts the passes over: Reduce Motion turned on or off, or the
    /// window back in focus.
    private struct Run: Equatable {
        let reduceMotion: Bool
        let focus: Int
    }

    public var body: some View {
        GeometryReader { box in
            let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
            ZStack {
                // The glow: the beam on a wider edge, softened.
                beam(in: box.size)
                    .mask(shape.strokeBorder(lineWidth: Self.glowWidth))
                    .blur(radius: Self.glowBlur)
                    .opacity(Self.glowShare)
                beam(in: box.size)
                    .mask(shape.strokeBorder(lineWidth: Self.lineWidth))
            }
            .opacity(light * Self.peak)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        // The tab shown builds the card, which starts the passes; the window
        // back in focus starts them over.
        .onChange(of: activeState == .inactive) { _, inactive in
            if !inactive { focus += 1 }
        }
        .task(id: Run(reduceMotion: reduceMotion, focus: focus)) {
            guard !reduceMotion else {
                light = 0
                return
            }
            try? await Task.sleep(for: Self.firstWait(now: .now, passEnds: passEnds))
            // A pass started over mid-turn left its light on: out first.
            if light > 0 {
                withAnimation(GlassMotion.curve(Self.fade)) { light = 0 }
                try? await Task.sleep(for: .seconds(Self.fade))
            }
            while !Task.isCancelled {
                // Back to the start at once while the light is out.
                withTransaction(Transaction(animation: nil)) { turn = 0 }
                try? await Task.sleep(for: .milliseconds(30))
                passEnds = .now + .seconds(Self.duration)
                // A lighthouse turns at one speed, so the turn is linear;
                // the fades take the one motion curve (`GlassMotion`).
                withAnimation(.linear(duration: Self.duration)) { turn = 1 }
                withAnimation(GlassMotion.curve(Self.fade)) { light = 1 }
                try? await Task.sleep(for: .seconds(Self.duration - Self.fade))
                withAnimation(GlassMotion.curve(Self.fade)) { light = 0 }
                try? await Task.sleep(for: Self.every - .seconds(Self.duration - Self.fade))
            }
        }
    }

    /// The beams, turned `turn` of the way round, filling a square wide
    /// enough to cover the card at any angle. The gradient starts at the
    /// right and runs clockwise, so the crests at a quarter and three
    /// quarters start at the bottom and the top.
    private func beam(in size: CGSize) -> some View {
        let side = hypot(size.width, size.height)
        return AngularGradient(gradient: Self.beam(GlassTokens.Color.tabEmber.color), center: .center,
                               angle: .degrees(0))
            .frame(width: side, height: side)
            .rotationEffect(.degrees(turn * 360))
            .position(x: size.width / 2, y: size.height / 2)
    }
}

public extension View {
    /// Draws `GlassCardBeacon` on this view's edge. Apply it after the
    /// card's tier, so the light sits over the tier's fill.
    func glassCardBeacon(radius: CGFloat = GlassTier.card.radius) -> some View {
        overlay { GlassCardBeacon(radius: radius) }
    }
}
