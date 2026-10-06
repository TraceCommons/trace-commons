import AppKit
import SwiftUI

public extension EnvironmentValues {
    /// Room the main pane leaves at its top for the window's controls (the
    /// traffic lights): this tall, and `windowControlsWidth` wide.
    @Entry var glassWindowControlsInset: CGFloat = 0
}

/// The widths of the window's panes for one window width (spec, "Panes",
/// as the owner revised it on #1241).
///
/// - The main pane is a compact fixed width (`paneLeftCompactWidth`). It
///   never changes when the map or the inspector opens or closes.
/// - The inspector is a fixed width (`inspectorWidth`).
/// - The map opens at its own width (`mapWidth`) and is the one pane that
///   takes a resize of the window, never below `mapMinWidth`.
/// - Opening a pane grows the window to the right by that pane and its gap;
///   closing it shrinks the window by the same (`windowWidth(current:from:to:)`),
///   so no other pane moves or changes width.
/// - 10pt window padding, 10pt gaps; a hidden pane reserves no space.
public struct GlassPaneLayout: Equatable, Sendable {
    public let main: CGFloat
    /// `nil` when the map is not drawn.
    public let map: CGFloat?
    /// `nil` when the inspector is not drawn.
    public let inspector: CGFloat?

    public init(windowWidth width: CGFloat, showsMap: Bool, showsInspector: Bool) {
        let fixed = Self.fixedWidth(showsInspector: showsInspector)
        let inspector = showsInspector ? GlassTokens.Size.inspectorWidth : nil
        let main = GlassTokens.Size.paneLeftCompactWidth
        if showsMap {
            self.main = main
            self.map = max(0, width - fixed - GlassTokens.Space.paneGap)
        } else {
            // Only a window wider than its panes (zoomed, or full screen)
            // has room left over; the main pane takes it rather than a gap.
            self.main = main + max(0, width - fixed)
            self.map = nil
        }
        self.inspector = inspector
    }

    /// The window's width with the main pane and, when shown, the inspector:
    /// everything except the map and its gap.
    static func fixedWidth(showsInspector: Bool) -> CGFloat {
        GlassTokens.Space.windowPadding * 2 + GlassTokens.Size.paneLeftCompactWidth
            + (showsInspector ? GlassTokens.Space.paneGap + GlassTokens.Size.inspectorWidth : 0)
    }

    /// The window's width with every shown pane at its default width.
    public static func windowWidth(showsMap: Bool, showsInspector: Bool) -> CGFloat {
        fixedWidth(showsInspector: showsInspector)
            + (showsMap ? GlassTokens.Space.paneGap + GlassTokens.Size.mapWidth : 0)
    }

    /// The narrowest the window may be for these panes: the map at its
    /// minimum, every other pane at its fixed width.
    public static func minimumWindowWidth(showsMap: Bool, showsInspector: Bool) -> CGFloat {
        fixedWidth(showsInspector: showsInspector)
            + (showsMap ? GlassTokens.Space.paneGap + GlassTokens.Size.mapMinWidth : 0)
    }

    /// The widest the window may be for these panes: without the map
    /// nothing takes extra width, so the window is exactly its panes.
    public static func maximumWindowWidth(showsMap: Bool, showsInspector: Bool) -> CGFloat {
        showsMap ? .infinity : fixedWidth(showsInspector: showsInspector)
    }

    /// The window's new width when panes open or close: the current width,
    /// plus each pane that opened (at its default width) and its gap, less
    /// each pane that closed (at the width it had) and its gap. The other
    /// panes keep their widths; the map keeps a width the person dragged it
    /// to when the inspector opens or closes.
    public static func windowWidth(current: CGFloat, from old: Visibility, to new: Visibility) -> CGFloat {
        let before = GlassPaneLayout(windowWidth: current, showsMap: old.map, showsInspector: old.inspector)
        let gap = GlassTokens.Space.paneGap
        var width = current
        if old.map != new.map {
            width += new.map ? GlassTokens.Size.mapWidth + gap : -((before.map ?? 0) + gap)
        }
        if old.inspector != new.inspector {
            width += (new.inspector ? 1 : -1) * (GlassTokens.Size.inspectorWidth + gap)
        }
        return max(minimumWindowWidth(showsMap: new.map, showsInspector: new.inspector), width)
    }

    /// The window's frame at `width`: the top edge and the leading edge stay
    /// where they are and the window grows or shrinks on its right. A window
    /// that would run off the screen's visible frame moves left only as far
    /// as it must; one wider than the screen is held to the screen, and the
    /// map, the one flexible pane, takes the difference.
    public static func windowFrame(_ frame: CGRect, width: CGFloat, visible: CGRect) -> CGRect {
        let width = min(width, visible.width)
        var x = frame.minX
        if x + width > visible.maxX { x = visible.maxX - width }
        x = max(visible.minX, x)
        // AppKit's origin is the bottom-left corner: keeping the top fixed
        // keeps `maxY`, and the height does not change.
        return CGRect(x: x, y: frame.minY, width: width, height: frame.height)
    }

    /// The panes a window opens with the first time, before the person has
    /// chosen (spec, "Opening state and restoration"): the map when the
    /// screen has 1100pt for it, the inspector from 900pt, as #1146 decides
    /// at launch. After that the window restores the person's own choice.
    public static func firstLaunch(windowWidth width: CGFloat) -> (showsMap: Bool, showsInspector: Bool) {
        (width >= GlassTokens.Size.mapBreakpoint, width >= GlassTokens.Size.inspectorBreakpoint)
    }

