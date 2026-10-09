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
///
/// The launch is quiet (R-44): an onboarded install puts no window on
/// screen. SwiftUI opens the first `Window` scene, the Monitor, on a normal
/// launch; from macOS 15 that scene's launch is suppressed, and the launch
/// opens what it needs itself (`Launcher.openAtLaunch`). A scene modifier
/// cannot be applied under `if #available` without dropping the scene on
/// the older system (`SceneBuilder` has no `else`), so the policy is a type
/// chosen once, here, rather than a branch in `body`. On macOS 14 the
/// Monitor still comes up on a normal launch, as the main window did
/// before R15, and is closed when the launch opens first run instead.
@main
enum TraceCommonsEntry {
    static func main() {
        if #available(macOS 15, *) {
            TraceCommonsShell<MonitorLaunchSuppressed>.main()
        } else {
            TraceCommonsShell<MonitorLaunchAutomatic>.main()
        }
    }
}

/// Whether SwiftUI opens the Monitor by itself at launch.
protocol MonitorLaunchPolicy {
    associatedtype Launched: Scene
    @MainActor static func launch(_ monitor: MonitorScene) -> Launched
}

/// macOS 14: SwiftUI's own launch window, which cannot be suppressed there.
enum MonitorLaunchAutomatic: MonitorLaunchPolicy {
    static func launch(_ monitor: MonitorScene) -> MonitorScene { monitor }
}

/// macOS 15 and later: no Monitor unless something asks for it.
@available(macOS 15, *)
enum MonitorLaunchSuppressed: MonitorLaunchPolicy {
    static func launch(_ monitor: MonitorScene) -> some Scene { monitor.defaultLaunchBehavior(.suppressed) }
}

struct TraceCommonsShell<MonitorLaunch: MonitorLaunchPolicy>: App {
    private let insightsStoreSelection: InsightsStoreSelection
    @StateObject private var model = AppModel()
    @State private var compute = ComputeModel()
    @State private var navigation = MainWindowNavigation()
    @State private var missionDrafts = MissionDraftsModel()
    /// The glass menu-bar popover's data (R13), shared by its item and panel.
    /// No client until the daemon runs: the menu-bar label attaches the
    /// app's live one (`AppModel.daemonData`), never sample data.
    @State private var menuPanel = MenuPanelStore(client: nil)
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

    var body: some Scene {
        // The glass strip and panel (R13 of #1173), the only menu-bar item
        // since R15. Its label is the `Launcher`, which starts the app's
        // services.
        MenuBarExtra {
            MenuBarGlassPanel(navigation: navigation, store: menuPanel)
                .environmentObject(model)
                .tint(GlassTokens.Color.purpleSoft.color)
        } label: {
            Launcher(model: model, compute: compute, navigation: navigation,
                     appDelegate: appDelegate, missionDrafts: missionDrafts, menuPanel: menuPanel)
        }
        .menuBarExtraStyle(.window)

        // The main window (R5 of #1173, the release default since R15).
        // `OpenMonitor` opens it, or first run while onboarding is required.
        MonitorLaunch.launch(MonitorScene(
            model: model, compute: compute, navigation: navigation,
            insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts))

        #if DEBUG
        // The menu-bar item and popover in a window (R13 of #1173), for
        // review on a menu bar with no room for the item. Debug builds only
        // (D-18). TRACE_COMMONS_MENU_PREVIEW=1 opens it at launch.
        Window("Menu bar", id: WindowID.menuPreview) {
            MenuBarPreviewWindow(navigation: navigation, store: menuPanel)
                .environmentObject(model)
                .tint(GlassTokens.Color.purpleSoft.color)
        }
        .windowResizability(.contentSize)
        #endif

        // First run in a glass pane over the scene (R12 of #1173): the
        // onboarding gate, opened by `OpenMonitor` while onboarding is
        // required.
        Window("First run", id: WindowID.firstRun) {
            FirstRunWindowView(navigation: navigation)
                .environmentObject(model)
                .tint(GlassTokens.Color.purpleSoft.color)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 860, height: 760)

    }
}

/// The Monitor's window scene, named so the launch policy
/// (`MonitorLaunchPolicy`) has a type to take.
struct MonitorScene: Scene {
    let model: AppModel
    let compute: ComputeModel
    let navigation: MainWindowNavigation
    let insightsStoreSelection: InsightsStoreSelection
    let missionDrafts: MissionDraftsModel

