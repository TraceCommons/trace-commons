import AppKit
import SwiftUI

public extension EnvironmentValues {
    /// Room the main pane leaves at its top for the window's controls (the
    /// traffic lights): this tall, and `windowControlsWidth` wide.
    @Entry var glassWindowControlsInset: CGFloat = 0
}

/// The widths of the window's panes for one window width (spec, "Panes",
/// as the owner revised it on #1241, and on 2026-10-08).
///
/// - The main pane is a fixed width (`mainWidth`). It never changes when
///   the map or the inspector opens or closes.
/// - The inspector is a fixed width, two thirds of the main pane's
///   (`inspectorWidth`).
/// - The map opens at its own width (`mapWidth`) and is the one pane that
///   takes a resize of the window, never below `mapMinWidth`.
/// - Opening a pane grows the window to the right by that pane and its gap;
///   closing it shrinks the window by the same (`windowWidth(current:from:to:)`),
///   so no other pane moves or changes width.
/// - No window padding (#1146 `.tc-window--floating`), 10pt gaps; a hidden
///   pane reserves no space.
public struct GlassPaneLayout: Equatable, Sendable {
    public let main: CGFloat
    /// `nil` when the map is not drawn.
    public let map: CGFloat?
    /// `nil` when the inspector is not drawn.
    public let inspector: CGFloat?
    /// The gap before the map. Always the pane gap, except while the map
    /// opens or closes: then it is the first part of the map's room to
    /// appear and the last to go, so nothing jumps by a gap.
    public let mapGap: CGFloat

    /// The main pane's width (the owner, 2026-10-08: 400pt, a little wider
    /// than #1146's compact 360; 2026-10-09: a little wider again).
    public static var mainWidth: CGFloat { GlassTokens.Size.paneLeftWidth + 40 }

    /// The inspector's width: two thirds of the main pane's (the owner,
    /// 2026-10-08).
    public static var inspectorWidth: CGFloat { (mainWidth * 2 / 3).rounded() }

    public init(windowWidth width: CGFloat, showsMap: Bool, showsInspector: Bool) {
        let fixed = Self.fixedWidth(showsInspector: showsInspector)
        self.inspector = showsInspector ? Self.inspectorWidth : nil
        self.mapGap = GlassTokens.Space.paneGap
        if showsMap {
            self.main = Self.mainWidth
            self.map = max(0, width - fixed - GlassTokens.Space.paneGap)
        } else {
            // Only a window wider than its panes (zoomed, or full screen)
            // has room left over; the main pane takes it rather than a gap.
            self.main = Self.mainWidth + max(0, width - fixed)
            self.map = nil
        }
    }

    /// The panes while the window moves from one composition to another
    /// (`Transition`). Every pane of either composition is drawn, so a
    /// closing pane stays until the window's edge has passed over it, and
    /// the panes keep the leading edge rather than reflowing:
    ///
    /// - A map that stays keeps the width it had; the window's edge
    ///   uncovers or covers the inspector beside it.
    /// - A map that opens or closes takes whatever room the window has past
    ///   the fixed panes, so it grows from nothing or shrinks to nothing as
    ///   the window does, and the inspector rides along beside it.
    /// - An inspector that opens or closes keeps its width; the window's
    ///   edge reveals or covers it.
    public init(windowWidth width: CGFloat, transition: Transition) {
        let showsInspector = transition.from.inspector || transition.to.inspector
        self.main = Self.mainWidth
        self.inspector = showsInspector ? Self.inspectorWidth : nil
        let gap = GlassTokens.Space.paneGap
        switch (transition.from.map, transition.to.map) {
        case (true, true):
            self.map = transition.mapWidth ?? GlassTokens.Size.mapWidth
            self.mapGap = gap
        case (false, false):
            self.map = nil
            self.mapGap = gap
        default:
            let room = max(0, width - Self.fixedWidth(showsInspector: showsInspector))
            self.mapGap = min(gap, room)
            self.map = room - self.mapGap
        }
    }

