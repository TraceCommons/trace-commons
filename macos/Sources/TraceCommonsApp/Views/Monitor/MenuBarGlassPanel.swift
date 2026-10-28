import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The menu-bar popover (R13 of #1173), from the "Menu bar item and
/// popover" handoff, as the owner reshaped it on 2026-10-19 and
/// 2026-10-22: a top row with the NEAR AI wordmark on the left and
/// Watching, Settings and Quit as icon buttons on the right; Private AI,
/// with the NEAR AI tools, one button each, for a quick Private AI session;
/// the suggestion and the estimate; the
/// shared/kept legend and the app's bar graph; and the way into the app.
/// It is the app's only menu-bar item, in every build.
///
/// The handoff's rules hold here:
/// - Nothing is sent from the popover. Its one write is pausing and
///   resuming. The contribution mode and Private AI pills are gone (owner,
///   2026-10-19), and with them the override and turning Private AI off.
/// - A tool button opens the window's Private AI tab at that tool's launch
///   dialog: a dialog inside `MenuBarExtra` is unreliable.
/// - No projected credit, and the badge is decisions owed.
struct MenuBarGlassPanel: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow
    let navigation: MainWindowNavigation
    let store: MenuPanelStore
    /// Whether the panel draws its own glass. Inside `MenuBarExtra` the
    /// system window already carries its material, and a second layer of
    /// glass on it is glass on glass (R14); the preview window has none.
    var ownsSurface = false
    /// The graph's bar under the pointer, whose day the legend counts.
    @State private var hovered: String?

    /// A quarter narrower than the handoff's 380 (owner, 2026-10-21).
    static let width: CGFloat = 285

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            topRow
            privateAI
            nudgeRow
            settlementRow
            graph
            glance
            menuItems
        }
        .padding(GlassTokens.Space.s5)
        .frame(width: Self.width)
        .modifier(PanelSurface(owns: ownsSurface))
        // The glance is re-read while open: its ledger can go stale silently.
        .task { await store.followGlance() }
        // A fresh read on opening; the label follows the event stream. The
        // app's own reads (the tools and Cmd-Shift-M read
        // `model.daemonSettings`) refresh on opening too, as the legacy menu
        // did.
        .task { await store.load() }
        .onAppear { model.refreshAll() }
    }

    // MARK: Top row

    /// The NEAR AI wordmark in the top left corner (owner, 2026-10-22),
    /// its "near" starting where the rows' words do, then Watching,
    /// Settings and Quit as icon buttons in the top right corner. Watching
    /// is a dropdown menu of its pause choices, or Resume.
    private var topRow: some View {
        HStack(spacing: GlassTokens.Space.s2) {
            GlassNearWordmark()
                .frame(height: Self.wordmarkHeight)
                .padding(.leading, GlassTokens.Space.s4)
            Spacer(minLength: GlassTokens.Space.s4)
            WatchingMenuButton()
            GlassRoundButton(MonitorWords.table?.settingsTitle ?? MenuWords.settings, icon: .glyph(.gear)) {
                NSApp.activate(ignoringOtherApps: true)
                navigation.requestSettings()
                openWindow(id: WindowID.settings)
            }
            GlassRoundButton(MenuWords.quit, systemImage: "power") { NSApp.terminate(nil) }
        }
    }

    /// The wordmark's height: about 85pt wide, past the brand kit's 72px
    /// floor for the wordmark, and shorter than the icon buttons beside it.
    static let wordmarkHeight: CGFloat = 16

    // MARK: Private AI

    /// Private AI, just under the top row (owner, 2026-10-22): its name
    /// with On or Off beside it, over the NEAR AI tools, one button each,
    /// for a quick Private AI session (owner, 2026-10-19). Drawn only while
    /// signed in to NEAR AI, and only with the core's words for its name.
    /// The buttons are drawn only while a tool can be started
    /// (`ManagedSessionsSection.toolArea`); a button opens the Private AI
    /// tab at that tool's launch dialog.
    @ViewBuilder
    private var privateAI: some View {
        if let snapshot = model.managedSnapshot, snapshot.nearAiSignedIn,
           let name = model.privateInferenceCopy?.destination
        {
            GlassEyebrowCard(name, accessory: {
                PrivateAIStatusLabel { open(.settings(.privateAI)) }
            }, content: {
                if MenuPanelStatus.showsToolButtons(snapshot) {
                    HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                        ForEach(snapshot.signInStartTools, id: \.self) { tool in
                            ManagedToolButton(tool: tool) {
                                model.pendingManagedLaunch = tool
                                open(.inference)
                            }
                        }
                    }
                    .disabled(model.managedBusy)
                }
            })
        }
    }

    // MARK: Legend and graph

    /// The shared and kept legend over the app's bar graph (owner,
    /// 2026-10-22: the popover's graph matches the app's), as the edge
    /// rail's Traces peek draws them: the Traces graph's days, counted as
    /// it counts them, in its compact form for the narrower panel. The
    /// legend counts the graph's days, or the day under the pointer. Each
    /// count comes from its own read (`store.sharedCount`,
    /// `store.keptCount`); one that is not current is the core's unknown
    /// word, never the last count and never a bare dash.
    private var graph: some View {
        let buckets = store.buckets
        let shown = buckets.first { $0.start.formatted(.iso8601) == hovered }
        let shared = store.sharedCount.map { shown?.shared ?? $0 }
        let kept = store.keptCount.map { shown?.kept ?? $0 }
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MenuWords.shared, value: MenuPanelData.countWord(shared, unknown: MonitorWords.unknown),
                                status: .shared)
                GlassLegendCell(MenuWords.kept, value: MenuPanelData.countWord(kept, unknown: MonitorWords.unknown),
                                status: .kept)
            }
            GlassBarGraph(
                MenuPanelData.bars(buckets, words: MonitorShellWords.graphWords),
                scaleFloor: 5, compact: true, hovered: $hovered)
                .frame(height: EdgeRailGraph.height)
        }
    }

    // MARK: Glance

    /// Today's routed calls per tool, only for fresh data with rows; every
    /// other state draws nothing, never a zero.
    @ViewBuilder
    private var glance: some View {
        if let glance = MenuPanelData.glanceToDraw(store.glance, stale: store.stale) {
            InsightsGlanceCard(glance: glance) { open(.inference) }
        }
    }

    // MARK: The nudge row

    /// The lead suggestion's panel row, in the daemon's words: tapping it
    /// opens its place (Traces at the idle sessions, Traces, or History).
    @ViewBuilder
    private var nudgeRow: some View {
        if let row = store.nudgeRow {
            Button {
                Task {
                    let destination = await store.open(row)
                    open(destination)
                }
            } label: {
                HStack {
                    Text(row.text)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").glassGlyph(10, weight: .semibold)
                }
            }
            .buttonStyle(GlassMenuRowStyle())
            .padding(.vertical, -GlassTokens.Space.s2)
        }
    }

    // MARK: The settlement estimate

    /// The machine-wide estimate, in the core's words: the finished line, an
    /// "Estimate" caption, and a button that opens Traces. Nothing here is
    /// sent anywhere; the line is drawn only while the panel is current and
    /// the daemon reported one.
    @ViewBuilder
    private var settlementRow: some View {
        if let line = store.settlementLine {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(line.label)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textTertiary)
                    .padding(.horizontal, GlassTokens.Space.s3)
                Button {
                    open(.traces(entryId: nil))
                } label: {
                    HStack {
                        Text(line.text)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right").glassGlyph(10, weight: .semibold)
                    }
                }
                .buttonStyle(GlassMenuRowStyle())
                .accessibilityLabel(line.text)
                .accessibilityHint(line.action)
                .padding(.vertical, -GlassTokens.Space.s2)
            }
        }
    }

    // MARK: Menu items

    /// The handoff spaces these rows like the popover's own children: 8pt
    /// between each row's selection and the next. The rows stack with no
    /// gap; each carries its own clear 4pt above and below
    /// (`GlassMenuRowStyle`), so its hit rect is about 34pt (a 26pt
    /// selection plus 8) and a click between two selections still lands on
    /// a row. The block gives back those 4pt at its top and bottom edges,
    /// so its gap to the content above and its bottom inset stay 8pt.
    private var menuItems: some View {
        VStack(alignment: .leading, spacing: 0) {
            hairline
            // The main window, wherever it was left (owner feedback,
            // 2026-10-11): closing it left no way back but the Dock.
            // Settings and Quit are icon buttons in the top row, and
            // Manage rules is gone (owner, 2026-10-19).
            Button(MenuWords.openApp) { open(nil) }
        }
        .buttonStyle(GlassMenuRowStyle())
        .padding(.vertical, -GlassTokens.Space.s2)
    }

    private var hairline: some View {
        Rectangle().fill(GlassColor.ink(0.12)).frame(height: 1)
            .padding(.vertical, GlassTokens.Space.s2)
            .accessibilityHidden(true)
    }

    /// The Monitor (or first run) at a destination; the handler raises it.
    private func open(_ destination: MonitorDestination?) {
        OpenMonitor.request(destination)
    }
}

