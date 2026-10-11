import SwiftUI

/// Shorthand for the token colours as SwiftUI colours. The secondary and
/// tertiary text and the hairline follow Increase Contrast by themselves
/// (R14), as the system's semantic colours do.
public enum GlassColor {
    public static var textPrimary: Color { GlassTokens.Color.textPrimary.color }
    public static var textSecondary: Color {
        GlassTokens.Color.textSecondary.adaptive(highContrast: GlassTokens.Color.textSecondaryHighContrast)
    }
    public static var textTertiary: Color {
        GlassTokens.Color.textTertiary.adaptive(highContrast: GlassTokens.Color.textTertiaryHighContrast)
    }
    public static var accentText: Color { GlassTokens.Color.purpleText.color }
    public static var hairline: Color {
        GlassTokens.Color.hairline.adaptive(highContrast: GlassTokens.Color.hairlineHighContrast)
    }

    /// An overlay at `alpha`: white over the dark appearance, black over the
    /// light one, for strokes and fills drawn over a surface.
    public static func ink(_ alpha: Double) -> Color {
        GlassTokens.Color.ink.opacity(alpha).color
    }
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

    /// The colour for this status drawn as text. On, ask and outside have
    /// text-safe variants that reach 4.5:1 in light (their glyph colours
    /// are tested only at the 3:1 non-text floor). Off is textSecondary, as
    /// statusOff is never for text (a Paused chip was 2.59:1 in flat Light).
    public var textRGBA: GlassRGBA {
        switch self {
        case .on: GlassTokens.Color.statusOnText
        case .ask: GlassTokens.Color.statusAskText
        case .outside: GlassTokens.Color.statusOutsideText
        case .off: GlassTokens.Color.textSecondary
        case .shared, .kept, .inference: rgba
        }
    }

    /// Off is the adaptive textSecondary, so a Paused chip deepens under
    /// Increase Contrast as a chip with no status does.
    public var textColor: Color { self == .off ? GlassColor.textSecondary : textRGBA.color }
}

// MARK: - Type

private struct GlassTypeModifier: ViewModifier {
    let style: GlassTypeStyle

    // Leading and tracking are resolved against the size AppKit draws the
    // text style at. SwiftUI's `dynamicTypeSize` does not scale macOS text
    // styles, so it is not read here.
    func body(content: Content) -> some View {
        content
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
    /// chevron, a check, the initials a tool with no logo shows in its 22pt
    /// tile. Glyphs keep their points because the control around them does;
    /// words never use this, they use `glassType(_:)` and follow the system
    /// text size.
    func glassGlyph(_ size: CGFloat, weight: GlassWeight = .regular) -> some View {
        font(.system(size: size, weight: weight.font))
    }
}

// MARK: - Edges

private struct GlassEdgeModifier<S: InsettableShape>: ViewModifier {
    let layers: [GlassShadow]
    let shape: S
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        let outer = layers.filter { !$0.inset }
        // Increase Contrast (spec, Appearance): the soft light-and-shade
        // edge becomes a solid 1pt stroke at 40% text colour. Only painted
        // surfaces draw this edge; Liquid Glass takes the system's own
        // contrasting border instead (R14).
        let increased = contrast == .increased && !layers.isEmpty
        let inner = increased ? [] : layers.filter(\.inset)
        return content
            .overlay {
                if increased {
                    shape
                        .strokeBorder(GlassTokens.Color.edgeHighContrast.color, lineWidth: 1)
                        .allowsHitTesting(false)
                }
            }
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
    ///
    /// `hover` is the fill the tier takes under the pointer while enabled:
    /// it replaces the tier's own fill, as #1146's `:hover` background does
    /// (owner ruling, 2026-10-07: hover states from #1146), and never stacks
    /// on it. `edge` replaces the tier's edge (the map's `mapEdge`).
    func glassTier(
        _ tier: GlassTier, radius: CGFloat? = nil, hover: GlassRGBA? = nil, edge: [GlassShadow]? = nil
    ) -> some View {
        modifier(GlassTierModifier(tier: tier, radius: radius, hover: hover, edge: edge))
    }
}

/// Which fill a tier draws: its own, or its hover fill in its place.
enum GlassTierFill: Equatable {
    case tier
    case hover(GlassRGBA)

