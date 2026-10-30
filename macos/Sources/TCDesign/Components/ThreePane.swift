import AppKit
import SwiftUI

public extension EnvironmentValues {
    /// Room the main pane leaves at its top for the window's controls (the
    /// traffic lights): this tall, and `windowControlsWidth` wide.
    @Entry var glassWindowControlsInset: CGFloat = 0
}

/// The widths of the window's panes for one window width (spec, "Panes",
/// as the owner revised it on #1241, on 2026-10-08, and on 2026-10-15).
///
/// - The main pane opens at `mainWidth` and never goes below it. It never
///   changes when the map or the inspector opens or closes.
/// - The inspector is a fixed width, two thirds of the main pane's default
///   (`inspectorWidth`).
/// - Without the map, the main pane takes a resize of the window, so the
///   window can always be made wider (owner, 2026-10-15).
/// - With the map, the map takes a resize first, down to `mapMinWidth`;
///   past that the main pane gives up room, down to `mainWidth`, and takes
///   it back first when the window widens again.
/// - The divider in the gap between the main pane and the map moves the
///   room between them (owner, 2026-10-20): dragged right it widens the
///   main pane and narrows the map, down to `mapMinWidth`
///   (`draggedMain(from:by:windowWidth:showsInspector:)`).
/// - Opening a pane grows the window to the right by that pane and its gap;
///   closing it shrinks the window by the same (`windowWidth(current:from:to:main:)`),
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

    /// How far into the map beside it the divider takes the pointer, so
    /// the gap is easier to catch without looking any wider. Not into the
    /// main pane: its scroller runs along the edge beside the gap.
    public static let dividerReach: CGFloat = 4

    /// `main` is the main pane's width the last time the window had no map
    /// (the width the person dragged it to), which it keeps beside the map
    /// while the window has room for the map at its minimum.
    public init(windowWidth width: CGFloat, showsMap: Bool, showsInspector: Bool, main: CGFloat = mainWidth) {
        let room = width - Self.othersWidth(showsInspector: showsInspector)
        self.inspector = showsInspector ? Self.inspectorWidth : nil
        self.mapGap = GlassTokens.Space.paneGap
        if showsMap {
            let shared = room - GlassTokens.Space.paneGap
            self.main = min(max(main, Self.mainWidth), max(Self.mainWidth, shared - GlassTokens.Size.mapMinWidth))
            self.map = max(0, shared - self.main)
        } else {
            // Without the map, the main pane takes all the room there is.
            self.main = max(Self.mainWidth, room)
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
        self.main = transition.mainWidth
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
            let room = max(0, width - Self.othersWidth(showsInspector: showsInspector) - transition.mainWidth)
            self.mapGap = min(gap, room)
            self.map = room - self.mapGap
        }
    }

    /// The window's width without the main pane, the map and the map's gap:
    /// its padding and, when shown, the inspector and its gap.
    static func othersWidth(showsInspector: Bool) -> CGFloat {
        GlassTokens.Space.windowPadding * 2 + (showsInspector ? GlassTokens.Space.paneGap + inspectorWidth : 0)
    }

    /// The window's width with the main pane at its default and, when
    /// shown, the inspector: everything except the map and its gap.
    static func fixedWidth(showsInspector: Bool) -> CGFloat {
        othersWidth(showsInspector: showsInspector) + mainWidth
    }

    /// The window's width with every shown pane at its default width, the
    /// map at `mapWidth`: its default, or the width the person last left it
    /// at (owner, 2026-10-19), never below its minimum. `main` is the main
    /// pane's kept width (owner, 2026-10-20), never below its default.
    public static func windowWidth(
        showsMap: Bool, showsInspector: Bool, mapWidth: CGFloat = GlassTokens.Size.mapWidth,
        main: CGFloat = mainWidth
    ) -> CGFloat {
        fixedWidth(showsInspector: showsInspector) + (max(main, mainWidth) - mainWidth)
            + (showsMap ? GlassTokens.Space.paneGap + max(GlassTokens.Size.mapMinWidth, mapWidth) : 0)
    }

    /// The main pane's width while the divider beside the map is dragged
    /// (owner, 2026-10-20): its width when the drag began plus how far the
    /// pointer has moved, never under `mainWidth` and never so wide that
    /// the map goes under `mapMinWidth`. The window keeps its width.
    public static func draggedMain(
        from start: CGFloat, by dx: CGFloat, windowWidth width: CGFloat, showsInspector: Bool
    ) -> CGFloat {
        GlassPaneLayout(windowWidth: width, showsMap: true, showsInspector: showsInspector, main: start + dx).main
    }

    /// The narrowest the window may be for these panes: the map at its
    /// minimum, every other pane at its fixed width. There is no widest: a
    /// wider window gives the map, or without it the main pane, the room.
    public static func minimumWindowWidth(showsMap: Bool, showsInspector: Bool) -> CGFloat {
        fixedWidth(showsInspector: showsInspector)
            + (showsMap ? GlassTokens.Space.paneGap + GlassTokens.Size.mapMinWidth : 0)
    }

    /// The window's limits while it moves between two compositions: wide
    /// enough for both ends, so neither its start nor its end is clamped
    /// and the window never jumps ahead of its animation.
    public static func windowWidthLimits(_ transition: Transition) -> ClosedRange<CGFloat> {
        let (a, b) = (transition.from, transition.to)
        let lower = min(minimumWindowWidth(showsMap: a.map, showsInspector: a.inspector),
                        minimumWindowWidth(showsMap: b.map, showsInspector: b.inspector))
        return lower...CGFloat.infinity
    }

    /// The window's new width when panes open or close: the current width,
    /// plus each pane that opened and its gap, less each pane that closed
    /// (at the width it had) and its gap. A map that opens comes back at
    /// `mapWidth`, the width the person last left it at, else its default;
    /// an inspector at its fixed width. The other panes keep their widths;
    /// the map keeps a width the person dragged it to when the inspector
    /// opens or closes, and the main pane keeps one whatever opens or
    /// closes. `main` is as `init(windowWidth:showsMap:showsInspector:main:)`
    /// takes it.
    public static func windowWidth(
        current: CGFloat, from old: Visibility, to new: Visibility, main: CGFloat = mainWidth,
        mapWidth: CGFloat = GlassTokens.Size.mapWidth
    ) -> CGFloat {
        let before = GlassPaneLayout(windowWidth: current, showsMap: old.map, showsInspector: old.inspector, main: main)
        let gap = GlassTokens.Space.paneGap
        var width = current
        if old.map != new.map {
            let opened: CGFloat = max(GlassTokens.Size.mapMinWidth, mapWidth) + gap
            width += new.map ? opened : -((before.map ?? 0) + gap)
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
    /// map takes the difference, then a main pane wider than its default.
    /// With `keepsOnScreen` false (a pane opening or closing) the window
    /// never moves: it grows past the screen's edge instead (owner,
    /// 2026-10-30).
    public static func windowFrame(
        _ frame: CGRect, width: CGFloat, visible: CGRect, keepsOnScreen: Bool = true
    ) -> CGRect {
        let width = min(width, visible.width)
        var x = frame.minX
        if keepsOnScreen {
            if x + width > visible.maxX { x = visible.maxX - width }
            x = max(visible.minX, x)
        }
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
        /// The main pane's width when the move began, which it keeps
        /// throughout.
        public let mainWidth: CGFloat

        public init(from: Visibility, to: Visibility, mapWidth: CGFloat?,
                    mainWidth: CGFloat = GlassPaneLayout.mainWidth) {
            self.from = from
            self.to = to
            self.mapWidth = mapWidth
            self.mainWidth = mainWidth
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
    /// The map's width as the person last left it, kept across launches by
    /// the caller; nil to open at the default width every time.
    private let mapWidth: Binding<Double>?
    /// The main pane's width beside the map as the person last dragged the
    /// divider to (owner, 2026-10-20), kept across launches by the caller.
    private let mainWidth: Binding<Double>?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var host = GlassWindowHost()
    /// The main pane's width while the divider is dragged, and its width
    /// when the drag began; nil otherwise.
    @State private var dragMain: CGFloat?
    @State private var dragStart: CGFloat?
    /// Whether the pointer is over the divider, for its grip.
    @State private var overDivider = false
    /// The composition the panes are laid out for while no move is under
    /// way. It trails the preferences by one move.
    @State private var presented: GlassPaneLayout.Visibility
    /// The move under way, if any.
    @State private var transition: GlassPaneLayout.Transition?

    /// `showsMap` and `showsInspector` are the person's preferences.
    /// `onFirstLayout` gets the width the window has to open into once, when
    /// it is first in a window (its screen's visible width): the moment to
    /// seed preferences nobody has chosen yet.
    ///
    /// `mapWidth` is where the map's width is kept (owner, 2026-10-19): the
    /// window opens with the map at it, whatever size the window was put
    /// back at, and a map that reopens comes back at it; each time the
    /// person resizes the window with the map shown, it is written.
    public init(
        showsMap: Bool,
        showsInspector: Bool,
        mapWidth: Binding<Double>? = nil,
        mainWidth: Binding<Double>? = nil,
        onFirstLayout: ((CGFloat) -> Void)? = nil,
        @ViewBuilder main: () -> Main,
        @ViewBuilder map: () -> Map,
        @ViewBuilder inspector: () -> Inspector
    ) {
        self.showsMap = showsMap
        self.showsInspector = showsInspector
        self.mapWidth = mapWidth
        self.mainWidth = mainWidth
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
            ... CGFloat.infinity
    }

    public var body: some View {
        GeometryReader { proxy in
            let layout = transition.map { GlassPaneLayout(windowWidth: proxy.size.width, transition: $0) }
                ?? GlassPaneLayout(
                    windowWidth: proxy.size.width, showsMap: presented.map, showsInspector: presented.inspector,
                    main: dragMain ?? host.main)
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
                    // The gap before the map is the divider while no pane
                    // opens or closes (owner, 2026-10-20).
                    Group {
                        if transition == nil {
                            divider(main: layout.main, windowWidth: proxy.size.width)
                        } else {
                            Color.clear
                        }
                    }
                    .frame(width: layout.mapGap, height: height)
                    // Above the map, so the divider's reach into it takes
                    // the pointer.
                    .zIndex(1)
                    map
                        .frame(width: width, height: height)
                        .clipped()
                        .opacity(transition?.to.map == false ? 0 : 1)
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
        .onGeometryChange(for: CGFloat.self, of: \.size.width) { width in
            host.width = width
            remember(width)
        }
        // The panes run to the window's top edge; the title bar's controls
        // sit inside the main pane rather than in a strip above it.
        .ignoresSafeArea(.container, edges: .top)
        .frame(
            minWidth: widthLimits.lowerBound, maxWidth: widthLimits.upperBound,
            minHeight: GlassTokens.Size.windowMinHeight, maxHeight: .infinity, alignment: .topLeading)
        .background(GlassWindowProbe { window in
            guard host.window !== window else { return }
            host.window = window
            // Before the seeding below can move a pane: this runs first.
            GlassWindowFrame.onNextTurn { openAtKeptWidth(window) }
            onFirstLayout?(window.screen?.visibleFrame.width ?? window.frame.width)
        }.frame(width: 0, height: 0))
        // On the next turn: `.onChange` runs inside SwiftUI's update, and a
        // window frame set there re-enters the window's layout pass.
        .onChange(of: wanted) { _, new in
            GlassWindowFrame.onNextTurn { move(to: new) }
        }
    }

    /// The map's kept width, or its default.
    private var keptMapWidth: CGFloat {
        mapWidth.map { CGFloat($0.wrappedValue) } ?? GlassTokens.Size.mapWidth
    }

    /// Sizes a window that has just opened to its panes, the map at its
    /// kept width (owner, 2026-10-19): the system puts a window back at the
    /// size it last had, and the map would take all of any difference. The
    /// top and leading edges stay; a full-screen window is left alone.
    private func openAtKeptWidth(_ window: NSWindow) {
        defer { host.opened = true }
        if let mainWidth { host.main = max(GlassPaneLayout.mainWidth, CGFloat(mainWidth.wrappedValue)) }
        guard mapWidth != nil, !window.styleMask.contains(.fullScreen) else { return }
        let width = GlassPaneLayout.windowWidth(
            showsMap: presented.map, showsInspector: presented.inspector, mapWidth: keptMapWidth,
            main: presented.map ? host.main : GlassPaneLayout.mainWidth)
        let visible = window.screen?.visibleFrame ?? CGRect(x: -1e6, y: -1e6, width: 2e6, height: 2e6)
        let frame = GlassPaneLayout.windowFrame(window.frame, width: width, visible: visible)
        if frame != window.frame { window.setFrame(frame, display: true) }
    }

    /// The divider between the main pane and the map (owner, 2026-10-20):
    /// the gap itself, with the left-right resize cursor over it and a
    /// faint grip while it is pointed at or dragged. Dragging moves room
    /// between the two panes (`GlassPaneLayout.draggedMain`); the window
    /// keeps its width. Measured in the window's coordinates, so the
    /// divider moving under the pointer does not feed back into the drag.
    ///
    /// The drag is AppKit's, not a SwiftUI gesture: the window is movable
    /// by its background (`GlassBackdrop`), so a drag in the gap moved the
    /// window and never resized the panes. The handle under the gap
    /// refuses to move the window, and takes the pointer
    /// `GlassPaneLayout.dividerReach` into the map, so the gap is easier to
    /// catch and looks no wider; it leaves the main pane's scroller its
    /// edge. A click that never drags changes nothing.
    private func divider(main: CGFloat, windowWidth: CGFloat) -> some View {
        Capsule()
            .fill(GlassColor.textTertiary)
            .frame(width: 3, height: 36)
            .opacity(overDivider || dragMain != nil ? 0.6 : 0)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .allowsHitTesting(false)
            .overlay {
                GlassDividerHandle(
                    onHover: { inside in overDivider = inside },
                    onDrag: { translation in
                        let start = dragStart ?? main
                        dragStart = start
                        dragMain = GlassPaneLayout.draggedMain(
                            from: start, by: translation, windowWidth: windowWidth,
                            showsInspector: presented.inspector)
                    },
                    onEnd: { translation in
                        // Only a drag that moved ends one: a bare click
                        // never writes the width it happens to see.
                        guard let start = dragStart else { return }
                        let kept = GlassPaneLayout.draggedMain(
                            from: start, by: translation, windowWidth: windowWidth,
                            showsInspector: presented.inspector)
                        host.main = kept
                        dragStart = nil
                        dragMain = nil
                        keep(main: kept, windowWidth: windowWidth)
                    })
                    .padding(.trailing, -GlassPaneLayout.dividerReach)
            }
            // A divider that goes while pointed at (the map hidden) drops
            // its grip.
            .onDisappear { overDivider = false }
            .accessibilityHidden(true)
    }

    /// Keeps the main pane's width the divider was left at, and the map's
    /// width beside it, for the next launch.
    private func keep(main: CGFloat, windowWidth: CGFloat) {
        mainWidth?.wrappedValue = Double(main)
        let layout = GlassPaneLayout(
            windowWidth: windowWidth, showsMap: true, showsInspector: presented.inspector, main: main)
        if let mapWidth, let map = layout.map, map >= GlassTokens.Size.mapMinWidth {
            mapWidth.wrappedValue = Double(map)
        }
    }

    /// Keeps the map's width as the person resizes the window with it
    /// shown: never while a pane opens or closes, before the window has
    /// been sized at launch, or in full screen.
    private func remember(_ width: CGFloat) {
        guard let mapWidth, host.opened, transition == nil, presented.map,
              let window = host.window, !window.styleMask.contains(.fullScreen) else { return }
        let layout = GlassPaneLayout(
            windowWidth: width, showsMap: true, showsInspector: presented.inspector, main: host.main)
        guard let map = layout.map, map >= GlassTokens.Size.mapMinWidth,
              abs(Double(map) - mapWidth.wrappedValue) >= 1 else { return }
        mapWidth.wrappedValue = Double(map)
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
        let before = GlassPaneLayout(
            windowWidth: current, showsMap: old.map, showsInspector: old.inspector, main: host.main)
        // The main pane keeps the width it has, whatever opens or closes.
        host.main = before.main
        let width = GlassPaneLayout.windowWidth(
            current: current, from: old, to: new, main: host.main, mapWidth: keptMapWidth)
        let visible = window.screen?.visibleFrame ?? CGRect(x: -1e6, y: -1e6, width: 2e6, height: 2e6)
        let frame = GlassPaneLayout.windowFrame(window.frame, width: width, visible: visible, keepsOnScreen: false)
        // A window held to the screen squeezes the main pane to make room
        // for the map; the move animates only a main pane that keeps its
        // width, so a squeezed one changes at once.
        let after = GlassPaneLayout(
            windowWidth: frame.width, showsMap: new.map, showsInspector: new.inspector, main: host.main)
        if reduceMotion || frame == window.frame || after.main != before.main {
            transition = nil
            presented = new
            if frame != window.frame { window.setFrame(frame, display: true) }
            return
        }
        presented = old
        withAnimation(GlassMotion.curve(GlassTokens.Motion.standard)) {
            transition = GlassPaneLayout.Transition(
                from: old, to: new, mapWidth: before.map, mainWidth: before.main)
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
    /// The main pane's width when a pane last opened or closed: the width
    /// it keeps beside the map (`GlassPaneLayout`'s `main`).
    var main = GlassPaneLayout.mainWidth
    /// Counts moves, so a finished animation can tell it was overtaken.
    var move = 0
    /// Whether the window has been sized to its panes since it opened;
    /// until then a width it reports is the one it was put back at.
    var opened = false
}

/// The divider's handle: the AppKit view under the gap beside the map that
/// takes the drag (`GlassDividerHandleView`).
private struct GlassDividerHandle: NSViewRepresentable {
    let onHover: @MainActor (Bool) -> Void
    let onDrag: @MainActor (CGFloat) -> Void
    let onEnd: @MainActor (CGFloat) -> Void

    func makeNSView(context: Context) -> GlassDividerHandleView {
        let view = GlassDividerHandleView()
        updateNSView(view, context: context)
        return view
    }

    func updateNSView(_ view: GlassDividerHandleView, context: Context) {
        view.onHover = onHover
        view.onDrag = onDrag
        view.onEnd = onEnd
    }
}

/// Reports a horizontal drag across it, measured in the window's
/// coordinates from where it began, and never lets the drag move the
/// window: the window is movable by its background, and without this a
/// drag in the gap moved it. Shows the left-right resize cursor while
/// pointed at or dragged.
final class GlassDividerHandleView: NSView {
    var onHover: (@MainActor (Bool) -> Void)?
    var onDrag: (@MainActor (CGFloat) -> Void)?
    var onEnd: (@MainActor (CGFloat) -> Void)?
    /// Where the drag began, in the window's coordinates; nil between drags.
    private var startX: CGFloat?
    /// The pointer moved since the mouse went down: only then does the
    /// mouse going up end a drag. A bare click, the one that activates the
    /// window included, reports nothing.
    private var moved = false
    private var tracking: NSTrackingArea?

    override var mouseDownCanMoveWindow: Bool { false }

    /// A click in a window that is not key starts the drag too.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseEnteredAndExited, .cursorUpdate, .activeInKeyWindow, .inVisibleRect],
            owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func cursorUpdate(with event: NSEvent) {
        NSCursor.resizeLeftRight.set()
    }

    override func mouseEntered(with event: NSEvent) { onHover?(true) }

    override func mouseExited(with event: NSEvent) { onHover?(false) }

    override func mouseDown(with event: NSEvent) {
        startX = event.locationInWindow.x
        moved = false
        NSCursor.resizeLeftRight.set()
    }

    override func mouseDragged(with event: NSEvent) {
        guard let startX else { return }
        // The divider stops at its limits while the pointer goes on; the
        // cursor stays the resize one until the drag ends.
        NSCursor.resizeLeftRight.set()
        moved = true
        onDrag?(event.locationInWindow.x - startX)
    }

    override func mouseUp(with event: NSEvent) {
        guard let startX else { return }
        self.startX = nil
        guard moved else { return }
        moved = false
        onEnd?(event.locationInWindow.x - startX)
    }
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