/// Watching as a round dropdown (owner, 2026-10-19: a traditional menu,
/// not a list sliding open in the panel): under its name, the pause
/// choices (`MenuBarWords`) while watching, or Resume while paused. The
/// menu-bar popover's top row has it, and the edge rail's Traces peek
/// (owner, 2026-10-22).
struct WatchingMenuButton: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassRoundMenu(MonitorWords.watching, systemImage: paused ? "eye.slash" : "eye") { watchItems }
            .accessibilityValue(watchValue)
            .disabled(watchState == .coreDown || watchState == .loading)
    }

    private var paused: Bool { watchState == .paused }

    /// Watching's state (the stack-wide ScreenState rule): a core that is
    /// not running is core down, and never reads as On.
    private var watchState: ScreenState {
        switch model.startup {
        case .running:
            return ScreenState.resolve(failure: nil, loaded: true, paused: model.status.paused, known: true)
        case .starting:
            return .loading
        default:
            return .coreDown
        }
    }

    private var watchValue: String {
        switch watchState {
        case .ready: MenuWords.on
        case .paused: MonitorWords.paused
        case .coreDown, .loading, .unknown: "—"
        }
    }

    @ViewBuilder
    private var watchItems: some View {
        Section(MonitorWords.watching) {
            if paused {
                Button(MenuBarWords.resume) { model.resume() }
            } else {
                ForEach(Array([MenuBarWords.pauseHour, MenuBarWords.pauseMorning,
                               MenuBarWords.pauseIndefinite].enumerated()), id: \.offset) { index, label in
                    Button(label) { model.pause(until: MenuBarWords.pauseUntil(index)) }
                }
            }
        }
    }
}

