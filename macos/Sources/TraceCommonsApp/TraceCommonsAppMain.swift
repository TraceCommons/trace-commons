import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The macOS contributor shell.
///
/// One application bundle, no second binary: the app links the C ABI and
/// hosts the watch/upload/digest loops in-process (see the macOS design
/// spec). It is a regular app: a Dock icon AND a menu-bar item. It was
/// menu-bar-only until `LSUIElement` was removed, because a status item is
/// not a reliable way to reach an app -- on a notched display with a full
/// menu bar it is assigned a frame that never draws, and there was no other
/// door.
@main
struct TraceCommonsShell: App {
    private let insightsStoreSelection: InsightsStoreSelection
    @StateObject private var model = AppModel()
    @State private var compute = ComputeModel()
    @State private var navigation = MainWindowNavigation()
    @State private var missionDrafts = MissionDraftsModel()
    #if DEBUG
    /// The glass menu-bar popover's data (R13), shared by its item and panel.
    /// No client until the daemon runs: the menu-bar label attaches the
    /// app's live one (`AppModel.daemonData`), never sample data.
    @State private var menuPanel = MenuPanelStore(client: nil)
    #endif
    /// Quit confirmation, Dock reopen and invite links all arrive outside
    /// SwiftUI's reach. See `AppDelegate`.
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    init() {
        insightsStoreSelection = .parse(arguments: CommandLine.arguments)
        // Earlier builds wrote the preview sheet's recent-search list to
        // UserDefaults. That list is the contributor's own record of what
        // they were checking for -- client names, employers, unreleased
        // products -- so it is removed here rather than only when a preview
        // sheet next happens to open. An upgraded install where nobody opens
        // a preview again would otherwise keep those terms forever, with no
        // surface left in the app that could clear them.
        RecentSearches.purgeLegacyStore()
    }

    /// Whether the glass menu-bar panel (R13) stands in for the shipping
    /// menu. Debug builds only, on `TRACE_COMMONS_GLASS_MENU=1`; a release
    /// build always has the shipping menu.
    private static let glassMenu: Bool = {
        #if DEBUG
        ProcessInfo.processInfo.environment["TRACE_COMMONS_GLASS_MENU"] == "1"
        #else
        false
        #endif
    }()

    var body: some Scene {
        // Exactly one of the two menu-bar items is inserted, so exactly one
        // `Launcher` (which starts the app's services) runs.
        MenuBarExtra(isInserted: .constant(!Self.glassMenu)) {
            MenuBarContent()
                .environmentObject(model)
                .tint(TC.accent)
        } label: {
            Launcher(model: model, compute: compute, navigation: navigation,
                     appDelegate: appDelegate, missionDrafts: missionDrafts)
        }

        #if DEBUG
        // The glass menu-bar panel (R13 of #1173), in place of the shipping
        // menu when TRACE_COMMONS_GLASS_MENU=1, until R15.
        MenuBarExtra(isInserted: .constant(Self.glassMenu)) {
            MenuBarGlassPanel(navigation: navigation, store: menuPanel)
                .environmentObject(model)
                .tint(TC.accent)
        } label: {
            Launcher(model: model, compute: compute, navigation: navigation,
                     appDelegate: appDelegate, missionDrafts: missionDrafts, menuPanel: menuPanel)
        }
        .menuBarExtraStyle(.window)
        #endif

        Window("Trace Commons", id: WindowID.main) {
            MainWindowView(navigation: navigation, missionDrafts: missionDrafts,
                           insightsStoreSelection: insightsStoreSelection)
                .environmentObject(model)
                .environment(compute)
                .frame(minWidth: 760, minHeight: 520)
                // The brand purple (D3), not the platform blue. Overriding the
                // user's chosen accent colour is a real departure from macOS
                // convention and it is made on purpose: the accent is the
                // single strongest cue that this app is the Trace product.
                // It tints fills (prominent buttons, checked boxes), so it is
                // the purple that carries white at 6.9:1 in both schemes.
                // Everything else about the controls -- shape, focus ring,
                // keyboard behaviour -- stays stock.
                .tint(TC.accent)
        }
        .defaultSize(width: 940, height: 660)
        // Cmd-1..7 for the seven destinations, and Cmd-Shift-M for the one
        // switch worth reaching without the window. Menu items, so they are
        // in-app only; see `MainWindowCommands`.
        .commands {
            MainWindowCommands(model: model, compute: compute, navigation: navigation,
                               missionDrafts: missionDrafts)
            #if DEBUG
            MonitorWindowCommands()
            #endif
        }

        #if DEBUG
        // The glass monitor window and the Settings window (R5 of #1173),
        // in debug builds until the screens they frame match the design.
        // TRACE_COMMONS_MONITOR=1 opens the monitor at launch.
        Window("Monitor", id: WindowID.monitor) {
            MonitorWindowView(navigation: navigation)
                .environmentObject(model)
                // The monitor reads the daemon, so it starts services on
                // appear (D-11).
                .onAppear { navigation.activateServicesForWindow() }
                .environment(compute)
                .tint(TC.accent)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth,
                     height: GlassTokens.Size.windowHeight)
        .windowResizability(.contentMinSize)

        // The menu-bar item and popover in a window (R13 of #1173), for
        // review on a menu bar with no room for the item.
        // TRACE_COMMONS_MENU_PREVIEW=1 opens it at launch.
        Window("Menu bar", id: WindowID.menuPreview) {
            MenuBarPreviewWindow(navigation: navigation, store: menuPanel)
                .environmentObject(model)
                .tint(TC.accent)
        }
        .windowResizability(.contentSize)

        // First run in a glass pane over the scene (R12 of #1173).
        // TRACE_COMMONS_FIRST_RUN=1 opens it at launch.
        Window("First run", id: WindowID.firstRun) {
            FirstRunWindowView()
                .environmentObject(model)
                .tint(TC.accent)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 860, height: 760)

        Settings {
            MonitorSettingsWindow(navigation: navigation)
                .environmentObject(model)
                // Settings loads and saves through the daemon; opened
                // before services started it would otherwise show nothing
                // and drop edits.
                .onAppear { navigation.activateServicesForWindow() }
                .environment(compute)
                .tint(TC.accent)
        }
        #endif
    }
}