    /// The window's width with the main pane and, when shown, the inspector:
    /// everything except the map and its gap.
    static func fixedWidth(showsInspector: Bool) -> CGFloat {
        GlassTokens.Space.windowPadding * 2 + mainWidth
            + (showsInspector ? GlassTokens.Space.paneGap + inspectorWidth : 0)
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

    /// The window's limits while it moves between two compositions: wide
    /// enough for both ends, so neither its start nor its end is clamped
    /// and the window never jumps ahead of its animation.
    public static func windowWidthLimits(_ transition: Transition) -> ClosedRange<CGFloat> {
        let (a, b) = (transition.from, transition.to)
        let lower = min(minimumWindowWidth(showsMap: a.map, showsInspector: a.inspector),
                        minimumWindowWidth(showsMap: b.map, showsInspector: b.inspector))
        let upper = max(maximumWindowWidth(showsMap: a.map, showsInspector: a.inspector),
                        maximumWindowWidth(showsMap: b.map, showsInspector: b.inspector))
        return lower...upper
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
            width += (new.inspector ? 1 : -1) * (inspectorWidth + gap)
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

    /// Which panes are drawn.
    public struct Visibility: Equatable, Sendable {
        public let map: Bool
        public let inspector: Bool

        public init(map: Bool, inspector: Bool) {
            self.map = map
            self.inspector = inspector
        }
    }

    /// A move from one composition to another, while the window resizes.
    public struct Transition: Equatable, Sendable {
        public let from: Visibility
        public let to: Visibility
        /// The map's width when the move began, which a map that stays
        /// keeps throughout.
        public let mapWidth: CGFloat?

        public init(from: Visibility, to: Visibility, mapWidth: CGFloat?) {
            self.from = from
            self.to = to
            self.mapWidth = mapWidth
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
///
/// Showing or hiding a pane is one motion: the window's edge and the pane's
/// fade run on the same clock and curve, the panes stay on the window's
/// leading edge throughout, and the composition the panes are laid out for
/// (`presented`) changes only when the window has arrived. Laid out for the
/// new composition at once, the panes used to jump to it and sit centred in
/// the old width before the window caught up.
public struct GlassThreePane<Main: View, Map: View, Inspector: View>: View {
    private let showsMap: Bool
    private let showsInspector: Bool
    private let main: Main
    private let map: Map
    private let inspector: Inspector
    private let onFirstLayout: ((CGFloat) -> Void)?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var host = GlassWindowHost()
    /// The composition the panes are laid out for while no move is under
    /// way. It trails the preferences by one move.
    @State private var presented: GlassPaneLayout.Visibility
    /// The move under way, if any.
    @State private var transition: GlassPaneLayout.Transition?

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
        _presented = State(initialValue: GlassPaneLayout.Visibility(map: showsMap, inspector: showsInspector))
    }

    private var wanted: GlassPaneLayout.Visibility {
        GlassPaneLayout.Visibility(map: showsMap, inspector: showsInspector)
    }

    /// The window's width limits: the presented composition's, or both
    /// ends' while a move is under way.
    private var widthLimits: ClosedRange<CGFloat> {
        if let transition { return GlassPaneLayout.windowWidthLimits(transition) }
        return GlassPaneLayout.minimumWindowWidth(showsMap: presented.map, showsInspector: presented.inspector)
            ... GlassPaneLayout.maximumWindowWidth(showsMap: presented.map, showsInspector: presented.inspector)
    }

    public var body: some View {
        GeometryReader { proxy in
            let layout = transition.map { GlassPaneLayout(windowWidth: proxy.size.width, transition: $0) }
                ?? GlassPaneLayout(
                    windowWidth: proxy.size.width, showsMap: presented.map, showsInspector: presented.inspector)
            // Every pane is exactly the window's height and starts at its top
            // edge. A pane whose content outgrew the window used to make the
            // row taller than the window, and the row centred the others in
            // it: they moved down off the traffic lights and lost their
            // bottoms. A pane that can outgrow the window scrolls inside.
            let height = max(0, proxy.size.height - GlassTokens.Space.windowPadding * 2)
            // Each pane after the first carries its own leading gap, so a
            // pane growing from nothing grows its gap first.
            HStack(alignment: .top, spacing: 0) {
                main
                    .frame(width: layout.main, height: height)
                    .environment(\.glassWindowControlsInset, GlassTokens.Space.windowControlsInset)
                if let width = layout.map {
                    map
                        .frame(width: width, height: height)
                        .clipped()
                        .opacity(transition?.to.map == false ? 0 : 1)
                        .padding(.leading, layout.mapGap)
                        .transition(.opacity)
                }
                if let width = layout.inspector {
                    inspector
                        .frame(width: width, height: height)
                        .opacity(transition?.to.inspector == false ? 0 : 1)
                        .padding(.leading, GlassTokens.Space.paneGap)
                        .transition(.opacity)
                }
            }
            .padding(GlassTokens.Space.windowPadding)
            // The leading edge, always: a row narrower or wider than the
            // window for a moment stays under the traffic lights, and a
            // pane past the window's trailing edge is simply covered by it.
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
        }
        // The width the panes were last laid out at: the window's width
        // before a pane opened or closed, whatever its size limits do next.
        .onGeometryChange(for: CGFloat.self, of: \.size.width) { host.width = $0 }
        // The panes run to the window's top edge; the title bar's controls
        // sit inside the main pane rather than in a strip above it.
        .ignoresSafeArea(.container, edges: .top)
        .frame(
            minWidth: widthLimits.lowerBound, maxWidth: widthLimits.upperBound,
            minHeight: GlassTokens.Size.windowMinHeight, maxHeight: .infinity, alignment: .topLeading)
        .background(GlassWindowProbe { window in
            guard host.window !== window else { return }
            host.window = window
            onFirstLayout?(window.screen?.visibleFrame.width ?? window.frame.width)
        }.frame(width: 0, height: 0))
        .onChange(of: wanted) { _, new in
            move(to: new)
        }
    }

    /// Moves to `new`: grows or shrinks the window by the pane that opened
    /// or closed, so no other pane changes width, with the panes faded on
    /// the same clock. A full-screen window has no edge to move, and under
    /// Reduce Motion the window and the panes change at once.
    private func move(to new: GlassPaneLayout.Visibility) {
        // A move that interrupts another starts from where that one was
        // going.
        let old = transition?.to ?? presented
        guard old != new else { return }
        host.move += 1
        let move = host.move
        guard let window = host.window, !window.styleMask.contains(.fullScreen) else {
            transition = nil
            presented = new
            return
        }
        let current = host.width ?? window.frame.width
        let width = GlassPaneLayout.windowWidth(current: current, from: old, to: new)
        let visible = window.screen?.visibleFrame ?? CGRect(x: -1e6, y: -1e6, width: 2e6, height: 2e6)
        let frame = GlassPaneLayout.windowFrame(window.frame, width: width, visible: visible)
        if reduceMotion || frame == window.frame {
            transition = nil
            presented = new
            if frame != window.frame { window.setFrame(frame, display: true) }
            return
        }
        let before = GlassPaneLayout(windowWidth: current, showsMap: old.map, showsInspector: old.inspector)
        presented = old
        withAnimation(GlassMotion.curve(GlassTokens.Motion.standard)) {
            transition = GlassPaneLayout.Transition(from: old, to: new, mapWidth: before.map)
        }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = GlassTokens.Motion.standard
            context.timingFunction = CAMediaTimingFunction(
                controlPoints: Float(GlassTokens.Motion.easeX1), Float(GlassTokens.Motion.easeY1),
                Float(GlassTokens.Motion.easeX2), Float(GlassTokens.Motion.easeY2))
            context.allowsImplicitAnimation = true
            window.animator().setFrame(frame, display: true)
        } completionHandler: {
            MainActor.assumeIsolated {
                // A later move has taken over; it finishes the job.
                guard host.move == move else { return }
                presented = new
                transition = nil
            }
        }
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
    /// Counts moves, so a finished animation can tell it was overtaken.
    var move = 0
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
