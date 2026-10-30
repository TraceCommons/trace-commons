import SwiftUI

/// A soft band of light crossing a card (owner, 2026-10-27: Home's Not
/// contributed card, in place of the beacon that turned round its edge,
/// and brighter than the band that crossed it before; softened the same
/// day, when a hard white streak with a second glint and a lit rim read as
/// too reflective): a slanted glow sweeps left to right in under a second,
/// warming the card's face and, faintly, its edge. One pass comes each
/// time the card's tab is shown or its window comes back into focus or into
/// sight, then one every 18s. Each pass is one SwiftUI animation of
/// `GlassCardShine`, so nothing redraws between passes. Under Reduce Motion
/// there are no passes, nor while the window cannot be seen
/// (`glassAnimationsVisible`). Give it with `glassCardShimmer` after the
/// card's tier, so the light sits over the card's fill, its watermark and
/// its words.
public struct GlassCardShimmer: View {
    let radius: CGFloat
    /// The passes made: each whole step is one, and the fractional part is
    /// how far across the card the light is (`GlassCardShine`). It only
    /// grows, as the map's shimmer's does, so a pass needs no reset.
    @State private var sweep: Double = 0
    /// Counts the times the window came back into focus; each starts the
    /// passes over.
    @State private var focus = 0
    /// When the pass under way ends.
    @State private var passEnds: ContinuousClock.Instant?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.glassAnimationsVisible) private var visible
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.colorScheme) private var colorScheme

    public init(radius: CGFloat = GlassTier.card.radius) {
        self.radius = radius
    }

    /// How often a pass starts, and how soon the first comes once the card
    /// is shown or its window is back in focus (owner, 2026-10-27: every
    /// 18s, and each time the tab is opened).
    public static let every: Duration = .seconds(18)
    public static let first: Duration = .milliseconds(400)
    /// One pass across the card: quick (owner, 2026-10-27).
    public static let duration: Double = 0.65

    /// How long to wait for the first pass: `first`, or until a pass still
    /// under way has ended, so it is not cut short.
    static func firstWait(now: ContinuousClock.Instant, passEnds: ContinuousClock.Instant?) -> Duration {
        guard let passEnds else { return first }
        return max(first, now.duration(to: passEnds))
    }

    /// `sweep` with no pass under way: the next whole step, where nothing
    /// shows.
    static func resting(_ sweep: Double) -> Double { sweep.rounded(.up) }

    /// What starts the passes over: Reduce Motion turned on or off, the
    /// window coming into or out of sight, or back in focus.
    private struct Run: Equatable {
        let reduceMotion: Bool
        let visible: Bool
        let focus: Int
    }

    public var body: some View {
        GlassCardShine(sweep: sweep, radius: radius, dark: colorScheme == .dark)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            // The tab shown builds the card, which starts the passes; the
            // window back in focus starts them over.
            .onChange(of: activeState == .inactive) { _, inactive in
                if !inactive { focus += 1 }
            }
            .task(id: Run(reduceMotion: reduceMotion, visible: visible, focus: focus)) {
                // A pass under way when this starts over runs out; none
                // starts after it.
                // Reduce Motion turned on mid-pass ends it at once.
                if reduceMotion {
                    withTransaction(Transaction(animation: nil)) { sweep = Self.resting(sweep) }
                }
                guard !reduceMotion, visible else { return }
                var wait = Self.firstWait(now: .now, passEnds: passEnds)
                while !Task.isCancelled {
                    try? await Task.sleep(for: wait)
                    guard !Task.isCancelled else { return }
                    passEnds = .now + .seconds(Self.duration)
                    // The one motion curve (`GlassMotion`), over the pass.
                    withAnimation(GlassMotion.curve(Self.duration)) { sweep += 1 }
                    wait = Self.every
                }
            }
    }
}

/// The light at one point of its pass. Animatable, so a pass is one
/// animation of `sweep` that redraws only while it runs.
struct GlassCardShine: View, Animatable {
    /// The passes made; the fractional part is how far across the card the
    /// light is, 0 wholly before it, toward 1 wholly past it. Nothing shows
    /// between passes.
    var sweep: Double
    let radius: CGFloat
    let dark: Bool

    nonisolated var animatableData: Double {
        get { sweep }
        set { sweep = newValue }
    }

