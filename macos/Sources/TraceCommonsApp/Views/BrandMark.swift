import SwiftUI

/// The Trace Commons mark -- "The Turn" -- as pure geometry.
///
/// Two corner brackets facing each other inside a hairline frame: the user's
/// bracket in green at the top left, the agent's answer in blue at the bottom
/// right, and the session implied in the space between them. That space is the
/// mark. Nothing is drawn in it, and nothing should be.
///
/// It is transcribed from the approved mockups (`design-import/DESIGN-SPEC.md`
/// section 1.2) rather than carried as an asset, which is what lets it stay
/// exact at 84pt on the onboarding screen and at 15pt in the menu bar. The
/// mockups state it on a `0 0 64 64` viewBox, so every number here is in
/// 64ths of the mark and scales with it:
///
/// - frame rect inset 1 with a 2-unit stroke, so it sits exactly on the edge
/// - green bracket `M11 28 V11 H28`, blue bracket `M53 36 v17 H36`
/// - stroke 7/64 in the framed variants (about 11% of the mark)
/// - stroke 8/64 in the template variant, thickened to survive the lost frame
///
/// No gradients. No fill other than the frame. The mark it replaces -- the
/// green/blue diagonal gradient square from the community site's CSS -- is
/// superseded everywhere in the clients; the site keeps its own.
///
/// `reveal` drives the one piece of motion in the app: see `BrandMarkIntro`.
struct BrandMark: View {
    /// Which of the three drawings in the spec this is.
    enum Variant {
        /// Framed, two-colour, following the system appearance. The default,
        /// and what every in-window use wants.
        case auto
        /// Framed, two-colour, pinned to the light drawing regardless of the
        /// system appearance. For captures and for surfaces that are light in
        /// both appearances.
        case light
        /// Framed, two-colour, pinned to the dark drawing.
        case dark
        /// Frameless, single ink, drawn in `.primary` so AppKit recolours it
        /// for the menu bar's light, dark and selected states. The status-bar
        /// variant, and the only one that is not a picture of the mark so much
        /// as a stencil of it.
        case template
    }

    var size: CGFloat = 28
    var variant: Variant = .auto
    /// `1` is the finished mark. Below that each bracket is drawn back along
    /// its own path, so the mark assembles from its own geometry rather than
    /// fading or sliding as a picture would.
    var reveal: Double = 1

    init(size: CGFloat = 28, variant: Variant = .auto, reveal: Double = 1) {
        self.size = size
        self.variant = variant
        self.reveal = reveal
    }

    /// The menu-bar spelling, kept so call sites that ask for monochrome do
    /// not have to know the variant vocabulary.
    init(size: CGFloat = 28, monochrome: Bool, reveal: Double = 1) {
        self.init(size: size, variant: monochrome ? .template : .auto, reveal: reveal)
    }

    var body: some View {
        ZStack {
            if variant != .template {
                Rectangle().fill(TC.markField)
                Rectangle().strokeBorder(TC.markInk, lineWidth: unit(Geometry.frameStroke))
            }
            bracket(Geometry.greenBracket, ink: greenInk)
            bracket(Geometry.blueBracket, ink: blueInk)
        }
        .frame(width: size, height: size)
        .environment(\.colorScheme, pinnedScheme ?? environmentScheme)
        .accessibilityHidden(true)
    }

    // MARK: - Drawing

    @Environment(\.colorScheme) private var environmentScheme

    private func bracket(_ points: [CGPoint], ink: Color) -> some View {
        Bracket(points: points)
            .trim(from: 0, to: max(0, min(1, reveal)))
            .stroke(
                ink,
                style: StrokeStyle(
                    lineWidth: unit(variant == .template ? Geometry.templateStroke : Geometry.stroke),
                    lineCap: .butt,
                    lineJoin: .miter
                )
            )
    }

    /// One unit of the 64-unit viewBox, in points.
    private func unit(_ value: CGFloat) -> CGFloat {
        value * size / Geometry.viewBox
    }

    private var greenInk: Color {
        variant == .template ? .primary : TC.markAccent
    }

    private var blueInk: Color {
        variant == .template ? .primary : TC.markInk
    }

    private var pinnedScheme: ColorScheme? {
        switch variant {
        case .light: return .light
        case .dark: return .dark
        case .auto, .template: return nil
        }
    }

    private enum Geometry {
        static let viewBox: CGFloat = 64
        static let frameStroke: CGFloat = 2
        static let stroke: CGFloat = 7
        static let templateStroke: CGFloat = 8
        /// `M11 28 V11 H28` -- the top-left corner, drawn from the bottom of
        /// its vertical arm.
        static let greenBracket = [CGPoint(x: 11, y: 28), CGPoint(x: 11, y: 11), CGPoint(x: 28, y: 11)]
        /// `M53 36 v17 H36` -- the bottom-right corner, drawn from the top of
        /// its vertical arm.
        static let blueBracket = [CGPoint(x: 53, y: 36), CGPoint(x: 53, y: 53), CGPoint(x: 36, y: 53)]
    }
}

/// One corner bracket, in the mark's 64-unit space, mapped onto whatever rect
/// it is given.
///
/// Both arms are the same length, so trimming the path draws the bracket at an
/// even speed through its corner rather than lurching at the turn.
private struct Bracket: Shape {
    let points: [CGPoint]

    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / 64
        func place(_ point: CGPoint) -> CGPoint {
            CGPoint(x: rect.minX + point.x * scale, y: rect.minY + point.y * scale)
        }
        var path = Path()
        guard let first = points.first else { return path }
        path.move(to: place(first))
        for point in points.dropFirst() {
            path.addLine(to: place(point))
        }
        return path
    }
}

/// The mark, assembling itself once, on the first screen a contributor ever
/// sees.
///
/// The community site has no motion at all -- there is not one `transition`,
/// `@keyframes` or `requestAnimationFrame` in it -- so there was nothing to
/// port. This is the one moment of motion in the app, and it is built from the
/// only thing the brand actually owns: the two brackets. Each draws itself in
/// along its own path and stops. There is no loop, no bounce, no ambient
/// drift, and no second animated thing anywhere else in the product, because
/// an app that asks to be trusted with somebody's source code should not
/// fidget.
///
/// It respects Reduce Motion by rendering the finished mark immediately.
struct BrandMarkIntro: View {
    var size: CGFloat
    var variant: BrandMark.Variant = .auto

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var reveal: Double = 0

    var body: some View {
        BrandMark(size: size, variant: variant, reveal: reduceMotion ? 1 : reveal)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeOut(duration: 0.85).delay(0.15)) { reveal = 1 }
            }
    }
}

/// One row of the mark at every size the clients render it at: 84 on
/// onboarding, 40 in the brand rows, 22 as an inline swatch, 20 in a header
/// bar, 16 in a title bar, 15 in the macOS menu bar, 14 in the smallest
/// chrome.
private struct BrandMarkSizeRow: View {
    let variant: BrandMark.Variant

    var body: some View {
        HStack(alignment: .bottom, spacing: TC.Space.l) {
            ForEach([84, 40, 22, 20, 16, 15, 14], id: \.self) { size in
                BrandMark(size: CGFloat(size), variant: variant)
            }
        }
    }
}

#Preview("The Turn") {
    VStack(alignment: .leading, spacing: TC.Space.xl) {
        BrandMarkSizeRow(variant: .light)
        BrandMarkSizeRow(variant: .dark)
        BrandMarkSizeRow(variant: .template)
    }
    .padding(TC.Space.xl)
    .background(TC.ground)
}
