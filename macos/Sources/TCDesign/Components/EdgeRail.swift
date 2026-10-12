import CoreGraphics
import SwiftUI

/// Where the edge rail's panel sits on a screen, from the design's measures
/// ("Run 1 OS-level Explorations", 1b), all of them tokens (`size.edgeRail*`,
/// `space.edgeRail*`): a thin hover zone on the right edge while closed, and,
/// while open, the icon rail inset from the edge with its peek beside it.
public enum EdgeRailGeometry {
    /// Room left of the peek for its edge and shadow to draw into.
    public static let shadowRoom: CGFloat = GlassTokens.Space.edgeRailGap + GlassTokens.Space.s2

    public static var openWidth: CGFloat {
        shadowRoom + GlassTokens.Size.edgeRailPeekWidth + GlassTokens.Space.edgeRailGap
            + GlassTokens.Size.edgeRailWidth + GlassTokens.Space.edgeRailInset
    }

    /// The panel's frame on a screen whose visible frame is `visible`:
    /// against its right edge, centred vertically, never taller than it.
    public static func frame(open: Bool, visible: CGRect) -> CGRect {
        let width = open ? openWidth : GlassTokens.Size.edgeRailZoneWidth
        let height = min(open ? GlassTokens.Size.edgeRailOpenHeight : GlassTokens.Size.edgeRailZoneHeight, visible.height)
        return CGRect(x: visible.maxX - width, y: visible.midY - height / 2, width: width, height: height)
    }
}

/// One tile on the edge rail: a glyph button on the control tiers, as the
/// segmented tabs draw a segment. No fill at rest, `controlHover` under the
/// pointer, and, selected, `controlSelected` with its edge and the label in
/// `textOnSelected` (the brand purple and white in the flat theme). The
/// press darkens the fill, never the glyph.
public struct GlassRailTile<Label: View>: View {
    private let selected: Bool
    private let action: () -> Void
    private let label: Label

    public init(selected: Bool, action: @escaping () -> Void, @ViewBuilder label: () -> Label) {
        self.selected = selected
        self.action = action
        self.label = label()
    }

    static var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: GlassTokens.Radius.cardQuiet, style: .continuous)
    }

    /// The glyph's ink: `textOnSelected` on the selection, else primary.
    static func ink(selected: Bool) -> GlassRGBA {
        selected ? GlassTokens.Color.textOnSelected : GlassTokens.Color.textPrimary
    }

    public var body: some View {
        Button(action: action) {
            label
                .foregroundStyle(Self.ink(selected: selected).color)
                .frame(width: GlassTokens.Size.edgeRailTile, height: GlassTokens.Size.edgeRailTile)
                .background {
                    if selected {
                        Self.shape.fill(GlassTokens.Color.controlSelected.color)
                            .glassPressedFill()
                            .glassEdge(GlassTokens.Shadow.controlSelectedEdge, in: Self.shape)
                    } else {
                        // Hover replaces the (empty) fill; never over the selection.
                        Color.clear.glassHover(GlassTokens.Color.controlHover, in: Self.shape)
                    }
                }
                .glassPressedWash(Self.shape)
                .contentShape(Self.shape)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
    }
}

/// The closed edge rail's handle: the `edgeRailHandle` bar on the screen's
/// right edge, `edgeRailHandleWidth` by `edgeRailHandleHeight`.
public struct GlassRailHandle: View {
    public init() {}

    public var body: some View {
        RoundedRectangle(cornerRadius: GlassTokens.Size.edgeRailHandleWidth / 2 + 0.5)
            .fill(GlassTokens.Color.edgeRailHandle.color)
            .frame(width: GlassTokens.Size.edgeRailHandleWidth, height: GlassTokens.Size.edgeRailHandleHeight)
    }
}

/// The rule between the rail's section tiles and the app's tile.
public struct GlassRailRule: View {
    public init() {}

    public var body: some View {
        Rectangle().fill(GlassTokens.Color.rule.color)
            .frame(width: GlassTokens.Size.edgeRailRuleWidth, height: 1)
            .padding(.vertical, GlassTokens.Space.s2)
            .accessibilityHidden(true)
    }
}