/// Private AI's one word, with its dot (owner, 2026-10-22, 2026-10-28): the
/// menu-bar popover's Private AI section has it, and the edge rail's
/// Private AI peek beside its title.
struct PrivateAIStatusLabel: View {
    @EnvironmentObject private var model: AppModel
    /// Where a click goes; nil draws the label alone.
    var action: (() -> Void)?

    var body: some View {
        let state = MenuPanelStatus.privateAI(model)
        let label = GlassStatusLabel(MenuPanelStatus.privateAIValue(state, model: model), status: MenuPanelStatus.privateAIDot(state))
            .lineLimit(1)
        let sentence = MenuPanelStatus.privateAISentence(model) ?? ""
        if let action {
            Button(action: action) { label }
                .buttonStyle(.plain)
                .help(sentence)
                .accessibilityHint(sentence)
        } else {
            label.help(sentence).accessibilityHint(sentence)
        }
    }
}

/// The popover's surface. Inside `MenuBarExtra` it is the app's own pane,
/// the same glass, veil and gradient as the main window's and the edge
/// rail's panes (owner, 2026-10-12: one glass spec), with the system
/// window's own material hidden under it (`MenuBarMaterialMask`), so there
/// is never glass on glass; and the window is sized and its top edge
/// pinned by `MenuBarTopEdgePin`, from the content's size measured at its
/// ideal height (so the window's own size never feeds back into it).
/// The debug preview window, which has no material, draws the popover tier.
private struct PanelSurface: ViewModifier {
    let owns: Bool
    /// The content's size at its ideal height, for the window's frame.
    @State private var contentSize: CGSize = .zero

    func body(content: Content) -> some View {
        if owns {
            content.glassSurface(.popover, radius: GlassTokens.Radius.menuPanel, floating: true)
        } else {
            content
                // Its ideal height whatever the window's height is: the
                // window is sized from this, never the other way round.
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGSize.self, of: \.size) { contentSize = $0 }
                .background {
                    // Full bleed, rounded as the window is.
                    Color.clear
                        .glassTier(.pane, radius: GlassTokens.Radius.menuPanel)
                        .ignoresSafeArea()
                }
                .background(MenuBarMaterialMask().frame(width: 0, height: 0))
                .background(MenuBarTopEdgePin(content: contentSize).frame(width: 0, height: 0))
                // Names this window in the log when it resizes.
                .windowBreadcrumb(.menuBarPanel)
        }
    }
}

