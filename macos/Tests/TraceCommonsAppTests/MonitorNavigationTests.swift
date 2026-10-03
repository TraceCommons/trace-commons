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

    func test_firstRunWinsWhileOnboardingIsRequired() {
        XCTAssertEqual(LaunchRouting.window(requiresOnboarding: true), .firstRun)
        XCTAssertEqual(LaunchRouting.window(requiresOnboarding: false), .monitor)
    }

    /// A quit refusal lands on Compute whatever onboarding says: the
    /// window follows onboarding, the Settings section does not.
    func test_aQuitRefusalOpensComputeWhateverOnboardingSays() {
        for requires in [true, false] {
            let opening = LaunchRouting.opening(.settings(.compute), requiresOnboarding: requires)
            XCTAssertEqual(opening.settings, .compute, "requiresOnboarding \(requires)")
            XCTAssertEqual(opening.window, LaunchRouting.window(requiresOnboarding: requires))
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
                #else
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

    /// Until R15 (T11) a release build has only the legacy window, which
    /// is opened at the matching legacy section; a quit refusal still lands
    /// on Compute there. T11 deletes this with `section` (ruling R-33).
    func test_aReleaseBuildOpensTheLegacyWindowAtTheMatchingSection() {
        XCTAssertEqual(MainWindowNavigation.legacySection(for: .settings(.compute)), .compute)
        XCTAssertEqual(MainWindowNavigation.legacySection(for: .settings(.watchedFolders)), .settings)
        XCTAssertEqual(MainWindowNavigation.legacySection(for: .traces(entryId: nil)), .queue)
        XCTAssertEqual(MainWindowNavigation.legacySection(for: .home(.history)), .history)
        XCTAssertEqual(MainWindowNavigation.legacySection(for: .inference), .privateInference)
        XCTAssertNil(MainWindowNavigation.legacySection(for: .home(.overview)), "no legacy overview: stays where it was")
        XCTAssertNil(MainWindowNavigation.legacySection(for: nil))
    }

    /// Every outside opener goes through OpenMonitor; nothing names the
    /// deleted main window's opener. (`WindowID.main` itself leaves
    /// TraceCommonsAppMain.swift with the legacy window in T11, ruling R-33.)
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
        XCTAssertTrue(main.contains("MonitorWindowView(navigation: navigation)"))
        XCTAssertFalse(main.contains("OpenMainWindow"))
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertFalse(panel.contains("MainWindowView.Section"))
        XCTAssertFalse(panel.contains("openMain("))
        XCTAssertTrue(panel.contains("OpenMonitor.request(destination)"))
        for needle in ["open(.inference)", "open(.traces(entryId: nil))", "open(.home(.history))",
                       "open(.settings(.watchedFolders))"] {
            XCTAssertTrue(panel.contains(needle), "the menu panel never opens \(needle)")
        }
        let menu = try Self.text("Views/MenuBarView.swift")
        XCTAssertFalse(menu.contains("openWindow(id: WindowID.main)"), "the shipping menu opens the Monitor by destination")
        XCTAssertTrue(menu.contains("OpenMonitor.request(.inference)"))
        XCTAssertTrue(menu.contains("OpenMonitor.request(.traces(entryId: nil))"))
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
