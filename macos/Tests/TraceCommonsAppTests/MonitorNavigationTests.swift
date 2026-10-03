@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

@MainActor
final class MonitorNavigationTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The screenshot hook pins its own appearance and draws the glass
    /// screens: no legacy palette, none of the views the cutover deletes.
    func test_screenshotsForceTheAppearanceWithoutTheLegacyPalette() throws {
        let hook = try Self.text("DebugScreenshot.swift")
        XCTAssertTrue(hook.contains("\"TRACE_COMMONS_APPEARANCE\""))
        XCTAssertTrue(hook.contains(".environment(\\.colorScheme"))
        XCTAssertNil(hook.range(of: #"\bTC\."#, options: .regularExpression), "the legacy palette is read")
        for legacy in ["QueueContent(", "CreditRecordView(", "WithdrawalConfirmationCapture(", "MenuBarContent("] {
            XCTAssertFalse(hook.contains(legacy), "\(legacy) is a legacy view")
        }
    }

    /// The Monitor's three stores read the app's live client, re-attached
    /// whenever the daemon restarts; sample data is debug-only and opt-in.
    func test_theMonitorUsesTheLiveClient() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("DaemonDataWiring.sample(choice.set)"), "a store is built on sample data")
        for store in ["traces", "inference", "home"] {
            XCTAssertTrue(window.contains("\(store).attach(client)"), "\(store) is never attached to the live client")
        }
        XCTAssertTrue(window.contains(".task(id: model.liveData.map(ObjectIdentifier.init))"))
        // The function's body: from its signature to the `#endif` that
        // closes its debug-only branch.
        let sample = try XCTUnwrap(window.range(of: "static func sampleClient()"))
        let end = try XCTUnwrap(window.range(of: "#endif", range: sample.upperBound ..< window.endIndex))
        let body = window[sample.lowerBound ..< end.upperBound]
        XCTAssertFalse(body.dropFirst().contains("static func"), "the scan ran past sampleClient()")
        XCTAssertTrue(body.contains("#if DEBUG"), "sample data must be debug-only")
        XCTAssertTrue(body.contains("TRACE_COMMONS_SAMPLE"))
        XCTAssertTrue(body.contains("#else\n        return nil"), "a release build has no sample client")
    }

    // MARK: No client: the core is down, never empty and healthy

    func test_withNoClientTheTracesStoreIsCoreDown() async {
        let store = TracesStore(client: nil)
        await store.run()
        XCTAssertEqual(store.phase, .failed(.unreachable))
        XCTAssertNil(store.decisionsOwed)
    }

    func test_withNoClientTheHomeStoreIsCoreDown() async {
        let store = HomeStore(client: nil)
        await store.run()
        XCTAssertEqual(store.failures["status"], .unreachable)
        XCTAssertNil(store.status)
        XCTAssertEqual(HomeFormat.watchingState(store), .coreDown)
        XCTAssertEqual(MissionFormat.count(store.missions), "—")
    }

    func test_withNoClientTheInferenceStoreIsCoreDown() async {
        let store = InferenceStore(client: nil)
        await store.run()
        XCTAssertEqual(store.failures["inference_calls"], .unreachable)
        XCTAssertEqual(store.failures["harness_list"], .unreachable)
        XCTAssertNil(store.calls)
    }

    // MARK: Attaching a new client: loading again, nothing from the old one as current

    func test_attachResetsEachStoreToLoading() async {
        let traces = TracesStore(client: SampleDaemonClient(.normalDay))
        await traces.load()
        XCTAssertEqual(traces.phase, .loaded)
        XCTAssertNotNil(traces.decisionsOwed)
        XCTAssertFalse(traces.tree.allSessions.isEmpty)
        traces.attach(SampleDaemonClient(.busyQueue))
        XCTAssertEqual(traces.phase, .loading)
        XCTAssertNil(traces.decisionsOwed, "the old daemon's count is not drawn as the new one's")
        XCTAssertTrue(traces.tree.tools.isEmpty, "the old daemon's folders are not drawn or acted on")
        XCTAssertTrue(traces.tree.allSessions.isEmpty)

        let home = HomeStore(client: SampleDaemonClient(.coreDown))
        await home.load()
        XCTAssertNotNil(home.failures["status"])
        home.attach(SampleDaemonClient(.normalDay))
        XCTAssertTrue(home.failures.isEmpty)
        XCTAssertNil(home.status)
        XCTAssertEqual(HomeFormat.watchingState(home), .loading)

        let inference = InferenceStore(client: SampleDaemonClient(.normalDay))
        await inference.load()
        XCTAssertNotNil(inference.calls)
        inference.attach(nil)
        XCTAssertNil(inference.calls)
        XCTAssertTrue(inference.failures.isEmpty)
        await inference.run()
        XCTAssertEqual(inference.failures["inference_calls"], .unreachable)
    }

    // MARK: A read the old client answers after attach is dropped

    /// A daemon restart while Home is reading: the old client's answers
    /// arrive after the new one was attached, and must not be drawn as its.
    func test_theHomeStoreDropsAReadTheOldClientAnswersAfterAttach() async {
        // Five of Home's six reads reach the daemon; missions is provisional.
        let entered = expectation(description: "every read reached the old daemon")
        entered.expectedFulfillmentCount = 5
        let gate = GatedTransport(.normalDay, entered: entered)
        let store = HomeStore(client: LiveDaemonClient(transport: gate))
        let loading = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 10)
        store.attach(nil)
        gate.open()
        await loading.value
        XCTAssertNil(store.status, "the old daemon's status is drawn after a restart")
        XCTAssertNil(store.destinations)
        XCTAssertNil(store.history)
        XCTAssertNil(store.rollup)
        XCTAssertNil(store.credit)
        XCTAssertTrue(store.failures.isEmpty, "an old read's outcome is recorded against the new client")
    }

    func test_theInferenceStoreDropsAReadTheOldClientAnswersAfterAttach() async {
        // Four of its five reads reach the daemon (harnesses, calls,
        // destinations and the Private AI switch, a live `get_settings`
        // read); the summary is provisional.
        let entered = expectation(description: "every read reached the old daemon")
        entered.expectedFulfillmentCount = 4
        let gate = GatedTransport(.normalDay, entered: entered)
        let store = InferenceStore(client: LiveDaemonClient(transport: gate))
        let loading = Task { await store.load() }
        await fulfillment(of: [entered], timeout: 10)
        store.attach(nil)
        gate.open()
        await loading.value
        XCTAssertNil(store.calls, "the old daemon's calls are drawn after a restart")
        XCTAssertNil(store.harnesses)
        XCTAssertNil(store.destinations)
        XCTAssertNil(store.privateAI, "the old daemon's Private AI switch is drawn after a restart")
        XCTAssertTrue(store.failures.isEmpty, "an old read's outcome is recorded against the new client")
    }

    // MARK: The live client's provisional methods (notAvailableYet)

    /// The live client throws `notAvailableYet` for the provisional methods
    /// (Zaki's C3). The stores draw those as absent, never as a failure
    /// that hides the rest and never as zero or healthy.
    func test_theInferenceStoreDrawsAProvisionalSummaryAsAbsent() async {
        let store = InferenceStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertNil(store.summary)
        XCTAssertNil(store.failures["inference_summary"])
        XCTAssertNotNil(store.calls, "the calls stand without the summary")
    }

    func test_theHomeStoreDrawsAProvisionalCatalogueAsAbsent() async {
        let store = HomeStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertNil(store.missions)
        XCTAssertNil(store.failures["mission_catalogue"])
        XCTAssertEqual(MissionFormat.count(store.missions), "—", "an unread catalogue is a dash, never 0")
        XCTAssertNotNil(store.status)
    }

    func test_theTracesStoreLoadsOverTheLiveClient() async {
        let store = TracesStore(client: LiveDaemonClient(transport: SampleTransport(.normalDay)))
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
    }

    // MARK: Opening the Monitor from outside (Review Focus 4)

    func test_aRequestBeforeTheHandlerIsReplayedOnce() {
        OpenMonitor.reset()
        addTeardownBlock { @MainActor in OpenMonitor.reset() }
        var opened: [MonitorDestination?] = []
        OpenMonitor.request(.traces(entryId: nil))
        OpenMonitor.request(.settings(.compute))
        XCTAssertTrue(opened.isEmpty)
        OpenMonitor.handler = { opened.append($0) }
        XCTAssertEqual(opened.count, 1, "a held request replays exactly once")
        XCTAssertEqual(opened.first, .settings(.compute), "the last destination wins")
        OpenMonitor.request(nil)
        XCTAssertEqual(opened.count, 2)
        XCTAssertEqual(opened.last, .some(nil))
        // A handler installed again replays nothing: the hold was spent.
        OpenMonitor.handler = { opened.append($0) }
        XCTAssertEqual(opened.count, 2, "a held request replays once, not once per handler")
    }

    /// A request with no destination (a Dock click, an invite link) is held
    /// as a request, not dropped because it carries no destination.
    func test_aRequestWithNoDestinationIsHeldToo() {
        OpenMonitor.reset()
        addTeardownBlock { @MainActor in OpenMonitor.reset() }
        var opened: [MonitorDestination?] = []
        OpenMonitor.request()
        OpenMonitor.handler = { opened.append($0) }
        XCTAssertEqual(opened.count, 1)
        XCTAssertEqual(opened.first, .some(nil))
    }

    /// With no request before it, installing the handler opens nothing.
    func test_noRequestNoReplay() {
        OpenMonitor.reset()
        addTeardownBlock { @MainActor in OpenMonitor.reset() }
        var opened = 0
        OpenMonitor.handler = { _ in opened += 1 }
        XCTAssertEqual(opened, 0)
    }

    /// First run wins while onboarding is required, for every destination
    /// but Inference: Private AI sign-in is reachable before Commons
    /// enrollment, as the legacy window allowed (R-38). Home, Traces,
    /// Settings and a plain request still open first run.
    func test_firstRunWinsWhileOnboardingIsRequiredExceptForInference() {
        let table: [(MonitorDestination?, LaunchRouting.Window)] = [
            (nil, .firstRun),
            (.inference, .monitor),
            (.home(.overview), .firstRun),
            (.home(.history), .firstRun),
            (.home(.insights), .firstRun),
            (.traces(entryId: nil), .firstRun),
            (.traces(entryId: "entry-7"), .firstRun),
            (.settings(.compute), .firstRun),
        ]
        for (destination, window) in table {
            XCTAssertEqual(LaunchRouting.window(for: destination, requiresOnboarding: true), window,
                           String(describing: destination))
            XCTAssertEqual(LaunchRouting.opening(destination, requiresOnboarding: true).window, window,
                           String(describing: destination))
            XCTAssertEqual(LaunchRouting.window(for: destination, requiresOnboarding: false), .monitor,
                           String(describing: destination))
        }
    }

    /// While onboarding is required the Monitor shows the Inference tab
    /// and nothing else, whatever tab was restored.
    func test_onlyInferenceIsShownWhileOnboardingIsRequired() {
        XCTAssertEqual(MonitorWindowView.Tab.shown(requiresOnboarding: true), [.inference])
        XCTAssertEqual(MonitorWindowView.Tab.shown(requiresOnboarding: false), MonitorWindowView.Tab.allCases)
        for tab in MonitorWindowView.Tab.allCases {
            XCTAssertEqual(MonitorWindowView.shownTab(tab, requiresOnboarding: true), .inference)
            XCTAssertEqual(MonitorWindowView.shownTab(tab, requiresOnboarding: false), tab)
        }
    }

    /// A quit refusal lands on Compute whatever onboarding says: the
    /// window follows onboarding, the Settings section does not.
    func test_aQuitRefusalOpensComputeWhateverOnboardingSays() {
        for requires in [true, false] {
            let opening = LaunchRouting.opening(.settings(.compute), requiresOnboarding: requires)
            XCTAssertEqual(opening.settings, .compute, "requiresOnboarding \(requires)")
            XCTAssertEqual(opening.window, LaunchRouting.window(for: .settings(.compute), requiresOnboarding: requires))
        }
        for destination: MonitorDestination? in [nil, .inference, .traces(entryId: nil), .home(.history)] {
            XCTAssertNil(LaunchRouting.opening(destination, requiresOnboarding: false).settings,
                         "\(String(describing: destination)) opens Settings")
        }
    }

    /// The launcher opens Settings from the opening's section after (outside)
    /// the window switch, so no window choice can skip it.
    func test_theLauncherOpensSettingsOutsideTheWindowSwitch() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("""
                let opening = LaunchRouting.opening(destination, requiresOnboarding: model.requiresOnboarding)
                switch opening.window {
                case .firstRun: openWindow(id: WindowID.firstRun)
                case .monitor: openWindow(id: WindowID.monitor)
                }
                if let section = opening.settings {
                    navigation.settingsSection = section
                    openSettings()
                }
            }
        """), "Settings must open from the opening, outside the window switch")
    }

    /// Each destination lands on its tab; Inference reveals the inspector,
    /// where the Private AI switch and sign-in are; a session's id selects
    /// it and shows its review.
    func test_eachDestinationLandsOnItsTab() {
        var tab = MonitorWindowView.Tab.home
        var page = HomeTabView.Page.overview
        var session = ""
        var inspector = false
        func land(_ destination: MonitorDestination) {
            MonitorWindowView.land(destination, tab: &tab, homePage: &page,
                                   selectedSession: &session, showsInspector: &inspector)
        }

        land(.inference)
        XCTAssertEqual(tab, .inference)
        XCTAssertTrue(inspector, "the Private AI switch and sign-in live in the Inference inspector")

        inspector = false
        land(.traces(entryId: nil))
        XCTAssertEqual(tab, .traces)
        XCTAssertEqual(session, "", "no id selects nothing")
        XCTAssertFalse(inspector)

        land(.traces(entryId: "entry-7"))
        XCTAssertEqual(session, "entry-7")
        XCTAssertTrue(inspector)

        land(.home(.history))
        XCTAssertEqual(tab, .home)
        XCTAssertEqual(page, .history)

        // Settings is its own window: the Monitor's tabs stay as they were.
        land(.settings(.compute))
        XCTAssertEqual(tab, .home)
        XCTAssertEqual(page, .history)
    }

    func test_onlySettingsDestinationsNameASettingsSection() {
        XCTAssertEqual(MonitorDestination.settings(.compute).settingsSection, .compute)
        XCTAssertEqual(MonitorDestination.settings(.watchedFolders).settingsSection, .watchedFolders)
        for destination: MonitorDestination in [.inference, .traces(entryId: nil), .traces(entryId: "x"),
                                                .home(.overview), .home(.history)] {
            XCTAssertNil(destination.settingsSection, "\(destination) opens Settings")
        }
    }

    /// The glass strip and panel are the only menu-bar item; the AppKit
    /// menu, its label and the env flag that chose between them are gone,
    /// and the pause words are all `MenuBarView.swift` keeps (D-12).
    func test_theGlassMenuBarIsTheOnlyMenuBarItem() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertFalse(main.contains("TRACE_COMMONS_GLASS_MENU"))
        XCTAssertFalse(main.contains("MenuBarContent("))
        XCTAssertFalse(main.contains("MenuBarLabel("))
        // The one item takes no arguments: no `isInserted:` to swap it out.
        XCTAssertEqual(main.components(separatedBy: "MenuBarExtra {").count - 1, 1, "exactly one menu-bar item")
        XCTAssertFalse(main.contains("MenuBarExtra("))
        XCTAssertTrue(main.contains(".menuBarExtraStyle(.window)"))
        XCTAssertTrue(main.contains("MenuBarGlassPanel(store: menuPanel)"))
        XCTAssertTrue(main.contains("MenuBarStripLabel(model: model, store: menuPanel)"))
        // R15: a release build draws the same panel and strip; no branch
        // of its own stands in for them (ruling R-35's transitional
        // `#else` is gone).
        XCTAssertFalse(main.contains("Transitional (ruling R-35)"))
        XCTAssertFalse(main.contains("GlassMenuBarStrip(columns: [], condition: .unavailable, badge: nil)"))
        let menu = try Self.text("Views/MenuBarView.swift")
        XCTAssertFalse(menu.contains("struct MenuBarContent"))
        XCTAssertFalse(menu.contains("struct MenuBarLabel"))
        XCTAssertFalse(menu.contains("struct MenuBarGlyph"))
        XCTAssertFalse(menu.contains("enum Format"))
        XCTAssertTrue(menu.contains("enum MenuBarWords"))
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertFalse(panel.contains("MenuBarContent."))
        XCTAssertFalse(panel.contains("let navigation: MainWindowNavigation"), "the panel opens through OpenMonitor")
        XCTAssertTrue(panel.contains("PrivateInferenceTray.perform("))
        XCTAssertTrue(panel.contains("MenuBarWords.pauseUntil(index)"))
    }

    /// The tray may turn Private AI off and may not turn it on: off writes
    /// and opens the destination, on only opens it.
    func test_theTrayTurnsItOffAndOpensToTurnOn() {
        var turnedOff = 0, opened = 0
        PrivateInferenceTray.perform(on: false, turnOff: { turnedOff += 1 }, open: { opened += 1 })
        XCTAssertEqual(turnedOff, 0)
        XCTAssertEqual(opened, 1)
        PrivateInferenceTray.perform(on: true, turnOff: { turnedOff += 1 }, open: { opened += 1 })
        XCTAssertEqual(turnedOff, 1)
        XCTAssertEqual(opened, 2)
    }

    /// Every outside opener goes through OpenMonitor; nothing names the
    /// deleted main window's opener. (`LegacyShellRetiredTests` pins that
    /// `WindowID.main` left with the legacy window.)
    func test_everyOpenerUsesOpenMonitor() throws {
        let delegate = try Self.text("AppDelegate.swift")
        XCTAssertTrue(delegate.contains("OpenMonitor.request(.settings(.compute))"), "quit refusal must land on Compute")
        XCTAssertTrue(delegate.contains("if !hasVisibleWindows { OpenMonitor.request() }"), "Dock reopen")
        XCTAssertTrue(delegate.contains("NSApp.activate(ignoringOtherApps: true)\n            OpenMonitor.request()\n"),
                      "an invite link opens the Monitor")
        XCTAssertTrue(delegate.contains("D-14"), "the invite-link comment must say a deep link only opens the Monitor")
        XCTAssertFalse(delegate.contains("OpenMainWindow"))
        XCTAssertFalse(delegate.contains("navigation?.section"))
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("Notifier.shared.onReview = { OpenMonitor.request(.traces(entryId: nil)) }"))
        XCTAssertTrue(main.contains("TRACE_COMMONS_SHOW_WINDOW"))
        XCTAssertTrue(main.contains("OpenMonitor.handler = { destination in"))
        XCTAssertTrue(main.contains("LaunchRouting.opening(destination, requiresOnboarding: model.requiresOnboarding)"))
        XCTAssertTrue(main.contains("MonitorWindowView(navigation: navigation, insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts)"))
        XCTAssertFalse(main.contains("OpenMainWindow"))
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertFalse(panel.contains("MainWindowView.Section"))
        XCTAssertFalse(panel.contains("openMain("))
        XCTAssertTrue(panel.contains("OpenMonitor.request(destination)"))
        for needle in ["open(.inference)", "open(.traces(entryId: nil))", "open(.home(.history))",
                       "open(MenuPanelData.manageRules(requiresOnboarding: model.requiresOnboarding))"] {
            XCTAssertTrue(panel.contains(needle), "the menu panel never opens \(needle)")
        }
        let pointer = try Self.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(pointer.contains("Button(copy.destination) {\n                            OpenMonitor.request(.inference)\n"))
        // The Monitor consumes the destination on an always-present
        // container, initially too, and lands Inference on its inspector.
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(".glassWindow()\n        .onChange(of: navigation.pending, initial: true) {"))
        // From `land`, not the file's first `case .inference:` (the tab's
        // title switch comes first).
        let land = try XCTUnwrap(window.range(of: "static func land("))
        let inference = try XCTUnwrap(window.range(of: "case .inference:", range: land.upperBound ..< window.endIndex))
        XCTAssertTrue(window[inference.upperBound...].prefix(200).contains("showsInspector = true"),
                      "the Private AI switch and sign-in live in the Inference inspector")
        XCTAssertTrue(window.contains(".onChange(of: navigation.settingsSection, initial: true) {"),
                      "Settings already open must still move to the asked-for section")
    }

    /// While onboarding is required the Monitor draws the notices, the
    /// signed-out notice (whose button routes to first run) and a tab strip
    /// of Inference alone (R-38): Home and Traces act on consent that has
    /// not been given. Every tab switch and the inspector draw the shown
    /// tab, so a restored Home or Traces never reaches the screen; the
    /// inspector admits the Private AI one alone. Before the core has said
    /// whether onboarding is required, the placeholder status is not read
    /// as "signed out": the pane waits and the inspector draws nothing.
    func test_onlyInferenceIsDrawnWhileOnboardingIsRequired() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("MonitorWords.signedOut"))
        XCTAssertTrue(window.contains("""
                        if !LaunchRouting.onboardingKnown(startup: model.startup, statusAnswered: model.status.answered, statusFailed: model.statusReadFailed) {
                            SettingsAwaiting()
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                        } else {
                            if model.requiresOnboarding {
                                GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                                    Button(OnboardingWelcomeWords.getStarted) { OpenMonitor.request() }
                                }
                            }
                            GlassSegmentedTabs(
                                String(localized: "Monitor", comment: "Monitor tabs name"),
                                selection: Binding(
                                    get: { MonitorWindowView.shownTab(tab, requiresOnboarding: model.requiresOnboarding) },
                                    set: { tab = $0 }),
                                segments: MonitorWindowView.Tab.shown(requiresOnboarding: model.requiresOnboarding).map { item in
        """), "the strip must show the onboarding-filtered tabs, under the notices")
        let pane = try XCTUnwrap(window.range(of: "private struct MonitorMainPane"))
        let map = try XCTUnwrap(window.range(of: "private struct MonitorMapPane"))
        XCTAssertEqual(window[pane.lowerBound ..< map.lowerBound].components(separatedBy: "GlassSegmentedTabs(").count - 1, 1,
                       "a second tab strip in the main pane would escape the gate")
        // Both tab switches (the main pane's content and the inspector)
        // switch on the shown tab, never the restored one.
        XCTAssertEqual(window.components(separatedBy: "switch Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) {").count - 1, 2)
        XCTAssertFalse(window.contains("switch tab {"), "a switch on the restored tab would draw Home or Traces during onboarding")
        XCTAssertTrue(window.contains("""
                    GlassPane {
                        // An empty branch would leave the pane nothing to draw, and
                        // it would vanish while the layout still reserved its width.
                        // While onboarding is required only Inference is shown, so
                        // the Private AI inspector is the only one admitted (R-38).
                        if !LaunchRouting.onboardingKnown(startup: model.startup, statusAnswered: model.status.answered, statusFailed: model.statusReadFailed) {
                            Color.clear
                        } else {
                            switch Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) {
        """), "the inspector must draw nothing until onboarding is known, and the shown tab's after")
    }

    /// Finishing first run closes it and opens the Monitor on Home.
    func test_finishingFirstRunOpensTheMonitor() throws {
        let firstRun = try Self.text("Views/Monitor/FirstRunViews.swift")
        XCTAssertTrue(firstRun.contains("dismissWindow(id: WindowID.firstRun)"))
        XCTAssertTrue(firstRun.contains("OpenMonitor.request(.home(.overview))"))
        XCTAssertTrue(firstRun.contains("""
                .onChange(of: model.requiresOnboarding, initial: true) { _, requires in
                    guard !requires else { return }
                    dismissWindow(id: WindowID.firstRun)
                    OpenMonitor.request(.home(.overview))
                }
        """), "the hand-off must follow requiresOnboarding turning false, on the always-present container")
    }

    /// A tall step (Uses with many scopes) scrolls inside the first-run
    /// window: the pane takes the window's height rather than its step's,
    /// so the step's own ScrollView has a bound to scroll within.
    func test_aTallFirstRunStepScrollsInsideTheWindow() throws {
        let firstRun = try Self.text("Views/Monitor/FirstRunViews.swift")
        XCTAssertFalse(firstRun.contains("fixedSize("), "a vertical fixedSize grows the pane past the window")
        XCTAssertTrue(firstRun.contains("""
                        .frame(width: FirstRunProgress.paneWidth)
                        .padding(.vertical, GlassTokens.Space.windowPadding * 3)
        """), "the pane is bounded by the window, less the scene's margin")
    }

    /// Whether onboarding is required is known only once the core has said
    /// enough: never from the placeholder status of a running daemon. A
    /// daemon that needs its folders, or refused, has no status to wait for.
    func test_onboardingIsKnownOnlyOnceTheCoreAnswers() {
        XCTAssertFalse(LaunchRouting.onboardingKnown(startup: .starting, statusAnswered: false, statusFailed: false))
        XCTAssertFalse(LaunchRouting.onboardingKnown(startup: .starting, statusAnswered: true, statusFailed: false))
        XCTAssertFalse(LaunchRouting.onboardingKnown(startup: .running, statusAnswered: false, statusFailed: false))
        XCTAssertTrue(LaunchRouting.onboardingKnown(startup: .running, statusAnswered: true, statusFailed: false))
        XCTAssertTrue(LaunchRouting.onboardingKnown(startup: .needsRoots, statusAnswered: false, statusFailed: false))
        XCTAssertTrue(LaunchRouting.onboardingKnown(startup: .refused("no"), statusAnswered: false, statusFailed: false))
    }

    /// A first status read that fails is an answer for the launch: the
    /// unanswered status requires onboarding, so first run opens (fail
    /// closed) rather than nothing at all.
    func test_aFailedStatusReadOpensFirstRun() {
        XCTAssertTrue(LaunchRouting.onboardingKnown(startup: .running, statusAnswered: false, statusFailed: true))
        XCTAssertFalse(LaunchRouting.onboardingKnown(startup: .starting, statusAnswered: false, statusFailed: true),
                       "a daemon still starting has not been asked")
        let model = AppModel()
        XCTAssertFalse(model.statusReadFailed)
        XCTAssertTrue(model.requiresOnboarding, "the placeholder status requires onboarding")
        XCTAssertEqual(LaunchRouting.opening(nil, requiresOnboarding: model.requiresOnboarding).window, .firstRun)
    }

    /// The failure is recorded from the `status` read itself, and cleared
    /// by its next answer.
    func test_theStatusReadRecordsItsFailure() throws {
        let model = try Self.text("AppModel.swift")
        XCTAssertTrue(model.contains(#"""
                perform("status", work: { try $0.status() }, onFailure: { self.publishIfChanged(\.statusReadFailed, true) }) {
                    self.publishIfChanged(\.status, $0)
                    self.publishIfChanged(\.statusReadFailed, false)
                }
        """#))
    }

    /// No consent surface before onboarding: the Monitor's Settings button
    /// is disabled while onboarding is required.
    func test_theSettingsButtonWaitsForOnboarding() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("""
                            GlassRoundButton(String(localized: "Settings", comment: "Settings button"), systemImage: "gearshape", small: true, action: onSettings)
                                .disabled(model.requiresOnboarding)
        """))
    }

    /// The placeholder status is not an answer; the first reply is. (The
    /// input `onboardingKnown` is given by the launch and the Monitor.)
    func test_statusIsAnsweredOnlyByAReply() {
        let model = AppModel()
        XCTAssertFalse(model.status.answered)
        model.setStatusForTesting(DaemonStatus(
            schemaVersion: "1.1", loggedIn: false, tenantID: nil, consentScopes: [], paused: false,
            queueDepth: 0, nextDigestAt: nil, health: DaemonHealth(lastErrorLabel: nil, since: nil)))
        XCTAssertTrue(model.status.answered)
    }

    /// The launch request is made once, from the always-present label, and
    /// never over a destination an earlier opener (an invite link, a
    /// notification) already asked for.
    func test_theLaunchOpensOnceWithoutOverridingAnEarlierRequest() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("""
                MenuBarStripLabel(model: model, store: menuPanel)
                    .task { launch() }
                    .onChange(of: LaunchRouting.onboardingKnown(startup: model.startup, statusAnswered: model.status.answered, statusFailed: model.statusReadFailed),
                              initial: true) { _, ready in
                        openAtLaunch(ready)
                    }
        """), "the launch request must hang off the always-present label")
        XCTAssertTrue(main.contains("""
                guard ready, !openedAtLaunch else { return }
                openedAtLaunch = true
                guard navigation.pending == nil else { return }
                OpenMonitor.request()
        """), "the launch must open once, and never over an earlier destination")
    }

    /// D-11: services no longer wait for the main window to leave Insights;
    /// the first request starts them, once.
    func test_servicesStartOnTheFirstRequestWhateverTheSection() {
        let navigation = MainWindowNavigation()
        var starts = 0
        navigation.activateServicesIfNeeded { starts += 1 }
        navigation.activateServicesIfNeeded { starts += 1 }
        navigation.activateServicesForWindow()
        XCTAssertEqual(starts, 1)
        XCTAssertNil(navigation.pending)
        XCTAssertNil(navigation.settingsSection)
    }
}

/// Answers `tc_call` from a sample set's daemon-shaped replies, as the
/// daemon would, so a store reads through the real `LiveDaemonClient`.
private final class SampleTransport: DaemonTransport, @unchecked Sendable {
    let set: SampleDaemonClient.SampleSet

    init(_ set: SampleDaemonClient.SampleSet) {
        self.set = set
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        SampleDaemonData.reply(method, in: set).map { #"{"id":0,"result":\#($0)}"# }
            ?? #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}

/// A `SampleTransport` whose every call blocks (on the live client's work
/// queue, not the main actor) until `open()`, so a test can attach a new
/// client while the old one's reads are still outstanding.
private final class GatedTransport: DaemonTransport, @unchecked Sendable {
    private let inner: SampleTransport
    private let entered: XCTestExpectation
    private let gate = DispatchSemaphore(value: 0)

    init(_ set: SampleDaemonClient.SampleSet, entered: XCTestExpectation) {
        inner = SampleTransport(set)
        self.entered = entered
    }

    /// Lets every waiting call, and every later one, through.
    func open() {
        gate.signal()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        entered.fulfill()
        gate.wait()
        // Pass the opening on to the next waiting call.
        gate.signal()
        return inner.call(method, params: paramsJSON)
    }
}
