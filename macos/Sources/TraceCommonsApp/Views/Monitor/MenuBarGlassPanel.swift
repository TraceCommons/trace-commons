import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The menu-bar popover (R13 of #1173), from the "Menu bar item and
/// popover" handoff: three state pills, a sub-list, the shared/kept legend
/// and day graph, recent activity, and shortcuts into the app. It is the
/// app's only menu-bar item, in every build.
///
/// The handoff's rules hold here:
/// - Nothing is sent from the popover. The writes are pausing, resuming,
///   turning Private AI off (`PrivateInferenceTray`) and the contribution
///   override.
/// - Private AI "On" opens the window at its destination; a menu press
///   never turns it on.
/// - The mode pill shows the daemon's roll-up (`status.contribution_mode`:
///   one mode, or Mixed). Its choices set a global contribution override
///   (#1208), each only after the core's confirmation for it; Auto
///   contribute's carries the arming disclosure, so arming never happens
///   from a single menu press.
/// - No projected credit, and the badge is decisions owed.
struct MenuBarGlassPanel: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigation: MainWindowNavigation
    let store: MenuPanelStore
    /// Whether the panel draws its own glass. Inside `MenuBarExtra` the
    /// system window already carries its material, and a second layer of
    /// glass on it is glass on glass (R14); the preview window has none.
    var ownsSurface = false

    enum Pill: Equatable { case mode, watch, privateAI }

    @State private var expanded: Pill = .mode
    @State private var sub: Pill?

    static let width: CGFloat = 380

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            if let sub {
                subList(sub)
                    .transition(.opacity)
            } else {
                pills
                    .transition(.opacity)
            }
            nudgeRow
            legend
            graph
            glance
            recent
            menuItems
        }
        .padding(GlassTokens.Space.s5)
        .frame(width: Self.width)
        .modifier(PanelSurface(owns: ownsSurface))
        .animation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide), value: sub)
        // The glance is re-read while open: its ledger can go stale silently.
        .task { await store.followGlance() }
        // A fresh read on opening; the label follows the event stream. The
        // app's own reads (the Private AI pill and Cmd-Shift-M read
        // `model.daemonSettings`) refresh on opening too, as the legacy menu
        // did.
        .task { await store.load() }
        .onAppear { model.refreshAll() }
    }

    // MARK: Pills

    private var pills: some View {
        HStack(spacing: GlassTokens.Space.s3) {
            pill(.mode, caption: Self.modeCaption, value: modeValue, image: "bell", fill: modeFill)
            pill(.watch, caption: MonitorWords.watching, value: watchValue,
                 image: watchState == .paused ? "eye.slash" : "eye",
                 fill: .solid(watchState == .ready ? GlassTokens.Color.blue : GlassTokens.Color.menuPillOff))
            if let label = model.privateInferenceCopy?.destination {
                pill(.privateAI, caption: label, value: privateAIValue,
                     image: "arrow.left.arrow.right",
                     fill: .solid(privateAIPill == .on ? GlassTokens.Color.dataShared : GlassTokens.Color.menuPillOff))
            }
        }
        .frame(height: 44)
        .animation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.reveal), value: expanded)
    }

    private func pill(_ which: Pill, caption: String, value: String, image: String, fill: GlassPillFill) -> some View {
        GlassStatePill(caption: caption, value: value, systemImage: image, fill: fill, expanded: expanded == which) {
            sub = which
        }
        .onHover { if $0 { expanded = which } }
    }

    private var paused: Bool { watchState == .paused }

    /// The Watching pill's state (the stack-wide ScreenState rule): a core
    /// that is not running is core down, and never reads as On.
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

    /// Private AI on or off as the core reported it; nil when it did not
    /// say, which is drawn as unknown, never as off.
    private var privateAIOn: Bool? { model.daemonSettings?.privateInferenceOn }

    /// What the pill draws: the listener's reported tone, never the switch
    /// alone, so a switch left on over a dead listener never reads On.
    private var privateAIPill: MenuPanelStatus.PrivateAI {
        MenuPanelStatus.privateAI(
            on: privateAIOn,
            tone: PrivateInferenceSurface.tone(model.privateInferenceState, calls: model.privateInferenceCalls))
    }

    /// On and Off in the core's words; a listener that is not working while
    /// the switch is on reads the core's state line for it.
    private var privateAIValue: String {
        switch privateAIPill {
        case .on: MenuWords.on
        case .off: MonitorWords.off
        case .notWorking:
            model.privateInferenceCopy.map {
                PrivateInferenceSurface.stateLine(model.privateInferenceState, copy: $0, calls: model.privateInferenceCalls)
            } ?? "—"
        case .unknown: "—"
        }
    }

    /// The core's roll-up, never one worked out here from the folders.
    private var rollup: MenuPanelData.ModeRollup {
        guard !store.stale else { return .none }
        return MenuPanelData.rollup(store.status?.contributionMode)
    }

    /// The pill's words, from the core (`tc_contribution_mode_copy_json`).
    private static let modeCopy = ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())

    private var modeValue: String {
        MenuPanelData.modeValue(rollup, mode: store.status?.contributionMode, copy: Self.modeCopy)
    }

    private var modeFill: GlassPillFill {
        switch rollup {
        case .ask: .solid(GlassTokens.Color.menuModeAsk)
        case .armed: .solid(GlassTokens.Color.menuModeArmed)
        case .never: .solid(GlassTokens.Color.menuModeNever)
        case .mixed: .mixed
        case .none: .solid(GlassTokens.Color.menuPillOff)
        }
    }

    static var modeCaption: String { MonitorWords.table?.contributionMode ?? "" }

    // MARK: Sub-lists

    @ViewBuilder
    private func subList(_ which: Pill) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            GlassSubListHeader(title(which), closeLabel: title(which)) { sub = nil }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                switch which {
                case .mode: modeOptions
                case .watch: watchOptions
                case .privateAI: privateAIOptions
                }
            }
            .padding(GlassTokens.Space.s3)
            .glassTier(.card)
        }
    }

    private func title(_ which: Pill) -> String {
        switch which {
        case .mode: Self.modeCaption
        case .watch: MonitorWords.watching
        case .privateAI: model.privateInferenceCopy?.destination ?? MonitorWords.off
        }
    }

    /// The overrides, in the core's words, then the Mixed row (each folder
    /// on its own setting, sub-lined with the core's `clear` description).
    /// The checked row agrees with the pill (`MenuPanelData.listChecks`):
    /// an override in force, else the core's roll-up; nothing while the
    /// core is down. Auto contribute carries the core's partial line, and
    /// the override line shows while one is in force. Choosing an override
    /// shows its core confirmation in place of the choices; only its
    /// confirm button writes. Choosing Mixed clears an override: after the
    /// core's clear confirmation when the override is Ask me or Never and a
    /// folder's own setting is Automatic, since clearing would resume
    /// unattended sending there; otherwise at once
    /// (`MenuPanelStore.chooseMixed`). While the core is down,
    /// loading or stale every row is disabled
    /// (`MenuPanelStore.canChooseOverride`).
    @ViewBuilder
    private var modeOptions: some View {
        if let copy = Self.modeCopy {
            if let confirming = store.confirming {
                OverrideConfirmation(copy: confirming) { confirmed in
                    Task { await store.resolveConfirmation(confirmed: confirmed) }
                }
            } else {
                if !store.stale, store.status?.contributionOverride != nil {
                    Text(copy.overrideActive)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Group {
                    ForEach(copy.choices, id: \.mode) { choice in
                        let sub = MenuPanelData.partialLine(choice.mode, status: store.status, copy: copy) ?? choice.line
                        GlassOptionRow(
                            choice.label,
                            sub: sub,
                            fill: .solid(Self.modeFill(choice.mode)),
                            checked: MenuPanelData.listChecks(choice.mode, status: store.status, stale: store.stale)) {
                                if MenuPanelData.pressOnlyCloses(choice.mode, status: store.status, stale: store.stale) {
                                    self.sub = nil
                                } else {
                                    store.choose(choice.mode)
                                }
                            }
                            .accessibilityLabel(choice.label)
                            .accessibilityHint(sub)
                    }
                    // Mixed: no override, so each folder keeps its own
                    // setting. Choosing it clears an override in force,
                    // after the core's confirmation when a folder's own
                    // setting is Automatic (`MenuPanelStore.chooseMixed`).
                    GlassOptionRow(
                        copy.mixed,
                        sub: copy.clear,
                        fill: .mixed,
                        checked: MenuPanelData.listChecks(nil, status: store.status, stale: store.stale)) {
                            if store.status?.contributionOverride != nil {
                                Task { await store.chooseMixed() }
                            } else {
                                sub = nil
                            }
                        }
                        .accessibilityLabel(copy.mixed)
                        .accessibilityHint(copy.clear)
                }
                // No override before onboarding is done (R-43): it is a
                // grant, and first run is where consent is asked.
                .disabled(!store.canChooseOverride || model.requiresOnboarding)
                // A refused override: the failed request's red line, under
                // the choices it is about (Ron, 2026-10-09).
                if let refusal = store.overrideRefusal {
                    GlassAlert(refusal)
                        .accessibilityAddTraits(.isStaticText)
                }
            }
        }
    }

    private static func modeFill(_ mode: String) -> GlassRGBA {
        switch MenuPanelData.rollup(mode) {
        case .armed: GlassTokens.Color.menuModeArmed
        case .never: GlassTokens.Color.menuModeNever
        default: GlassTokens.Color.menuModeAsk
        }
    }

    /// The pause choices (`MenuBarWords`), or resume.
    @ViewBuilder
    private var watchOptions: some View {
        if paused {
            GlassOptionRow(MenuBarWords.resume, fill: .solid(GlassTokens.Color.blue), checked: false) {
                model.resume()
                sub = nil
            }
        } else {
            ForEach(Array([MenuBarWords.pauseHour, MenuBarWords.pauseMorning,
                           MenuBarWords.pauseIndefinite].enumerated()), id: \.offset) { index, label in
                GlassOptionRow(label, fill: .solid(GlassTokens.Color.menuPillOff), checked: false) {
                    model.pause(until: MenuBarWords.pauseUntil(index))
                    sub = nil
                }
            }
        }
    }

    /// On opens the window at the destination; Off is the tray's one write.
    /// Both sub-lines are the core's tray words.
    @ViewBuilder
    private var privateAIOptions: some View {
        if let copy = model.privateInferenceCopy {
            GlassOptionRow(MenuWords.on, sub: privateAIOn == true ? nil : copy.trayOpenToTurnOn,
                           fill: .solid(GlassTokens.Color.menuModeArmed), checked: privateAIOn == true) {
                open(.inference)
                sub = nil
            }
            GlassOptionRow(MonitorWords.off, sub: privateAIOn == true ? copy.trayTurnOff : nil,
                           fill: .solid(GlassTokens.Color.menuPillOff), checked: privateAIOn == false) {
                PrivateInferenceTray.perform(
                    on: privateAIOn == true, turnOff: { model.applyPrivateInference(false) },
                    open: { open(.inference) })
                sub = nil
            }
            .disabled(privateAIOn == true && (model.privateInferenceBusy || model.daemonSettings?.privateInference == nil))
        }
    }

    // MARK: Legend and graph

    private var legend: some View {
        let columns = store.columns
        // Stale data is a dash, never the last counts.
        let stale = store.stale
        return HStack(spacing: GlassTokens.Space.s3) {
            GlassLegendCell(MenuWords.shared, value: stale ? "—" : String(columns.reduce(0) { $0 + $1.up }), status: .shared)
            GlassLegendCell(MenuWords.kept, value: stale ? "—" : String(columns.reduce(0) { $0 + $1.down }), status: .kept)
        }
    }

    private var graph: some View {
        let columns = store.columns
        let start = Calendar.current.date(byAdding: .day, value: -(MenuPanelData.days - 1), to: Date()) ?? Date()
        let stale = store.stale
        return GlassDayGraph(
            columns: columns, paused: paused || stale,
            summary: "\(FlowMapScene.pair(MenuWords.shared, stale ? nil : columns.reduce(0) { $0 + $1.up })) · \(FlowMapScene.pair(MenuWords.kept, stale ? nil : columns.reduce(0) { $0 + $1.down }))",
            sharedChip: stale ? "—" : String(columns.reduce(0) { $0 + $1.up }),
            keptChip: stale ? "—" : String(columns.reduce(0) { $0 + $1.down }),
            leading: start.formatted(.dateTime.month(.abbreviated).day()),
            trailing: Date().formatted(.dateTime.month(.abbreviated).day()))
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

    // MARK: Recent activity

    @ViewBuilder
    private var recent: some View {
        let rows = MenuPanelData.recent(
            pending: store.pending, history: store.history, calls: store.calls,
            statusLabel: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) })
        if !rows.isEmpty && !store.stale {
            VStack(alignment: .leading, spacing: 0) {
                Text(MenuWords.recentActivity)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textTertiary)
                    .padding(.horizontal, GlassTokens.Space.s3)
                    .padding(.top, GlassTokens.Space.s1)
                ForEach(rows) { row in
                    GlassActivityRow(tool: row.tool, text: row.text, trailing: row.trailing) {
                        switch row.kind {
                        case .waiting: open(.traces(entryId: nil))
                        case .contributed: open(.home(.history))
                        case .call: open(.inference)
                        }
                    }
                }
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
            Button { open(.traces(entryId: nil)) } label: {
                HStack {
                    Text(FlowMapScene.dotPair(MenuWords.flagged, store.stale ? nil : MenuPanelData.flagged(store.pending)))
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").glassGlyph(10, weight: .semibold)
                }
            }
            hairline
            Button(MenuWords.manageRules) { open(MenuPanelData.manageRules(requiresOnboarding: model.requiresOnboarding)) }
            Button(MenuWords.settings) {
                NSApp.activate(ignoringOtherApps: true)
                navigation.requestSettings()
                openWindow(id: WindowID.settings)
            }
            Button(MenuWords.quit) { NSApp.terminate(nil) }
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

/// One contribution override's confirmation, inline in the pill's sub-list
/// (a sheet or alert inside `MenuBarExtra` is unreliable): the core's title,
/// body and, for Auto contribute, its arming disclosure, then cancel and
/// confirm. VoiceOver focus moves to the title when it appears.
private struct OverrideConfirmation: View {
    let copy: ContributionOverrideConfirmCopy
    let resolve: (Bool) -> Void
    @AccessibilityFocusState private var titleFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(copy.title)
                .glassType(GlassTokens.TypeScale.body.weight(.semibold))
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
                .accessibilityFocused($titleFocused)
            ForEach(Array(copy.paragraphs.enumerated()), id: \.offset) { _, paragraph in
                Text(paragraph)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: GlassTokens.Space.s3) {
                Spacer(minLength: 0)
                Button(copy.cancel) { resolve(false) }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .keyboardShortcut(.cancelAction)
                    .accessibilityLabel(copy.cancel)
                Button(copy.confirm) { resolve(true) }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                    .accessibilityLabel(copy.confirm)
            }
        }
        .padding(GlassTokens.Space.s2)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(copy.title)
        .onAppear { titleFocused = true }
    }
}

