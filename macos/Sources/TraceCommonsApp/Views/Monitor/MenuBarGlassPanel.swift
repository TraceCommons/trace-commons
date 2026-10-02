#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The menu-bar popover (R13 of #1173), from the "Menu bar item and
/// popover" handoff: three state pills, a sub-list, the shared/kept legend
/// and day graph, recent activity, and shortcuts into the app. Debug-only,
/// in place of the shipping menu, until R15 (`TRACE_COMMONS_GLASS_MENU=1`).
///
/// The handoff's rules hold here:
/// - Nothing is sent from the popover. The only write is the shipping
///   menu's own: pausing, resuming, and turning Private AI off.
/// - Private AI "On" opens the window at its destination; a menu press
///   never turns it on.
/// - The mode pill reflects the folders' modes rolled up (one mode, or
///   Mixed). Its overrides are shown and do nothing yet: applying one to
///   every folder needs the core's override and confirmation (Kristi's
///   lane), and arming never happens from a menu press.
/// - No projected credit, and the badge is decisions owed.
struct MenuBarGlassPanel: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigation: MainWindowNavigation
    let store: MenuPanelStore

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
        .glassSurface(.popover, radius: GlassTokens.Radius.menuPanel, floating: true)
        .animation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide), value: sub)
        .task { await store.run() }
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
                     fill: .solid(privateAIOn == true ? GlassTokens.Color.dataShared : GlassTokens.Color.menuPillOff))
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

    private var privateAIValue: String {
        switch privateAIOn {
        case true?: MenuWords.on
        case false?: MonitorWords.off
        case nil: "—"
        }
    }

    private var rollup: MenuPanelData.ModeRollup {
        guard let projects = store.projects else { return .none }
        return MenuPanelData.rollup(projects.map(\.mode))
    }

    private var modeValue: String {
        switch rollup {
        case .ask: ProjectCopy.modeChoiceLabel(.ask)
        case .armed: ProjectCopy.modeChoiceLabel(.autoUpload)
        case .never: ProjectCopy.modeChoiceLabel(.ignore)
        case .mixed: MenuWords.mixed
        case .none: "—"
        }
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

    /// The overrides, shown with the current roll-up checked. Disabled: the
    /// core has no override yet, and arming is never done from a menu press.
    private var modeOptions: some View {
        Group {
            GlassOptionRow(ProjectCopy.modeChoiceLabel(.ask), fill: .solid(GlassTokens.Color.menuModeAsk),
                           checked: rollup == .ask) {}
            GlassOptionRow(ProjectCopy.modeChoiceLabel(.autoUpload), fill: .solid(GlassTokens.Color.menuModeArmed),
                           checked: rollup == .armed) {}
            GlassOptionRow(ProjectCopy.modeChoiceLabel(.ignore), fill: .solid(GlassTokens.Color.menuModeNever),
                           checked: rollup == .never) {}
        }
        .disabled(true)
    }

    /// The shipping menu's pause choices, in its words, or resume.
    @ViewBuilder
    private var watchOptions: some View {
        if paused {
            GlassOptionRow(MenuBarContent.resumeLabel, fill: .solid(GlassTokens.Color.blue), checked: false) {
                model.resume()
                sub = nil
            }
        } else {
            ForEach(Array([MenuBarContent.pauseHourLabel, MenuBarContent.pauseMorningLabel,
                           MenuBarContent.pauseIndefiniteLabel].enumerated()), id: \.offset) { index, label in
                GlassOptionRow(label, fill: .solid(GlassTokens.Color.menuPillOff), checked: false) {
                    model.pause(until: MenuBarContent.pauseUntil(index))
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
                openMain(.privateInference)
                sub = nil
            }
            GlassOptionRow(MonitorWords.off, sub: privateAIOn == true ? copy.trayTurnOff : nil,
                           fill: .solid(GlassTokens.Color.menuPillOff), checked: privateAIOn == false) {
                MenuBarContent.performPrivateInferenceTray(
                    on: privateAIOn == true, turnOff: { model.applyPrivateInference(false) },
                    open: { openMain(.privateInference) })
                sub = nil
            }
            .disabled(privateAIOn == true && (model.privateInferenceBusy || model.daemonSettings?.privateInference == nil))
        }
    }

    // MARK: Legend and graph

    private var legend: some View {
        let columns = store.columns
        return HStack(spacing: GlassTokens.Space.s3) {
            GlassLegendCell(MenuWords.shared, value: String(columns.reduce(0) { $0 + $1.up }), status: .shared)
            GlassLegendCell(MenuWords.kept, value: String(columns.reduce(0) { $0 + $1.down }), status: .kept)
        }
    }

    private var graph: some View {
        let columns = store.columns
        let start = Calendar.current.date(byAdding: .day, value: -(MenuPanelData.days - 1), to: Date()) ?? Date()
        return GlassDayGraph(
            columns: columns, paused: paused,
            sharedChip: String(columns.reduce(0) { $0 + $1.up }),
            keptChip: String(columns.reduce(0) { $0 + $1.down }),
            leading: start.formatted(.dateTime.month(.abbreviated).day()),
            trailing: Date().formatted(.dateTime.month(.abbreviated).day()))
    }

    // MARK: Recent activity

    @ViewBuilder
    private var recent: some View {
        let rows = MenuPanelData.recent(
            pending: store.pending, history: store.history, calls: store.calls,
            statusLabel: { model.publicRunCopy?.contributionStatusLabel(for: $0) })
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                Text(MenuWords.recentActivity)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textTertiary)
                    .padding(.horizontal, GlassTokens.Space.s3)
                    .padding(.top, GlassTokens.Space.s1)
                ForEach(rows) { row in
                    GlassActivityRow(tool: row.tool, text: row.text, trailing: row.trailing) {
                        switch row.kind {
                        case .waiting: openMain(.queue)
                        case .contributed: openMain(.history)
                        case .call: openMain(.privateInference)
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
            Button { openMain(.queue) } label: {
                HStack {
                    Text(FlowMapScene.pair(MenuWords.flagged, MenuPanelData.flagged(store.pending)))
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").glassGlyph(10, weight: .semibold)
                }
            }
            hairline
            Button(MenuWords.manageRules) { openMain(.settings) }
            Button(MenuWords.settings) {
                NSApp.activate(ignoringOtherApps: true)
                openSettings()
            }
            Button(MenuWords.quit) { NSApp.terminate(nil) }
        }
        .buttonStyle(GlassMenuRowStyle())
    }

    private var hairline: some View {
        Rectangle().fill(Color.white.opacity(0.12)).frame(height: 1)
            .padding(.vertical, GlassTokens.Space.s2)
            .accessibilityHidden(true)
    }

    private func openMain(_ section: MainWindowView.Section) {
        navigation.section = section
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: WindowID.main)
    }
}

/// The menu-bar item in the glass system: the last seven days as a strip,
/// with the decisions-owed badge. Grey while paused.
struct MenuBarStripLabel: View {
    @ObservedObject var model: AppModel
    let store: MenuPanelStore

    var body: some View {
        GlassMenuBarStrip(
            columns: store.columns, paused: model.status.paused,
            badge: MenuPanelStatus.badge(model.decisionsOwed))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(MenuBarStatus.accessibilityLabel(
                decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
                paused: model.status.paused, available: model.startup == .running))
            .task { await store.load() }
    }
}

/// The badge for the menu-bar item. Pure, so tested.
enum MenuPanelStatus {
    /// Decisions owed when there are any. Zero and unknown draw none.
    static func badge(_ decisionsOwed: Int?) -> Int? {
        guard let decisionsOwed, decisionsOwed > 0 else { return nil }
        return decisionsOwed
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
#endif

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
                .background(Capsule().fill(Color.white.opacity(0.12)))
            MenuBarGlassPanel(navigation: navigation, store: store)
        }
        .padding(GlassTokens.Space.s10)
        .background(LinearGradient(colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color], startPoint: .top, endPoint: .bottom))
    }
}
#endif