/// Takes the `MenuBarExtra` window's own background away, so the popover's
/// pane is the only glass: the window draws no background (on macOS 26 its
/// material is the window's, drawn behind the content, with no view of its
/// own), and any visual-effect view behind the panel's content has its
/// material masked out (`maskImage`, which masks the material and never the
/// views inside it). It runs again a runloop later: the system finishes the
/// window after the content lands in it, and a change made before then was
/// undone (seen with `TRACE_COMMONS_GLASS_PROBE`, 2026-10-12).
struct MenuBarMaterialMask: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Mask() }

    func updateNSView(_ view: NSView, context: Context) {}

    /// An image with nothing in it: as a mask it shows none of a material.
    static let empty: NSImage = {
        let image = NSImage(size: NSSize(width: 1, height: 1))
        image.capInsets = NSEdgeInsetsZero
        image.resizingMode = .stretch
        return image
    }()

    /// The visual-effect views under `root`, depth first.
    static func effects(in root: NSView) -> [NSVisualEffectView] {
        var found: [NSVisualEffectView] = []
        if let effect = root as? NSVisualEffectView { found.append(effect) }
        for child in root.subviews { found += effects(in: child) }
        return found
    }

    private final class Mask: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            apply()
            // The system finishes the window after the content lands in it.
            DispatchQueue.main.async { [weak self] in self?.apply() }
            #if DEBUG
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_GLASS_PROBE"] != nil {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.apply() }
            }
            #endif
        }

        private func apply() {
            guard let window, let frame = window.contentView?.superview ?? window.contentView else { return }
            // The pane's own glass is an NSGlassEffectView or, before
            // macOS 26, a vibrancy view inside the panel's content; only the
            // window's, outside this view's hosting tree, are masked.
            let hosting = enclosingHosting()
            var masked: [String] = []
            for effect in MenuBarMaterialMask.effects(in: frame) where hosting.map({ !effect.isDescendant(of: $0) }) ?? true {
                effect.maskImage = MenuBarMaterialMask.empty
                masked.append(String(describing: type(of: effect)))
            }
            #if DEBUG
            // A development probe: what was masked, for checking the popover
            // has one glass layer. Never on by default.
            if let path = ProcessInfo.processInfo.environment["TRACE_COMMONS_GLASS_PROBE"] {
                func tree(_ view: NSView, _ depth: Int) -> String {
                    String(repeating: "  ", count: depth) + "\(type(of: view)) \(view.frame.size)\n"
                        + view.subviews.map { tree($0, depth + 1) }.joined()
                }
                let root = window.contentView?.superview ?? window.contentView
                let line = "\(type(of: window)) masked=\(masked) hosting=\(hosting.map { String(describing: type(of: $0)) } ?? "none")\n"
                    + (root.map { tree($0, 0) } ?? "")
                if let handle = FileHandle(forWritingAtPath: path) {
                    handle.seekToEndOfFile()
                    handle.write(Data(line.utf8))
                    try? handle.close()
                } else {
                    try? line.write(toFile: path, atomically: true, encoding: .utf8)
                }
            }
            #else
            _ = masked
            #endif
            window.isOpaque = false
            window.backgroundColor = .clear
        }

        /// The SwiftUI hosting view this view sits in.
        private func enclosingHosting() -> NSView? {
            var view: NSView? = superview
            var last: NSView?
            while let current = view {
                if String(describing: type(of: current)).contains("NSHostingView") { last = current }
                view = current.superview
            }
            return last
        }
    }
}

/// Sizes the `MenuBarExtra` window and holds its top edge under the menu
/// bar: the one writer of the panel's size.
///
/// SwiftUI used to size this window to its content itself, from inside
/// the window's layout pass (`NSHostingView.updateAnimatedWindowSize`), and
/// AppKit aborted the app when that kept asking for another pass: the
/// crash reported from 0.16 to 0.20.4, every report through a borderless
/// window's frame (`NSNextStepFrame`). The edge rail had the same signature
/// until #1402 took its sizing away from its hosting view; this does the
/// same for the panel. The hosting view takes no sizing options or
/// safe-area insets, and the window is set here, on the next turn, to the
/// content's measured size (`PanelSurface` measures it with the content at
/// its ideal height, so it never depends on the window), with its top at
/// the menu bar's bottom edge. The sizing is taken over on the next turn
/// too, after SwiftUI has placed and first sized the window. Empty sizing
/// options do not stop the hosting view's animated resize of its window;
/// the window drops that (`MenuBarPanelFrame`).
///
/// The top: an `NSWindow` that is resized keeps its bottom edge, so a panel
/// that grew used to rise into the menu bar and one that shrank dropped
/// away from it (owner, 2026-10-11). The top is the menu bar's bottom edge
/// on the window's screen, whenever the window opens, moves or resizes.
struct MenuBarTopEdgePin: NSViewRepresentable {
    /// The panel content's size, as last measured.
    let content: CGSize

