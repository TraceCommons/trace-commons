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
    @Environment(\.openSettings) private var openSettings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
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
            legend
            graph
            recent
            menuItems
        }
        .padding(GlassTokens.Space.s5)
        .frame(width: Self.width)
        .modifier(PanelSurface(owns: ownsSurface))
        .animation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide), value: sub)
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
            pill(.mode, caption: Self.modeCaption, value: modeValue, image: "bell.fill", fill: modeFill)
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

    /// The overrides, in the core's words, with the core's roll-up checked,
    /// its partial line under Auto contribute, and its override line and
    /// clear action while one is in force. Choosing one shows that
    /// override's core confirmation in place of the choices; only its
    /// confirm button writes. While the core is down, loading or stale the
    /// choices are disabled (`MenuPanelStore.canChooseOverride`).
    @ViewBuilder
    private var modeOptions: some View {
        if let copy = Self.modeCopy {
            if let confirming = store.confirming {
                OverrideConfirmation(copy: confirming) { confirmed in
                    Task { await store.resolveConfirmation(confirmed: confirmed) }
                }
            } else {
                if store.status?.contributionOverride != nil {
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
                            checked: MenuPanelData.rollup(choice.mode) == rollup) {
                                store.choose(choice.mode)
                            }
                            .accessibilityLabel(choice.label)
                            .accessibilityHint(sub)
                    }
                    if store.status?.contributionOverride != nil {
                        Button(copy.clear) { Task { await store.clearOverride() } }
                            .buttonStyle(GlassButtonStyle(.link, small: true))
                            .accessibilityLabel(copy.clear)
                    }
                }
                // No override before onboarding is done (R-43): it is a
                // grant, and first run is where consent is asked.
                .disabled(!store.canChooseOverride || model.requiresOnboarding)
                if let refusal = store.overrideRefusal {
                    Text(refusal)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
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

    private var menuItems: some View {
        VStack(alignment: .leading, spacing: 0) {
            hairline
            Button { open(.traces(entryId: nil)) } label: {
                HStack {
                    Text(FlowMapScene.pair(MenuWords.flagged, store.stale ? nil : MenuPanelData.flagged(store.pending)))
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").glassGlyph(10, weight: .semibold)
                }
            }
            hairline
            Button(MenuWords.manageRules) { open(MenuPanelData.manageRules(requiresOnboarding: model.requiresOnboarding)) }
            Button(MenuWords.settings) {
                NSApp.activate(ignoringOtherApps: true)
                openSettings()
            }
            Button(MenuWords.quit) { NSApp.terminate(nil) }
        }
        .buttonStyle(GlassMenuRowStyle())
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
private struct PanelSurface: ViewModifier {
    let owns: Bool

    func body(content: Content) -> some View {
        if owns {
            content.glassSurface(.popover, radius: GlassTokens.Radius.menuPanel, floating: true)
        } else {
            content
        }
    }
}

/// The menu-bar item in the glass system: the last seven days as a strip,
/// with the decisions-owed badge. Grey while paused.
struct MenuBarStripLabel: View {
    @ObservedObject var model: AppModel
    let store: MenuPanelStore

    var body: some View {
        GlassMenuBarStrip(
            columns: store.columns,
            condition: MenuPanelStatus.condition(
                decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
                paused: model.status.paused, available: model.startup == .running, stale: store.stale),
            badge: MenuPanelStatus.badge(model.decisionsOwed))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(MenuBarStatus.accessibilityLabel(
                decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
                paused: model.status.paused, available: model.startup == .running))
            // The label is always alive, so it owns the subscription: the
            // app's live client, re-attached whenever the daemon restarts.
            .task(id: model.liveData.map(ObjectIdentifier.init)) {
                store.attach(model.daemonData, configDirectory: model.configDirectory)
                await store.run()
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
    let store: MenuPanelStore

    var body: some View {
        VStack(alignment: .trailing, spacing: GlassTokens.Space.s4) {
            MenuBarStripLabel(model: model, store: store)
                .padding(.horizontal, GlassTokens.Space.s4)
                .background(Capsule().fill(GlassColor.ink(0.12)))
            MenuBarGlassPanel(store: store, ownsSurface: true)
        }
        .padding(GlassTokens.Space.s10)
        .background(LinearGradient(colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color], startPoint: .top, endPoint: .bottom))
    }
}
#endif
