import SwiftUI

/// Shorthand for the token colours as SwiftUI colours.
public enum GlassColor {
    public static var textPrimary: Color { GlassTokens.Color.textPrimary.color }
    public static var textSecondary: Color { GlassTokens.Color.textSecondary.color }
    public static var textTertiary: Color { GlassTokens.Color.textTertiary.color }
    public static var accentText: Color { GlassTokens.Color.purpleText.color }
    public static var hairline: Color { GlassTokens.Color.hairline.color }
}

/// Status is carried by a dot and a label, never by a fill.
public enum GlassStatus: Sendable, Equatable {
    case on, ask, off, outside, shared, kept, inference

    public var rgba: GlassRGBA {
        switch self {
        case .on: GlassTokens.Color.statusOn
        case .ask: GlassTokens.Color.statusAsk
        case .off: GlassTokens.Color.statusOff
        case .outside: GlassTokens.Color.statusOutside
        case .shared: GlassTokens.Color.dataShared
        case .kept: GlassTokens.Color.dataKept
        case .inference: GlassTokens.Color.dataInference
        }
    }

    public var color: Color { rgba.color }
}

// MARK: - Type

private struct GlassTypeModifier: ViewModifier {
    let style: GlassTypeStyle
    /// Read so the modifier re-runs when the system text size changes:
    /// leading and tracking are resolved against the drawn size, which
    /// AppKit has already scaled, so the value itself is not applied again.
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func body(content: Content) -> some View {
        _ = dynamicTypeSize
        return content
            .font(style.font)
            .tracking(style.resolvedTracking)
            .lineSpacing(style.lineSpacing)
            .textCase(style.uppercase ? .uppercase : nil)
    }
}

public extension View {
    /// Set text in a step of the type scale (SF Pro, or SF Mono for `.mono`).
    func glassType(_ style: GlassTypeStyle) -> some View {
        modifier(GlassTypeModifier(style: style))
    }
}

// MARK: - Glyphs

public extension View {
    /// Size an SF Symbol or a mark inside a control of fixed size: a
    /// chevron, a check, a tool's initials in a 22pt tile. Glyphs keep their
    /// points because the control around them does; words never use this,
    /// they use `glassType(_:)` and follow the system text size.
    func glassGlyph(_ size: CGFloat, weight: GlassWeight = .regular) -> some View {
        font(.system(size: size, weight: weight.font))
    }
}

// MARK: - Edges

private struct GlassEdgeModifier<S: InsettableShape>: ViewModifier {
    let layers: [GlassShadow]
    let shape: S

    func body(content: Content) -> some View {
        let outer = layers.filter { !$0.inset }
        let inner = layers.filter(\.inset)
        return content
            .overlay {
                ZStack {
                    ForEach(Array(inner.enumerated()), id: \.offset) { _, layer in
                        // A highlight or shade is a stroke on the inside of
                        // the shape, offset toward the lit or shaded side
                        // and clipped back to the shape.
                        shape
                            .strokeBorder(layer.color.color, lineWidth: max(1, layer.blur + 1))
                            .offset(x: layer.x, y: layer.y)
                            .blur(radius: layer.blur / 2)
                            .clipShape(shape)
                    }
                }
                .allowsHitTesting(false)
            }
            .background {
                ZStack {
                    ForEach(Array(outer.enumerated()), id: \.offset) { _, layer in
                        shape
                            .fill(Color.black.opacity(0.001))
                            .shadow(
                                color: layer.color.color,
                                radius: layer.blur / 2,
                                x: layer.x,
                                y: layer.y
                            )
                    }
                }
                .allowsHitTesting(false)
            }
    }
}

public extension View {
    /// Draw an edge from the token's layers: inset layers as specular
    /// highlights and shades inside `shape`, outer layers as drop shadows.
    /// Containers get edges, never solid borders.
    func glassEdge<S: InsettableShape>(_ layers: [GlassShadow], in shape: S) -> some View {
        modifier(GlassEdgeModifier(layers: layers, shape: shape))
    }
}

// MARK: - Tiers

/// The material tiers. Panes are the containment; cards tint a pane; wells
/// recess into a card; controls sit on top. Never glass on glass.
public enum GlassTier: Sendable, Equatable {
    case pane, card, cardQuiet, well, control, controlSelected, popover, menu, nodeCard

    public var radius: CGFloat {
        switch self {
        case .pane: GlassTokens.Radius.pane
        case .card, .popover, .nodeCard: GlassTokens.Radius.card
        case .cardQuiet, .menu: GlassTokens.Radius.cardQuiet
        case .well, .control, .controlSelected: GlassTokens.Radius.pill
        }
    }

    public var edge: [GlassShadow] {
        switch self {
        case .pane: GlassTokens.Shadow.paneEdge
        case .card: GlassTokens.Shadow.cardEdge
        case .cardQuiet: GlassTokens.Shadow.cardEdgeQuiet
        case .well: GlassTokens.Shadow.wellEdge
        case .control: GlassTokens.Shadow.controlEdge
        case .controlSelected: GlassTokens.Shadow.controlSelectedEdge
        case .popover: GlassTokens.Shadow.popoverEdge
        case .menu: GlassTokens.Shadow.menuEdge
        case .nodeCard: GlassTokens.Shadow.nodeCardEdge
        }
    }

    /// The fill. A pane is the native material (R3, D4): Liquid Glass on
    /// macOS 26, the HUD material on 14–25, the opaque base under Reduce
    /// Transparency. Everything inside a pane is painted: never glass on
    /// glass.
    @ViewBuilder
    func fill(in shape: RoundedRectangle) -> some View {
        switch self {
        case .pane:
            GlassPaneFill(radius: shape.cornerSize.width)
        case .card:
            shape.fill(GlassTokens.Gradient.cardFill.linear)
        case .cardQuiet:
            shape.fill(GlassTokens.Gradient.cardFillQuiet.linear)
        case .well:
            shape.fill(GlassTokens.Color.wellFill.color)
        case .control:
            shape.fill(GlassTokens.Gradient.controlFill.linear)
        case .controlSelected:
            shape.fill(GlassTokens.Color.controlSelected.color)
        case .popover:
            shape.fill(GlassTokens.Color.popoverFill.color)
        case .menu:
            shape.fill(GlassTokens.Color.menuFill.color)
        case .nodeCard:
            shape.fill(GlassTokens.Color.nodeCardFill.color)
        }
    }
}

public extension View {
    /// Put this view on a material tier: its fill, its edge, its radius.
    func glassTier(_ tier: GlassTier, radius: CGFloat? = nil) -> some View {
        modifier(GlassTierModifier(tier: tier, radius: radius))
    }
}

private struct GlassTierModifier: ViewModifier {
    let tier: GlassTier
    let radius: CGFloat?
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius ?? tier.radius, style: .continuous)
        let material = GlassMaterial.current(reduceTransparency: reduceTransparency)
        // Clip the content and fill first; the edge's drop shadows fall
        // outside the shape and must not be clipped with them.
        return content
            .background { tier.fill(in: shape) }
            .clipShape(shape)
            .glassEdge(tier.drawsOwnEdge(on: material) ? tier.edge : [], in: shape)
            .contentShape(shape)
    }
}

extension GlassTier {
    /// Whether this tier draws its own edge. A pane on Liquid Glass does
    /// not: `NSGlassEffectView` draws its own rim, and ours on top doubles
    /// it. Everything else, and every tier before macOS 26, draws its edge.
    func drawsOwnEdge(on material: GlassMaterial) -> Bool {
        !(self == .pane && material == .liquidGlass)
    }
}