    static func choose(hover: GlassRGBA?, hovering: Bool, enabled: Bool) -> GlassTierFill {
        guard let hover, GlassHover.shows(hovering: hovering, enabled: enabled) else { return .tier }
        return .hover(hover)
    }
}

private struct GlassTierModifier: ViewModifier {
    let tier: GlassTier
    let radius: CGFloat?
    let hover: GlassRGBA?
    let edge: [GlassShadow]?
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius ?? tier.radius, style: .continuous)
        let fill = GlassTierFill.choose(hover: hover, hovering: hovering, enabled: isEnabled)
        // Clip the content and fill first; the edge's drop shadows fall
        // outside the shape and must not be clipped with them. Every tier
        // draws its edge, a pane on Liquid Glass too, as #1146's `.tc-pane`
        // keeps `--tc-pane-edge` over native glass (owner ruling, 2026-10-07).
        return content
            .background {
                ZStack {
                    tier.fill(in: shape).opacity(fill == .tier ? 1 : 0)
                    if let hover {
                        shape.fill(hover.color).opacity(fill == .tier ? 0 : 1)
                    }
                }
                .glassPressedFill()
                .animation(GlassMotion.fast(reduceMotion), value: fill)
            }
            .clipShape(shape)
            .glassEdge(edge ?? tier.edge, in: shape)
            .contentShape(shape)
            .onHover { inside in
                if hover != nil { hovering = inside }
            }
    }
}

// MARK: - Floating layer (R4)

/// Where a surface sits. Inside a pane everything is painted (never glass
/// on glass). Over content, such as the map, controls and cards float on
/// their own glass: Liquid Glass on macOS 26, the painted tier before it.
public enum GlassLayer: Sendable, Equatable {
    case content
    case floating
}

public extension EnvironmentValues {
    @Entry var glassLayer: GlassLayer = .content
}

private struct GlassSurfaceModifier: ViewModifier {
    let tier: GlassTier
    let radius: CGFloat?
    let floating: Bool?
    let hover: GlassRGBA?
    @Environment(\.glassLayer) private var layer

    func body(content: Content) -> some View {
        // Reduce Transparency is the system's to apply: the HUD blur turns
        // opaque by itself (R14).
        switch GlassSurfaceBacking.choose(floating: floating ?? (layer == .floating)) {
        case .blur:
            // The painted tier over a real blur of what it floats on, so a
            // popover or node card reads as glass and not as a flat
            // translucent fill: #1146's fill, edge and backdrop blur, on
            // macOS 26 too (owner ruling, 2026-10-07).
            content
                .glassTier(tier, radius: radius, hover: hover)
                .background { GlassFloatingBlur(cornerRadius: radius ?? tier.radius) }
        case .painted:
            content.glassTier(tier, radius: radius, hover: hover)
        }
    }
}

/// What a surface is drawn over.
enum GlassSurfaceBacking: Equatable {
    /// The painted tier over a within-window blur (floating), as #1146
    /// draws its floating controls, popovers, menus and node cards: their
    /// fill and edge over `backdrop-filter`, on every macOS (owner ruling,
    /// 2026-10-07).
    case blur
    /// The painted tier alone, inside a pane that is its backing.
    case painted

    /// What a surface floats on. Reduce Transparency does not change it:
    /// under it `NSVisualEffectView` draws opaque by itself, so floating
    /// text never shows the map through (R14).
    static func choose(floating: Bool) -> GlassSurfaceBacking {
        floating ? .blur : .painted
    }
}

public extension View {
    /// Put this view on a tier that may float. `floating: nil` follows the
    /// surrounding `glassLayer`; `true` or `false` fixes it (a menu always
    /// floats; a well never does). `hover` replaces the tier's fill under
    /// the pointer (`glassTier`).
    func glassSurface(
        _ tier: GlassTier, radius: CGFloat? = nil, floating: Bool? = nil, hover: GlassRGBA? = nil
    ) -> some View {
        modifier(GlassSurfaceModifier(tier: tier, radius: radius, floating: floating, hover: hover))
    }
}

/// Controls and cards that float over content together: the map's tabs,
/// its toolbar and its node cards. Everything inside is on the floating
/// layer, and on macOS 26 the glass shapes share one container, so they
/// blend and morph as one surface instead of stacking.
public struct GlassFloatingGroup<Content: View>: View {
    private let spacing: CGFloat
    private let content: Content

    public init(spacing: CGFloat = GlassTokens.Space.s4, @ViewBuilder content: () -> Content) {
        self.spacing = spacing
        self.content = content()
    }

    public var body: some View {
        Group {
            if #available(macOS 26.0, *) {
                GlassEffectContainer(spacing: spacing) { content }
            } else {
                content
            }
        }
        .environment(\.glassLayer, .floating)
    }
}