    func makeNSView(context: Context) -> NSView {
        let pin = Pin()
        pin.content = content
        return pin
    }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? Pin)?.content = content
    }

    /// The panel's frame for content of size `content`: that size, the
    /// leading edge it has, the top at `top` (the top it has while none is
    /// known), extending downward. Nil when the window is already there, or
    /// nothing has been measured yet. Computed from the content and the
    /// menu bar alone, never from the window's own size, so a resize this
    /// causes settles in one step. Pure, so tested.
    static func frame(current: CGRect, content: CGSize, top: CGFloat?) -> CGRect? {
        guard content.width > 0, content.height > 0 else { return nil }
        let maxY = top ?? current.maxY
        let target = CGRect(
            x: current.minX, y: maxY - content.height, width: content.width, height: content.height)
        if abs(target.width - current.width) < 0.5, abs(target.height - current.height) < 0.5,
           abs(target.maxY - current.maxY) < 0.5 { return nil }
        return target
    }

    /// The menu bar's bottom edge on a screen: the top of its visible
    /// frame, or, when the menu bar hides itself and the visible frame
    /// reaches the top, the screen's top less the menu bar's thickness.
    /// Pure, so tested.
    static func menuBarBottom(screen: CGRect, visible: CGRect, thickness: CGFloat) -> CGFloat {
        visible.maxY < screen.maxY - 0.5 ? visible.maxY : screen.maxY - thickness
    }

    /// Takes the window's sizing away from SwiftUI: the hosting view at
    /// the root of `view`'s window no longer sizes the window to its
    /// content, and takes no safe-area insets (#1402's two settings).
    static func takeOverSizing(from view: NSView) {
        var current: NSView? = view.superview
        var outermost: NSView?
        while let candidate = current {
            if candidate is HostingWindowSizing { outermost = candidate }
            current = candidate.superview
        }
        guard let sizing = outermost as? HostingWindowSizing else { return }
        if !sizing.sizingOptions.isEmpty { sizing.sizingOptions = [] }
        if !sizing.safeAreaRegions.isEmpty { sizing.safeAreaRegions = [] }
    }

    private final class Pin: NSView {
        private var observers: WindowNotificationObservers?
        /// A placement already asked for on the next turn.
        private var scheduled = false

        var content: CGSize = .zero {
            didSet {
                if abs(content.width - oldValue.width) >= 0.5 || abs(content.height - oldValue.height) >= 0.5 {
                    schedule()
                }
            }
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observers = nil
            guard let window else { return }
            schedule()
            let center = NotificationCenter.default
            let tokens = [NSWindow.didBecomeKeyNotification, NSWindow.didResizeNotification,
                          NSWindow.didMoveNotification, NSWindow.didChangeOcclusionStateNotification].map { name in
                // On the next turn: a notification can arrive inside a
                // resize of the window, and setting its frame there
                // re-enters that layout pass.
                center.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.schedule() }
                }
            }
            observers = WindowNotificationObservers(center: center, tokens: tokens)
        }

        /// Places the window on the next turn of the run loop, once however
        /// many changes ask for it in this one, taking its sizing over from
        /// SwiftUI first: by then SwiftUI has placed and first sized the
        /// window, which it does after the content lands in it. From then
        /// on the window keeps only the sizes placed here.
        private func schedule() {
            guard !scheduled else { return }
            scheduled = true
            GlassWindowFrame.onNextTurn { [weak self] in
                guard let self else { return }
                self.scheduled = false
                guard let window = self.window else { return }
                MenuBarTopEdgePin.takeOverSizing(from: self)
                MenuBarPanelFrame.hold(window)
                self.place(window)
            }
        }

        private func place(_ window: NSWindow) {
            guard let screen = window.screen ?? NSScreen.main else { return }
            let top = MenuBarTopEdgePin.menuBarBottom(
                screen: screen.frame, visible: screen.visibleFrame, thickness: NSStatusBar.system.thickness)
            if let frame = MenuBarTopEdgePin.frame(current: window.frame, content: content, top: top) {
                MenuBarPanelFrame.place(frame, in: window)
            }
        }

    }
}