    var body: some Scene {
        Window("Monitor", id: WindowID.monitor) {
            MonitorWindowView(navigation: navigation, insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts)
                .environmentObject(model)
                // The monitor reads the daemon, so it starts services on
                // appear (D-11).
                .onAppear { navigation.activateServicesForWindow() }
                .environment(compute)
                // The brand purple (D3), not the platform blue. Overriding the
                // user's chosen accent colour is a real departure from macOS
                // convention and it is made on purpose: the accent is the
                // single strongest cue that this app is the Trace product.
                // Everything else about the controls -- shape, focus ring,
                // keyboard behaviour -- stays stock.
                .tint(GlassTokens.Color.purpleSoft.color)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: GlassThreePane<EmptyView, EmptyView, EmptyView>.defaultWidth,
                     height: GlassTokens.Size.windowHeight)
        // The panes set the window's limits (without the map it is exactly
        // its panes), and showing or hiding a pane grows or shrinks the
        // window on its right (`GlassPaneLayout`), so the window follows its
        // content's size.
        .windowResizability(.contentSize)
        // Cmd-1..3 for the three tabs, and Cmd-Shift-M for the one switch
        // worth reaching without the window. Menu items, so they are in-app
        // only; see `MonitorCommands`.
        .commands {
            MonitorCommands(model: model, navigation: navigation)
        }
    }
}

/// The View menu's Monitor commands, and the one shortcut that acts rather
/// than navigates.
///
/// In-app only, by construction: these are menu items, so they fire when
/// this app is frontmost and never while another app owns the keyboard. A
/// global, system-wide hotkey would need a registration this app does not
/// make and is deliberately out of scope.
///
/// It authors no wording: each tab's item is the tab's own word
/// (`MonitorWindowView.Tab.title`), the switch's is the core's. Every item
/// opens through `OpenMonitor`, so while onboarding is required Home and
/// Traces open first run, and Inference opens the Monitor on its own tab,
/// where Private AI sign-in is (R-38).
struct MonitorCommands: Commands {
    @ObservedObject var model: AppModel
    let navigation: MainWindowNavigation

    /// Cmd-N for the Nth tab.
    static let tabModifiers: EventModifiers = [.command]
    /// Cmd-Shift-M for the switch. Shifted so it cannot be reached by the
    /// same reflex that reaches a tab: this one changes what the machine is
    /// doing, the other three only change what is on screen.
    static let toggleModifiers: EventModifiers = [.command, .shift]
    static let toggleKey: Character = "m"

    /// The tab's shortcut: its position in the tab strip.
    static func shortcut(_ tab: MonitorWindowView.Tab) -> Character {
        switch tab {
        case .home: "1"
        case .inference: "2"
        case .traces: "3"
        }
    }

    /// Where the tab's item goes. Home lands on its overview.
    static func destination(_ tab: MonitorWindowView.Tab) -> MonitorDestination {
        switch tab {
        case .home: .home(.overview)
        case .inference: .inference
        case .traces: .traces(entryId: nil)
        }
    }

    var body: some Commands {
        CommandGroup(after: .sidebar) {
            ForEach(MonitorWindowView.Tab.allCases) { tab in
                Button(tab.title) { OpenMonitor.request(Self.destination(tab)) }
                    .keyboardShortcut(KeyEquivalent(Self.shortcut(tab)), modifiers: Self.tabModifiers)
            }
            Divider()
            toggle
        }
        // Cmd-comma: Settings is Ron's modal over the Monitor (#1146; #1241
        // Task 10), not a window of its own.
        CommandGroup(replacing: .appSettings) {
            OpenSettingsModalButton(navigation: navigation)
        }
    }

    /// The switch, under the same asymmetry the menu-bar row follows.
    ///
    /// This is a menu, and a menu press must not enable answering: the on
    /// direction raises the window at the destination, where
    /// `offer_exposure` is, and writes nothing. It shares
    /// `PrivateInferenceTray.perform` with the menu-bar row rather than
    /// restating the rule, because two statements of one rule is how this
    /// shortcut came to disagree with that row in the first place.
    ///
    /// The off direction *sets* false; it does not invert. An inverted press
    /// against a stale switch position is an enable.
    @ViewBuilder
    private var toggle: some View {
        if let copy = model.privateInferenceCopy {
            let on = model.daemonSettings?.privateInferenceOn ?? false
            Button(PrivateInferenceTray.label(on: on, copy: copy)) {
                PrivateInferenceTray.perform(
                    on: on,
                    turnOff: { model.applyPrivateInference(false) },
                    open: { OpenMonitor.request(.inference) })
            }
            .keyboardShortcut(KeyEquivalent(Self.toggleKey), modifiers: Self.toggleModifiers)
            // Only the writing direction can be unavailable. Navigation stays
            // reachable whatever the daemon is doing.
            .disabled(
                on && (model.privateInferenceBusy || model.daemonSettings?.privateInference == nil))
        }
    }
}

private struct OpenSettingsModalButton: View {
    let navigation: MainWindowNavigation
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button(MonitorWords.table?.settings ?? "") {
            openWindow(id: WindowID.monitor)
            navigation.requestSettings()
        }
        .keyboardShortcut(",", modifiers: .command)
    }
}