    /// Which panes are drawn. Pane show and hide animate on this, not on
    /// the widths, so dragging the window's edge resizes without easing.
    public struct Visibility: Equatable, Sendable {
        public let map: Bool
        public let inspector: Bool

        public init(map: Bool, inspector: Bool) {
            self.map = map
            self.inspector = inspector
        }
    }

    public var visibility: Visibility { Visibility(map: map != nil, inspector: inspector != nil) }

    /// The smallest window any composition fits: the main pane alone.
    public static var minimumWindowWidth: CGFloat {
        minimumWindowWidth(showsMap: false, showsInspector: false)
    }
}

/// The window: the main pane (the tabs), the map, and the inspector,
/// floating with a gap between them and no chrome around them (D9 as decided
/// on #1173: #1146's floating layout). The map and the inspector hide
/// independently; the main pane always stays and takes the window's
/// controls through `glassWindowControlsInset`. Showing or hiding a pane
/// grows or shrinks the window on its right (`GlassPaneLayout`).
public struct GlassThreePane<Main: View, Map: View, Inspector: View>: View {
    private let showsMap: Bool
    private let showsInspector: Bool
    private let main: Main
    private let map: Map
    private let inspector: Inspector
    private let onFirstLayout: ((CGFloat) -> Void)?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var host = GlassWindowHost()

    /// `showsMap` and `showsInspector` are the person's preferences.
    /// `onFirstLayout` gets the width the window has to open into once, when
    /// it is first in a window (its screen's visible width): the moment to
    /// seed preferences nobody has chosen yet.
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

    private var wanted: GlassPaneLayout.Visibility {
        GlassPaneLayout.Visibility(map: showsMap, inspector: showsInspector)
    }

    public var body: some View {
        GeometryReader { proxy in
            let layout = GlassPaneLayout(
                windowWidth: proxy.size.width, showsMap: showsMap, showsInspector: showsInspector)
            HStack(spacing: GlassTokens.Space.paneGap) {
                main
                    .frame(width: layout.main)
                    .environment(\.glassWindowControlsInset, GlassTokens.Space.windowControlsInset)
                if let width = layout.map {
                    map
                        .frame(width: width)
                        .transition(reduceMotion ? .identity : .opacity)
                }
                if let width = layout.inspector {
                    inspector
                        .frame(width: width)
                        .transition(reduceMotion ? .identity : .opacity)
                }
            }
            .padding(GlassTokens.Space.windowPadding)
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
            .animation(GlassMotion.standard(reduceMotion), value: layout.visibility)
        }
        // The width the panes were last laid out at: the window's width
        // before a pane opened or closed, whatever its size limits do next.
        .onGeometryChange(for: CGFloat.self, of: \.size.width) { host.width = $0 }
        // The panes run to the window's top edge; the title bar's controls
        // sit inside the main pane rather than in a strip above it.
        .ignoresSafeArea(.container, edges: .top)
        .frame(
            minWidth: GlassPaneLayout.minimumWindowWidth(showsMap: showsMap, showsInspector: showsInspector),
            maxWidth: GlassPaneLayout.maximumWindowWidth(showsMap: showsMap, showsInspector: showsInspector),
            minHeight: GlassTokens.Size.windowMinHeight, maxHeight: .infinity)
        .background(GlassWindowProbe { window in
            guard host.window !== window else { return }
            host.window = window
            onFirstLayout?(window.screen?.visibleFrame.width ?? window.frame.width)
        }.frame(width: 0, height: 0))
        .onChange(of: wanted) { old, new in
            resize(from: old, to: new)
        }
    }

    /// Grows or shrinks the window by the pane that opened or closed, so no
    /// other pane changes width. A full-screen window has no edge to move.
    private func resize(from old: GlassPaneLayout.Visibility, to new: GlassPaneLayout.Visibility) {
        guard let window = host.window, !window.styleMask.contains(.fullScreen) else { return }
        let width = GlassPaneLayout.windowWidth(current: host.width ?? window.frame.width, from: old, to: new)
        let visible = window.screen?.visibleFrame ?? CGRect(x: -1e6, y: -1e6, width: 2e6, height: 2e6)
        let frame = GlassPaneLayout.windowFrame(window.frame, width: width, visible: visible)
        guard frame != window.frame else { return }
        window.setFrame(frame, display: true, animate: !reduceMotion)
    }

    /// The window's default width: every pane shown at its default width.
    public static var defaultWidth: CGFloat {
        GlassPaneLayout.windowWidth(showsMap: true, showsInspector: true)
    }
}

/// The window a `GlassThreePane` is in, once it is in one. A reference, so
/// learning it does not redraw the panes.
@MainActor
private final class GlassWindowHost {
    weak var window: NSWindow?
    var width: CGFloat?
}

/// Reports the window the view lands in.
private struct GlassWindowProbe: NSViewRepresentable {
    let onWindow: @MainActor (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = Probe()
        view.onWindow = onWindow
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {}

    private final class Probe: NSView {
        var onWindow: (@MainActor (NSWindow) -> Void)?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let window { onWindow?(window) }
        }
    }
}