/// The two settings #1402 turned off on the edge rail's hosting view, on
/// a hosting view of any content type: the `MenuBarExtra`'s hosting view
/// is generic over a type SwiftUI does not expose, and this conformance
/// reaches its settings through a plain `NSView`.
@MainActor
protocol HostingWindowSizing: AnyObject {
    var sizingOptions: NSHostingSizingOptions { get set }
    var safeAreaRegions: SafeAreaRegions { get set }
}

extension NSHostingView: HostingWindowSizing {}


/// The menu-bar item in the glass system: the NEAR AI mark and the last 24
/// hours as paired up-and-down bars (`GlassMenuBarStrip`), with the
/// decisions-owed badge. Trace bars grey while paused.
struct MenuBarStripLabel: View {
    @ObservedObject var model: AppModel
    let store: MenuPanelStore
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        let available = model.startup == .running && !store.stale
        // The strip draws the columns, so it is unavailable when either read
        // behind them did not answer, never the last bars as current.
        let condition = MenuPanelStatus.condition(
            decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
            paused: model.status.paused, available: model.startup == .running,
            stale: store.stale || !store.columnsCurrent)
        let badge = MenuPanelStatus.badge(model.decisionsOwed)
        // The mark's words, only while the strip draws the mark: the badge
        // and condition come from a different read than the nudge, and a
        // ring or halo the strip hid is never spoken or shown as a tooltip.
        let words = MenuPanelStatus.markWords(
            store.status?.nudge, available: available, badge: badge, condition: condition)
        Image(nsImage: rendered(
            condition: condition, badge: badge,
            mark: MenuPanelStatus.mark(store.status?.nudge, available: available)))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(MenuPanelStatus.markAccessibility(
                base: MenuBarStatus.accessibilityLabel(
                    decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
                    paused: model.status.paused, available: model.startup == .running),
                words: words))
            // The drawn mark's tooltip clause, in the core's words.
            .help(MenuPanelStatus.markTooltip(words))
            // The label is always alive, so it owns the subscription: the
            // app's live client, re-attached whenever the daemon restarts.
            .task(id: model.liveData.map(ObjectIdentifier.init)) {
                store.attach(model.daemonData, configDirectory: model.configDirectory)
                await store.run()
            }
    }

    /// The strip as a bitmap. A `MenuBarExtra` label keeps only an `Image`
    /// and a `Text` out of whatever view it is given, so the strip drawn as
    /// a view lost its bars and kept the badge as a bare number. Drawn as a
    /// non-template image it reaches the menu bar in its own colours, once
    /// for a dark bar and once for a light one (`MenuBarItemImage`).
    /// Exactly the item's height, so the menu bar centres the mark and the
    /// bars in it, and padded past the badge's halo so it is not clipped.
    /// The condition, badge and nudge mark are the ones `body` speaks, so
    /// the drawn mark and its words never disagree.
    @MainActor
    private func rendered(
        condition: GlassMenuBarStrip.Condition, badge: Int?, mark: GlassMenuBarStrip.Mark
    ) -> NSImage {
        let traces = store.stripTraces
        let privateAI = store.stripPrivateAI
        func render(_ scheme: ColorScheme) -> NSImage {
            let renderer = ImageRenderer(content: GlassMenuBarStrip(
                traces: traces,
                privateAI: privateAI,
                condition: condition,
                badge: badge,
                mark: mark)
                .padding(.trailing, GlassMenuBarStrip.haloReach)
                .environment(\.colorScheme, scheme))
            renderer.scale = displayScale
            return renderer.nsImage ?? NSImage()
        }
        return MenuBarItemImage.adaptive(dark: render(.dark), light: render(.light))
    }
}

/// The menu-bar item's image, drawn in the menu bar's own appearance.
enum MenuBarItemImage {
    /// One image that draws `dark` on a dark menu bar and `light` on a light
    /// one. Its drawing handler runs each time the item draws, in the
    /// appearance of the bar it sits on, which is not always the app's: the
    /// bar follows the wallpaper behind it, so a Mac in Light Mode can show
    /// a dark bar. Not a template, so the bars keep their colours.
    static func adaptive(dark: NSImage, light: NSImage) -> NSImage {
        let image = NSImage(size: dark.size, flipped: false) { rect in
            (isDark(NSAppearance.currentDrawing()) ? dark : light).draw(in: rect)
            return true
        }
        image.isTemplate = false
        return image
    }