/// The menu-bar label, plus the one-time launch work. It lives in a view
/// rather than in `App` so it can reach `openWindow`, which every
/// `OpenMonitor` request needs.
private struct Launcher: View {
    @ObservedObject var model: AppModel
    let compute: ComputeModel
    let navigation: MainWindowNavigation
    let appDelegate: AppDelegate
    let missionDrafts: MissionDraftsModel
    /// The glass menu-bar item's data (R13), drawn as its strip.
    let menuPanel: MenuPanelStore
    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismissWindow) private var dismissWindow

    /// Whether the launch has made its one `OpenMonitor` request.
    @State private var openedAtLaunch = false

    var body: some View {
        MenuBarStripLabel(model: model, store: menuPanel)
            .task { launch() }
            .onChange(of: model.launchOpening, initial: true) { _, opening in
                openAtLaunch(opening)
            }
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
        OpenMonitor.handler = { destination, activate in
            if activate { NSApp.activate(ignoringOtherApps: true) }
            open(destination)
        }
        appDelegate.compute = compute
        appDelegate.model = model
        navigation.registerServiceStart { startServices() }
        activateServices()
        // The only thing a digest action may do is open the Monitor at
        // Traces.
        Notifier.shared.onReview = { OpenMonitor.request(.traces(entryId: nil)) }
        // A re-engagement button sends its one request (best effort: the
        // place opens either way, and a "Not now" the core did not take
        // leaves the suggestion as it was) and opens its place, if any.
        Notifier.shared.onNudge = { intent in
            let effect = NudgeSurface.effect(intent)
            if let client = model.daemonData {
                Task { try? await NudgeSurface.send(effect, through: client) }
            }
            if let destination = effect.destination {
                OpenMonitor.request(MonitorDestination(nudge: destination))
            }
        }

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

        // Used by scripts/run-demo.sh to bring the window up, activated, for
        // a screenshot: the Monitor, or first run while onboarding is
        // required. The launch's own request (`openAtLaunch`) opens no
        // window for an onboarded install, so a login launch or `open -g`
        // puts nothing on screen (R-44); this hook always opens one.
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_SHOW_WINDOW"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                OpenMonitor.request()
            }
        }
        #if DEBUG
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_MENU_PREVIEW"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                openWindow(id: WindowID.menuPreview)
            }
        }
        #endif
        DebugScreenshot.scheduleIfRequested(model: model)
        SelfTest.runIfRequested(model: model)
    }

    /// The launch's window (`LaunchRouting.launchOpening`), once, when the
    /// core has said enough. An onboarded install opens nothing (R-44).
    /// First run opens activated, alone: the Monitor SwiftUI may have
    /// opened at launch (macOS 14), or that an opener opened before the
    /// core answered (a cold-start invite link or notification), is
    /// closed. A destination such an opener left keeps waiting in
    /// `navigation.pending` for first run's hand-off: this plain request
    /// does not clear it (`MainWindowNavigation.leave`). A refused daemon
    /// opens the Monitor at the refusal, without taking focus.
    @MainActor
    private func openAtLaunch(_ opening: LaunchRouting.LaunchOpening) {
        guard opening != .wait, !openedAtLaunch else { return }
        openedAtLaunch = true
        switch opening {
        case .firstRun:
            dismissWindow(id: WindowID.monitor)
            OpenMonitor.request()
        case .monitor:
            OpenMonitor.request(activate: false)
        case .nothing, .wait:
            break
        }
    }

    /// One `OpenMonitor` request: first run while onboarding is required,
    /// the Monitor otherwise (and before the core has said, which the
    /// Monitor waits on, and over a refused daemon), which consumes
    /// `navigation.pending` once it can show it; a Settings destination
    /// opens the Monitor's Settings modal alone, at its section.
    @MainActor
    private func open(_ destination: MonitorDestination?) {
        navigation.leave(destination)
        let opening = LaunchRouting.opening(
            destination, startup: model.startup, requiresOnboarding: model.requiresOnboarding,
            onboardingKnown: model.onboardingKnown)
        switch opening.window {
        case .firstRun: openWindow(id: WindowID.firstRun)
        case .monitor: openWindow(id: WindowID.monitor)
        case nil: break
        }
        // Settings is the Monitor's modal (#1241 Task 10): a Settings
        // destination raises the Monitor and asks it for Settings at the
        // section.
        if let section = opening.settings {
            openWindow(id: WindowID.monitor)
            navigation.requestSettings(at: section)
        }
    }
}

enum WindowID {
    /// The glass monitor window (R5), the main window since R15.
    static let monitor = "trace-commons-monitor"
    /// The glass first-run pane (R12), the onboarding gate.
    static let firstRun = "trace-commons-first-run"
    /// The menu-bar item and popover in a window (R13), for review where
    /// the menu bar has no room for the item. Debug builds only.
    static let menuPreview = "trace-commons-menu-preview"
}
