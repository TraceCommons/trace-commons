import SwiftUI

/// A soft band of light crossing a card (owner, 2026-10-27: Home's Not
/// contributed card, in place of the beacon that turned round its edge,
/// and brighter than the band that crossed it before; softened the same
/// day, when a hard white streak with a second glint and a lit rim read as
/// too reflective): a slanted glow sweeps left to right in under a second,
/// warming the card's face and, faintly, its edge. One pass comes each
/// time the card's tab is shown or its window comes back into focus, then
/// one every 18s. Each pass is one SwiftUI animation of `GlassCardShine`,
/// so nothing redraws between passes. Under Reduce Motion there are no
/// passes. Give it with `glassCardShimmer` after the card's tier, so the
/// light sits over the card's fill, its watermark and its words.
public struct GlassCardShimmer: View {
    let radius: CGFloat
    /// How far across the card the light is: 0 wholly before it, 1 wholly
    /// past it. Nothing shows at either end.
    @State private var progress: Double = 0
    /// Counts the times the window came back into focus; each starts the
    /// passes over.
    @State private var focus = 0
    /// When the pass under way ends.
    @State private var passEnds: ContinuousClock.Instant?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
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

    /// What starts the passes over: Reduce Motion turned on or off, or the
    /// window back in focus.
    private struct Run: Equatable {
        let reduceMotion: Bool
        let focus: Int
    }

    public var body: some View {
        GlassCardShine(progress: progress, radius: radius, dark: colorScheme == .dark)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            // The tab shown builds the card, which starts the passes; the
            // window back in focus starts them over.
            .onChange(of: activeState == .inactive) { _, inactive in
                if !inactive { focus += 1 }
            }
            .task(id: Run(reduceMotion: reduceMotion, focus: focus)) {
                guard !reduceMotion else {
                    withTransaction(Transaction(animation: nil)) { progress = 0 }
                    return
                }
                try? await Task.sleep(for: Self.firstWait(now: .now, passEnds: passEnds))
                while !Task.isCancelled {
                    // Back before the card at once, where nothing shows.
                    withTransaction(Transaction(animation: nil)) { progress = 0 }
                    try? await Task.sleep(for: .milliseconds(30))
                    passEnds = .now + .seconds(Self.duration)
                    // The one motion curve (`GlassMotion`), over the pass.
                    withAnimation(GlassMotion.curve(Self.duration)) { progress = 1 }
                    try? await Task.sleep(for: Self.every)
                }
            }
    }
}

/// The light at one point of its pass. Animatable, so a pass is one
/// animation of `progress` that redraws only while it runs.
struct GlassCardShine: View, Animatable {
    var progress: Double
    let radius: CGFloat
    let dark: Bool

    nonisolated var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    /// How far the band leans from upright, its top to the right, as the
    /// map's and the promotions' shimmers do.
    static let slant: Double = 20
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

    /// The band at `progress`: the gradient's start and end. The band leans
    /// `slant` degrees from upright and travels left to right, from wholly
    /// before the card's first corner to wholly past its last, so a pass
    /// starts and ends off the card whatever its shape.
    static func band(progress: Double, size: CGSize) -> (start: CGPoint, end: CGPoint) {
        let angle = slant * .pi / 180
        // Square to the band: rightward, tipped down by the slant (y runs
        // down), so the band itself leans with its top to the right.
        let unit = CGVector(dx: CGFloat(cos(angle)), dy: CGFloat(sin(angle)))
        let corners = [CGPoint.zero, CGPoint(x: size.width, y: 0), CGPoint(x: 0, y: size.height),
                       CGPoint(x: size.width, y: size.height)]
        let along = corners.map { $0.x * unit.dx + $0.y * unit.dy }
        let half = halfWidth * max(hypot(size.width, size.height), 1)
        let from = (along.min() ?? 0) - half
        let to = (along.max() ?? 0) + half
        let centre = from + (to - from) * CGFloat(progress)
        return (CGPoint(x: unit.dx * (centre - half), y: unit.dy * (centre - half)),
                CGPoint(x: unit.dx * (centre + half), y: unit.dy * (centre + half)))
    }

    var body: some View {
        Canvas { context, size in
            guard progress > 0, progress < 1 else { return }
            let band = Self.band(progress: progress, size: size)
            let card = CGRect(origin: .zero, size: size)
            let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
            for layer in [Self.glow(dark: dark), Self.core(dark: dark)] {
                let light = GraphicsContext.Shading.linearGradient(layer, startPoint: band.start, endPoint: band.end)
                // The face the light crosses.
                context.fill(shape.path(in: card), with: light)
                // A faint, blurred glow on the edge beside it.
                context.drawLayer { glow in
                    glow.opacity = Self.rimGlowShare
                    glow.addFilter(.blur(radius: Self.rimGlowBlur))
                    glow.clip(to: shape.path(in: card))
                    glow.stroke(shape.path(in: card), with: light, lineWidth: Self.rimGlowWidth)
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