    /// Whether an appearance is a dark one, its vibrant and high-contrast
    /// forms included.
    static func isDark(_ appearance: NSAppearance) -> Bool {
        switch appearance.bestMatch(from: [.aqua, .darkAqua, .vibrantLight, .vibrantDark]) {
        case .darkAqua?, .vibrantDark?: true
        default: false
        }
    }
}

/// The badge for the menu-bar item. Pure, so tested.
enum MenuPanelStatus {
    /// Decisions owed when there are any. Zero and unknown draw none.
    static func badge(_ decisionsOwed: Int?) -> Int? {
        guard let decisionsOwed, decisionsOwed > 0 else { return nil }
        return decisionsOwed
    }

    /// What the strip says, in `MenuBarStatus.state`'s spirit: a daemon
    /// that is not running, or data that could not be read, is
    /// unavailable; trouble or an unknown count needs attention; then
    /// paused; then live. Never live unless everything is.
    static func condition(
        decisionsOwed: Int?, unhealthy: Bool, paused: Bool, available: Bool, stale: Bool
    ) -> GlassMenuBarStrip.Condition {
        guard available, !stale else { return .unavailable }
        if unhealthy || decisionsOwed == nil { return .attention }
        return paused ? .paused : .live
    }

    /// The nudge mark the strip draws; nothing while it cannot vouch for
    /// what it shows.
    static func mark(_ nudge: DaemonData.Nudge?, available: Bool) -> GlassMenuBarStrip.Mark {
        switch NudgeSurface.mark(nudge, available: available) {
        case .news: .news
        case .ready: .ready
        case .none: .none
        }
    }

    /// The core's words for the mark the strip draws, or nil while it draws
    /// none: the daemon lit nothing, the strip cannot vouch for it, or the
    /// badge or condition hides it (`GlassMenuBarStrip.shownMark`).
    static func markWords(
        _ nudge: DaemonData.Nudge?, available: Bool, badge: Int?, condition: GlassMenuBarStrip.Condition
    ) -> DaemonData.NudgeMarkText? {
        let lit = mark(nudge, available: available)
        guard GlassMenuBarStrip.shownMark(lit, badge: badge, condition: condition) != .none else { return nil }
        return NudgeSurface.markText(nudge, available: available)
    }

    /// The item's accessibility label: what it already says, then the drawn
    /// mark's sentence from the core, whole.
    static func markAccessibility(base: String, words: DaemonData.NudgeMarkText?) -> String {
        guard let sentence = words?.accessibility, !sentence.isEmpty else { return base }
        return base + " " + sentence
    }

    /// The drawn mark's tooltip clause; empty while no mark is drawn.
    static func markTooltip(_ words: DaemonData.NudgeMarkText?) -> String {
        words?.tooltip ?? ""
    }

    /// The settlement estimate line: the core's text, its caption and the
    /// action's name. The title and the accessibility name are separate keys
    /// in the core; the button label is `action`.
    struct SettlementLine: Equatable {
        let text: String
        let label: String
        let action: String
    }