#if DEBUG
/// Window ▸ Monitor, to open the glass monitor window in a debug build.
private struct MonitorWindowCommands: Commands {
    var body: some Commands {
        CommandGroup(after: .windowList) {
            OpenMonitorButton()
        }
    }
}

private struct OpenMonitorButton: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Monitor") { openWindow(id: WindowID.monitor) }
        Button("First run") { openWindow(id: WindowID.firstRun) }
    }
}
#endif

/// The menu-bar label, plus the one-time launch work. It lives in a view
/// rather than in `App` so it can reach `openWindow`, which the notification
/// `Review` action and the queue-full banner both need.
private struct Launcher: View {
    @ObservedObject var model: AppModel
    let compute: ComputeModel
    let navigation: MainWindowNavigation
    let appDelegate: AppDelegate
    let missionDrafts: MissionDraftsModel
    #if DEBUG
    /// Set for the glass menu-bar item (R13): its strip replaces the mark.
    var menuPanel: MenuPanelStore?
    #endif
    @Environment(\.openWindow) private var openWindow
    #if DEBUG
    @Environment(\.openSettings) private var openSettings
    #endif

    var body: some View {
        label
            .task { launch() }
    }

    @ViewBuilder
    private var label: some View {
        #if DEBUG
        if let menuPanel {
            MenuBarStripLabel(model: model, store: menuPanel)
        } else {
            MenuBarLabel(model: model)
        }
        #else
        MenuBarLabel(model: model)
        #endif
    }

    @MainActor
    private func activateServices() {
        navigation.activateServicesIfNeeded { startServices() }
    }

    /// The service start, once, from whichever comes first: launch, or a
    /// window that needs services (Settings, the monitor) opening (D-11).
    @MainActor
    private func startServices() {
        model.start()
        Task {
            await compute.start()
            compute.startMonitoring()
        }
        // Update checks begin here and nowhere else. UpdateController itself
        // decides whether Sparkle runs at all: under a Homebrew install this
        // call constructs no updater and schedules nothing.
        UpdateController.shared.start()
        Notifier.shared.configure()
    }

    @MainActor
    private func launch() {
        missionDrafts.loadCopy()
        OpenMonitor.handler = { destination in
            NSApp.activate(ignoringOtherApps: true)
            open(destination)
        }
        appDelegate.compute = compute
        appDelegate.model = model
        navigation.registerServiceStart { startServices() }
        activateServices()
        // The only thing a notification action may do is open the Monitor
        // at Traces.
        Notifier.shared.onReview = { OpenMonitor.request(.traces(entryId: nil)) }

        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: .main
        ) { _ in
            MainActor.assumeIsolated { model.shutdown() }
        }

        // Development hook, same family as TRACE_COMMONS_SHOW_WINDOW below:
        // pins the app to one appearance so a capture run can produce light
        // and dark images without touching the machine's system setting.
        // Unset (the normal case) leaves the app following the system, which
        // is the only correct behaviour for a shipping build.
        switch ProcessInfo.processInfo.environment["TRACE_COMMONS_APPEARANCE"] {
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        default: break
        }

        // Used by scripts/run-demo.sh to bring the window up for a
        // screenshot. Since the app became a regular one it opens its window
        // on a normal launch anyway, so this now only matters for the paths
        // that do not -- a login launch, and `open -g`.
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_SHOW_WINDOW"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                OpenMonitor.request()
            }
        }
        #if DEBUG
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_MONITOR"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                openWindow(id: WindowID.monitor)
            }
        }
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_MENU_PREVIEW"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                openWindow(id: WindowID.menuPreview)
            }
        }
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_FIRST_RUN"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                openWindow(id: WindowID.firstRun)
            }
        }
        #endif
        DebugScreenshot.scheduleIfRequested(model: model)
        SelfTest.runIfRequested(model: model)
    }

    /// One `OpenMonitor` request: first run while onboarding is required,
    /// the Monitor otherwise, which consumes `navigation.pending`; a
    /// Settings destination also opens Settings at its section.
    @MainActor
    private func open(_ destination: MonitorDestination?) {
        #if DEBUG
        navigation.pending = destination
        let opening = LaunchRouting.opening(destination, requiresOnboarding: model.requiresOnboarding)
        switch opening.window {
        case .firstRun: openWindow(id: WindowID.firstRun)
        case .monitor: openWindow(id: WindowID.monitor)
        }
        if let section = opening.settings {
            navigation.settingsSection = section
            openSettings()
        }
        #else
        // Until R15 a release build has only the legacy window: open it at
        // the matching section. T11 deletes this branch with the window
        // (ruling R-33).
        if let section = MainWindowNavigation.legacySection(for: destination) {
            navigation.section = section
        }
        openWindow(id: WindowID.main)
        #endif
    }
}

enum WindowID {
    static let main = "trace-commons-main"
    /// The glass monitor window (R5), debug builds only for now.
    static let monitor = "trace-commons-monitor"
    /// The glass first-run pane (R12), debug builds only for now.
    static let firstRun = "trace-commons-first-run"
    /// The menu-bar item and popover in a window (R13), for review where
    /// the menu bar has no room for the item. Debug builds only.
    static let menuPreview = "trace-commons-menu-preview"
}
