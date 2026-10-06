import SwiftUI

public extension EnvironmentValues {
    /// Room the main pane leaves at its top for the window's controls (the
    /// traffic lights): this tall, and `windowControlsWidth` wide.
    @Entry var glassWindowControlsInset: CGFloat = 0
    /// True while the window is too narrow to draw the map, whatever the
    /// preference. The map toggle reads it: it cannot show the map then.
    @Entry var glassMapCompacted: Bool = false
}

/// The widths of the window's panes for one window width (spec, "Panes").
///
/// - The main pane is `min(400, max(320, 0.34 × width))`, and fills the
///   window when the map is hidden, less the inspector and its gap.
/// - The map takes what remains. Below 1100pt it is hidden whatever the
///   preference (a compact-layout rule): the preference is kept, so
///   widening the window brings it back.
/// - The inspector is 300pt.
/// - 10pt window padding, 10pt gaps; a hidden pane reserves no space.
public struct GlassPaneLayout: Equatable, Sendable {
    public let main: CGFloat
    /// `nil` when the map is not drawn.
    public let map: CGFloat?
    /// `nil` when the inspector is not drawn.
    public let inspector: CGFloat?

    public init(windowWidth width: CGFloat, showsMap: Bool, showsInspector: Bool) {
        let padding = GlassTokens.Space.windowPadding
        let gap = GlassTokens.Space.paneGap
        let inner = max(0, width - padding * 2)
        let inspector = showsInspector ? GlassTokens.Size.inspectorWidth : nil
        let besideMain = inspector.map { $0 + gap } ?? 0
        if showsMap && width >= GlassTokens.Size.mapBreakpoint {
            let main = min(
                GlassTokens.Size.paneLeftWidth,
                max(GlassTokens.Size.paneLeftMinWidth, GlassTokens.Size.paneLeftWindowShare * width))
            self.main = main
            self.map = max(0, inner - main - gap - besideMain)
        } else {
            self.main = max(0, inner - besideMain)
            self.map = nil
        }
        self.inspector = inspector
    }

    /// The panes a window opens with the first time, before the person has
    /// chosen (spec, "Opening state and restoration"): the map from 1100pt,
    /// the inspector from 900pt, as #1146 decides at launch. After that the
    /// window restores the person's own choice.
    public static func firstLaunch(windowWidth width: CGFloat) -> (showsMap: Bool, showsInspector: Bool) {
        (width >= GlassTokens.Size.mapBreakpoint, width >= GlassTokens.Size.inspectorBreakpoint)
    }

    /// The map is wanted but the window is too narrow to draw it.
    public static func compactsMap(windowWidth width: CGFloat, showsMap: Bool) -> Bool {
        showsMap && width < GlassTokens.Size.mapBreakpoint
    }

    /// Which panes are drawn. Pane show and hide animate on this, not on
    /// the widths, so dragging the window's edge resizes without easing.
    public struct Visibility: Equatable, Sendable {
        public let map: Bool
        public let inspector: Bool
    }

    public var visibility: Visibility { Visibility(map: map != nil, inspector: inspector != nil) }

    /// The smallest window every composition fits: the main pane at its
    /// minimum beside the inspector (the map hides below 1100pt).
    public static var minimumWindowWidth: CGFloat {
        max(
            GlassTokens.Size.windowMinWidth,
            GlassTokens.Size.paneLeftMinWidth + GlassTokens.Space.paneGap + GlassTokens.Size.inspectorWidth
                + GlassTokens.Space.windowPadding * 2)
    }
}

/// The window: the main pane (the tabs), the map, and the inspector,
/// floating with a gap between them and no chrome around them (D9 as decided
/// on #1173: #1146's floating layout). The map and the inspector hide
/// independently; the main pane always stays and takes the window's
/// controls through `glassWindowControlsInset`.
public struct GlassThreePane<Main: View, Map: View, Inspector: View>: View {
    private let showsMap: Bool
    private let showsInspector: Bool
    private let main: Main
    private let map: Map
    private let inspector: Inspector
    private let onFirstLayout: ((CGFloat) -> Void)?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `showsMap` and `showsInspector` are the person's preferences; the map
    /// is also hidden below the compact breakpoint without changing them.
    /// `onFirstLayout` gets the window's width once, when it first lays
    /// out: the moment to seed preferences nobody has chosen yet.
    public init(
        showsMap: Bool,
        showsInspector: Bool,
        onFirstLayout: ((CGFloat) -> Void)? = nil,
        @ViewBuilder main: () -> Main,
        @ViewBuilder map: () -> Map,
        @ViewBuilder inspector: () -> Inspector
    ) {
        self.showsMap = showsMap
        self.showsInspector = showsInspector
        self.onFirstLayout = onFirstLayout
        self.main = main()
        self.map = map()
        self.inspector = inspector()
    }

    public var body: some View {
        GeometryReader { proxy in
            let layout = GlassPaneLayout(
                windowWidth: proxy.size.width, showsMap: showsMap, showsInspector: showsInspector)
            HStack(spacing: GlassTokens.Space.paneGap) {
                main
                    .frame(width: layout.main)
                    .environment(\.glassWindowControlsInset, GlassTokens.Space.windowControlsInset)
                    .environment(
                        \.glassMapCompacted,
                        GlassPaneLayout.compactsMap(windowWidth: proxy.size.width, showsMap: showsMap))
                if let width = layout.map {
                    map
                        .frame(width: width)
                        .transition(reduceMotion ? .identity : .opacity)
                }
                if let width = layout.inspector {
                    inspector
                        .frame(width: width)
                        .transition(reduceMotion ? .identity : .move(edge: .trailing).combined(with: .opacity))
                }
            }
            .padding(GlassTokens.Space.windowPadding)
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
            .animation(GlassMotion.standard(reduceMotion), value: layout.visibility)
            .onAppear { onFirstLayout?(proxy.size.width) }
        }
        // The panes run to the window's top edge; the title bar's controls
        // sit inside the main pane rather than in a strip above it.
        .ignoresSafeArea(.container, edges: .top)
        .frame(minWidth: GlassPaneLayout.minimumWindowWidth, minHeight: GlassTokens.Size.windowMinHeight)
    }

    /// The window's default width (1320, as in #1146).
    public static var defaultWidth: CGFloat { GlassTokens.Size.windowWidth }
}