    /// How far the band leans from upright, its top to the right: the
    /// map's shimmer's lean.
    static let slant = GlassMapMarks.shimmerSlant
    /// Half the band the light spans, as a share of the card's diagonal.
    static let halfWidth: CGFloat = 0.13
    /// The faint glow the light leaves on the card's edge as it passes:
    /// blurred, with no crisp line, which read as a reflection.
    static let rimGlowWidth: CGFloat = 5
    static let rimGlowBlur: CGFloat = 4
    static let rimGlowShare: Double = 0.5

    /// The light across the band, edge to edge, as two layers: a wide soft
    /// glow, and over it a brighter, narrower (but not hard) centre. Each
    /// falls off smoothly (a bell, not bars), so it reads as light rather
    /// than a gradient, and neither has a hard edge or a second glint, so
    /// it does not read as a reflection. On the dark theme both are white
    /// and add to what is under them (`plusLighter`). On the light theme,
    /// where white on a pale card would barely show, the glow is a sky-blue
    /// haze (the NEAR blue) that sets the white centre off.
    struct Profile {
        let peak: Double
        /// The bell's width, as a share of the band.
        let spread: Double

        func light(at x: Double) -> Double {
            let value = peak * exp(-pow((x - 0.5) / spread, 2))
            // Clear, not a faint tail, toward the band's edges.
            return value < 0.004 ? 0 : value
        }
    }

    /// The glow and the core on each theme.
    static func glowProfile(dark: Bool) -> Profile {
        dark ? Profile(peak: 0.1, spread: 0.2) : Profile(peak: 0.26, spread: 0.18)
    }

    static func coreProfile(dark: Bool) -> Profile {
        dark ? Profile(peak: 0.3, spread: 0.08) : Profile(peak: 0.55, spread: 0.08)
    }

    /// How finely a profile is laid down as gradient stops: fine enough
    /// for the centre's bell.
    static let samples = 61

    static func gradient(_ profile: Profile, _ color: Color) -> Gradient {
        Gradient(stops: (0..<samples).map { index in
            let x = Double(index) / Double(samples - 1)
            return .init(color: color.opacity(profile.light(at: x)), location: x)
        })
    }

    /// The light theme's haze: the NEAR sky blue (owner, 2026-10-20).
    static let haze = GlassRGBA(0x7EBCFF)

    static func glow(dark: Bool) -> Gradient {
        gradient(glowProfile(dark: dark), dark ? .white : haze.color)
    }

    static func core(dark: Bool) -> Gradient {
        gradient(coreProfile(dark: dark), .white)
    }

    /// Each theme's two layers, glow under core, built once rather than on
    /// every frame of a pass.
    static let darkLayers = [glow(dark: true), core(dark: true)]
    static let lightLayers = [glow(dark: false), core(dark: false)]

    /// The band at `progress` (`GlassShimmerBand`): wholly before the card
    /// at 0, wholly past it at 1, so a pass starts and ends off the card.
    static func band(progress: Double, size: CGSize) -> (start: CGPoint, end: CGPoint) {
        GlassShimmerBand.band(progress: progress, size: size, slant: slant, halfWidth: halfWidth)
    }

    var body: some View {
        Canvas { context, size in
            let progress = sweep - sweep.rounded(.down)
            guard progress > 0 else { return }
            let band = Self.band(progress: progress, size: size)
            let card = RoundedRectangle(cornerRadius: radius, style: .continuous)
                .path(in: CGRect(origin: .zero, size: size))
            for layer in dark ? Self.darkLayers : Self.lightLayers {
                let light = GraphicsContext.Shading.linearGradient(layer, startPoint: band.start, endPoint: band.end)
                // The face the light crosses.
                context.fill(card, with: light)
                // A faint, blurred glow on the edge beside it.
                context.drawLayer { glow in
                    glow.opacity = Self.rimGlowShare
                    glow.addFilter(.blur(radius: Self.rimGlowBlur))
                    glow.clip(to: card)
                    glow.stroke(card, with: light, lineWidth: Self.rimGlowWidth)
                }
            }
        }
        .blendMode(dark ? .plusLighter : .normal)
    }
}

public extension View {
    /// Draws `GlassCardShimmer` over this view. Apply it after the card's
    /// tier, so the light sits over the tier's fill.
    func glassCardShimmer(radius: CGFloat = GlassTier.card.radius) -> some View {
        overlay { GlassCardShimmer(radius: radius) }
    }
}
