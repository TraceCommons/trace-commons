import SwiftUI

/// A band of light crossing a card from left to right now and then (owner,
/// 2026-10-20: Home's Not contributed card), slanted as the promotions' and
/// the map's shimmers are. Each pass is one SwiftUI animation, so nothing
/// redraws between passes. Under Reduce Motion there are no passes. Give it
/// with `glassCardShimmer` before the card's tier, so the tier's fill stays
/// under it and the card's words over it.
public struct GlassCardShimmer: View {
    @State private var progress: CGFloat = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    public init() {}

    /// How often a pass starts, how soon the first comes once the card is
    /// shown, and how long a pass takes (the promotions' shimmer's 1.4s in
    /// every 6s).
    public static let every: Duration = .seconds(6)
    public static let first: Duration = .seconds(1)
    public static let duration: Double = 1.4
    /// How far the band leans from upright, its top to the right.
    public static let slant: Double = 20
    /// The band's width, as a share of the card's.
    public static let bandShare: CGFloat = 0.45

    /// The light at the band's crest: white on the dark theme's cards, the
    /// accent on light ones, where white would not show.
    static func crest(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color.white.opacity(0.14) : GlassTokens.Color.purpleSoft.color.opacity(0.12)
    }

    /// The band's leading edge at `progress`: wholly before the card at 0,
    /// wholly past it at 1, whatever the slant.
    public static func x(progress: CGFloat, width: CGFloat, band: CGFloat) -> CGFloat {
        let start = -band * 1.5
        let end = width + band * 0.5
        return start + (end - start) * progress
    }

    public var body: some View {
        GeometryReader { box in
            let band = box.size.width * Self.bandShare
            LinearGradient(colors: [.clear, Self.crest(colorScheme), .clear], startPoint: .leading, endPoint: .trailing)
                .frame(width: band, height: box.size.height * 2)
                .rotationEffect(.degrees(Self.slant))
                .offset(x: Self.x(progress: progress, width: box.size.width, band: band), y: -box.size.height / 2)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task(id: reduceMotion) {
            guard !reduceMotion else { return }
            try? await Task.sleep(for: Self.first)
            while !Task.isCancelled {
                // Back before the card at once, then across it.
                withTransaction(Transaction(animation: nil)) { progress = 0 }
                try? await Task.sleep(for: .milliseconds(30))
                withAnimation(.easeInOut(duration: Self.duration)) { progress = 1 }
                try? await Task.sleep(for: Self.every)
            }
        }
    }
}

public extension View {
    /// Draws `GlassCardShimmer` behind this view, clipped to the card's
    /// shape. Apply it before the card's tier, so the tier's fill stays
    /// under it.
    func glassCardShimmer(radius: CGFloat = GlassTier.card.radius) -> some View {
        background {
            GlassCardShimmer()
                .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
        }
    }
}