/// The popover's own glass, only where nothing else provides a material.
/// Inside `MenuBarExtra` it pins the window's top edge instead.
private struct PanelSurface: ViewModifier {
    let owns: Bool

    func body(content: Content) -> some View {
        if owns {
            content.glassSurface(.popover, radius: GlassTokens.Radius.menuPanel, floating: true)
        } else {
            content.background(MenuBarTopEdgePin().frame(width: 0, height: 0))
        }
    }
}

/// Holds the `MenuBarExtra` window's top edge under the menu bar while the
/// panel changes height. An `NSWindow` that is resized keeps its bottom
/// edge, so opening a pill's sub-list grew the window upward into the menu
/// bar, where AppKit pushed it back down, and closing it then dropped the
/// top edge away from the menu bar. The top is the one the system gave the
/// window when it opened (it becomes key on each opening).
struct MenuBarTopEdgePin: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Pin() }

    func updateNSView(_ view: NSView, context: Context) {}

    /// The origin that puts `frame`'s top edge at `top`, or nil when it is
    /// already there. Pure, so tested.
    static func pinnedOrigin(_ frame: CGRect, top: CGFloat?) -> CGPoint? {
        guard let top, abs(frame.maxY - top) > 0.5 else { return nil }
        return CGPoint(x: frame.minX, y: top - frame.height)
    }

    private final class Pin: NSView {
        private var top: CGFloat?
        private var observers: [NSObjectProtocol] = []

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
            top = nil
            guard let window else { return }
            if window.isKeyWindow { top = window.frame.maxY }
            let center = NotificationCenter.default
            observers.append(center.addObserver(
                forName: NSWindow.didBecomeKeyNotification, object: window, queue: .main
            ) { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    guard let window else { return }
                    self?.top = window.frame.maxY
                }
            })
            observers.append(center.addObserver(
                forName: NSWindow.didResizeNotification, object: window, queue: .main
            ) { [weak self, weak window] _ in
                MainActor.assumeIsolated {
                    guard let self, let window, window.isVisible,
                          let origin = MenuBarTopEdgePin.pinnedOrigin(window.frame, top: self.top) else { return }
                    window.setFrameOrigin(origin)
                }
            })
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }
    }
}