    /// The core's nudge table, read once.
    static let nudgeCopy: NudgeCopy? = NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON())

    /// The line to draw, or nil: nothing while the panel cannot vouch for
    /// its data, while the daemon reported no estimate (switch off, no
    /// drawable table, nothing known: never $0), or while the core did not
    /// word the caption or the action.
    static func settlementLine(
        _ status: DaemonData.Status?, available: Bool, copy: NudgeCopy? = MenuPanelStatus.nudgeCopy
    ) -> SettlementLine? {
        guard available, let estimate = status?.settlementEstimate,
              !estimate.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let label = copy?[.estimateTotalLabel], let action = copy?[.estimateTotalAction]
        else { return nil }
        return SettlementLine(text: estimate.text, label: label, action: action)
    }

    /// The Private AI row's state.
    enum PrivateAI: Equatable {
        /// The core has not said whether the switch is on, or has no word
        /// for the listener's state.
        case unknown
        case off
        /// The switch is on: the core's one word for the listener's state
        /// and the tone of its dot. Only a clear tone is drawn green.
        case listener(word: String, tone: PrivateInferenceTone)
    }

    /// Off while the switch is off; otherwise the core's word and tone for
    /// the listener's state, so a switch left on over a dead listener never
    /// reads On and an On that is not answering is not drawn green.
    static func privateAI(on: Bool?, menu: (word: String, tone: PrivateInferenceTone)?) -> PrivateAI {
        guard let on else { return .unknown }
        guard on else { return .off }
        guard let menu else { return .unknown }
        return .listener(word: menu.word, tone: menu.tone)
    }

    /// Private AI's state: the listener's reported state, never the switch
    /// alone. A switch the core did not report is unknown, never Off.
    @MainActor
    static func privateAI(_ model: AppModel) -> PrivateAI {
        privateAI(
            on: model.daemonSettings?.privateInference,
            menu: PrivateInferenceSurface.menuState(model.privateInferenceState, calls: model.privateInferenceCalls))
    }

    /// One word, in the core's words (owner, 2026-10-28): the full state
    /// sentence truncated here and read as an error, so it stays on the
    /// Private AI page and the dot carries the difference. Unknown is a
    /// dash, never Off.
    @MainActor
    static func privateAIValue(_ state: PrivateAI, model: AppModel) -> String {
        switch state {
        case .listener(let word, _): word
        case .off: MonitorWords.off
        case .unknown: "—"
        }
    }

    /// The dot beside Private AI's word: green only for a clear tone, amber
    /// for On with something to look at, red for a listener that is not
    /// working, grey otherwise.
    static func privateAIDot(_ state: PrivateAI) -> GlassStatus {
        switch state {
        case .listener(_, let tone): privateAIDot(tone)
        case .off, .unknown: .off
        }
    }

    static func privateAIDot(_ tone: PrivateInferenceTone) -> GlassStatus {
        switch tone {
        case .clear: .on
        case .attention: .ask
        case .refused: .outside
        case .neutral, .held: .off
        }
    }

    /// The row's full sentence for a tooltip and the accessibility hint:
    /// the same sentence the Private AI page shows.
    @MainActor
    static func privateAISentence(_ model: AppModel) -> String? {
        guard let copy = model.privateInferenceCopy else { return nil }
        return PrivateInferenceSurface.stateLine(model.privateInferenceState, copy: copy, calls: model.privateInferenceCalls)
    }

    /// Whether Private AI's section draws the tool buttons: only while a
    /// tool can be started, the Private AI tab's own rule
    /// (`ManagedSessionsSection.toolArea`), and there is one to start.
    static func showsToolButtons(_ snapshot: ManagedSnapshot) -> Bool {
        ManagedSessionsSection.toolArea(snapshot) == .buttons && !snapshot.signInStartTools.isEmpty
    }
}

/// The popover's single words and short labels, beside the core's copy.
/// The popover's words, from the core's table (`MonitorWords.table`). This
/// shell holds none of its own.
enum MenuWords {
    static var on: String { MonitorWords.table?.on ?? "" }
    static var mixed: String { MonitorWords.table?.mixed ?? "" }
    static var shared: String { MonitorWords.table?.shared ?? "" }
    static var kept: String { MonitorWords.table?.kept ?? "" }
    static var recentActivity: String { MonitorWords.table?.recentActivity ?? "" }
    static var openApp: String { MonitorWords.table?.openApp ?? "" }
    static var manageRules: String { MonitorWords.table?.manageRules ?? "" }
    static var settings: String { MonitorWords.table?.settings ?? "" }
    static var quit: String { MonitorWords.table?.quit ?? "" }
}
#if DEBUG
/// The menu-bar item and the popover under it, in a window: the same views
/// on the same store, for reviewing them where the menu bar has no room.
struct MenuBarPreviewWindow: View {
    @EnvironmentObject private var model: AppModel
    let navigation: MainWindowNavigation
    let store: MenuPanelStore

    var body: some View {
        VStack(alignment: .trailing, spacing: GlassTokens.Space.s4) {
            MenuBarStripLabel(model: model, store: store)
                .padding(.horizontal, GlassTokens.Space.s4)
                .background(Capsule().fill(GlassColor.ink(0.12)))
            MenuBarGlassPanel(navigation: navigation, store: store, ownsSurface: true)
        }
        .padding(GlassTokens.Space.s10)
        .background(LinearGradient(colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color], startPoint: .top, endPoint: .bottom))
    }
}
#endif
