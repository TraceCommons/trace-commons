import SwiftUI

public extension EnvironmentValues {
    /// Room a pane leaves at its top for the window's controls (the traffic
    /// lights). Set on whichever pane is leading, zero on the others.
    @Entry var glassWindowControlsInset: CGFloat = 0
}

/// The window: a leading pane, the center, and an inspector, floating with
/// a gap between them and no chrome around them (D9 as decided on #1173:
/// #1146's floating layout). The leading pane and the inspector hide
/// independently; the center always stays. Whichever pane is leading takes
/// the window's controls, through `glassWindowControlsInset`.
public struct GlassThreePane<Leading: View, Center: View, Trailing: View>: View {
    private let showsLeading: Bool
    private let showsTrailing: Bool
    private let leading: Leading
    private let center: Center
    private let trailing: Trailing

    public init(
        showsLeading: Bool,
        showsTrailing: Bool,
        @ViewBuilder leading: () -> Leading,
        @ViewBuilder center: () -> Center,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.showsLeading = showsLeading
        self.showsTrailing = showsTrailing
        self.leading = leading()
        self.center = center()
        self.trailing = trailing()
    }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.paneGap) {
            if showsLeading {
                leading
                    .frame(width: GlassTokens.Size.paneLeftWidth)
                    .environment(\.glassWindowControlsInset, GlassTokens.Space.windowControlsInset)
                    .transition(.move(edge: .leading).combined(with: .opacity))
            }
            center
                .frame(minWidth: GlassTokens.Size.mapWidth * 0.6, maxWidth: .infinity)
                .environment(\.glassWindowControlsInset, showsLeading ? 0 : GlassTokens.Space.windowControlsInset)
            if showsTrailing {
                trailing
                    .frame(width: GlassTokens.Size.inspectorWidth)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
        .frame(maxHeight: .infinity)
        // The panes run to the window's top edge; the title bar's controls
        // sit inside the leading pane rather than in a strip above it.
        .ignoresSafeArea(.container, edges: .top)
        .animation(.easeInOut(duration: GlassTokens.Motion.standard), value: showsLeading)
        .animation(.easeInOut(duration: GlassTokens.Motion.standard), value: showsTrailing)
    }

    /// The window's default width with every pane showing.
    public static var defaultWidth: CGFloat {
        GlassTokens.Size.paneLeftWidth + GlassTokens.Size.mapWidth + GlassTokens.Size.inspectorWidth
            + GlassTokens.Space.paneGap * 2
    }
}