/// The menu-bar item in the glass system: the last seven days as a strip,
/// with the decisions-owed badge. Grey while paused.
struct MenuBarStripLabel: View {
    @ObservedObject var model: AppModel
    let store: MenuPanelStore
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        let available = model.startup == .running && !store.stale
        let condition = MenuPanelStatus.condition(
            decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
            paused: model.status.paused, available: model.startup == .running, stale: store.stale)
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
    /// non-template image it reaches the menu bar in its own colours.
    /// Padded past the badge's offset so the badge is not clipped.
    /// The condition, badge and nudge mark are the ones `body` speaks, so
    /// the drawn mark and its words never disagree.
    @MainActor
    private func rendered(
        condition: GlassMenuBarStrip.Condition, badge: Int?, mark: GlassMenuBarStrip.Mark
    ) -> NSImage {
        let renderer = ImageRenderer(content: GlassMenuBarStrip(
            columns: store.columns,
            condition: condition,
            badge: badge,
            mark: mark)
            .padding(.trailing, 6)
            .padding(.bottom, 2))
        renderer.scale = displayScale
        let image = renderer.nsImage ?? NSImage()
        image.isTemplate = false
        return image
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

    /// The Private AI pill's state.
    enum PrivateAI: Equatable {
        /// The core has not said whether the switch is on.
        case unknown
        case off
        /// The switch is on and the listener reports clear.
        case on
        /// The switch is on but the listener refused, crashed, is stopping
        /// or reports anything else: never drawn On.
        case notWorking
    }

    /// On only while the switch is on AND the listener's tone is clear
    /// (`PrivateInferenceIndicator.status`); fail closed otherwise.
    static func privateAI(on: Bool?, tone: PrivateInferenceTone) -> PrivateAI {
        guard let on else { return .unknown }
        guard on else { return .off }
        return PrivateInferenceIndicator.status(tone) == .on ? .on : .notWorking
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
    static var flagged: String { MonitorWords.table?.flagged ?? "" }
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
