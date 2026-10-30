import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R13 of #1173: the menu-bar popover from the handoff, under its rules.
@MainActor
final class MenuBarGlassPanelTests: XCTestCase {
    /// C19: the panel's corner is #1146's popover radius
    /// (`--tc-radius-card`, 14), not a larger one of its own.
    func test_thePanelHasThePopoverRadius() throws {
        XCTAssertEqual(GlassTokens.Radius.menuPanel, GlassTokens.Radius.card)
        XCTAssertEqual(GlassTokens.Radius.menuPanel, 14)
        let panel = try GlassSurfaceRulesTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("radius: GlassTokens.Radius.menuPanel"))
    }

    /// The popover is a quarter narrower than the handoff's 380 (owner,
    /// 2026-10-21). The wordmark in its top row stays past the brand kit's
    /// 72px floor for the wordmark.
    func test_thePanelIsAQuarterNarrower() throws {
        XCTAssertEqual(MenuBarGlassPanel.width, 380 * 0.75)
        let ratio = GlassNearWordmark.viewBox.width / GlassNearWordmark.viewBox.height
        XCTAssertGreaterThanOrEqual(MenuBarGlassPanel.wordmarkHeight * ratio, 72)
    }

    // MARK: Mode roll-up

    /// One mode for every folder reads as that mode; any difference is
    /// Mixed; no folders is unknown.
    func test_theModePillRollsUpTheFolders() {
        XCTAssertEqual(MenuPanelData.rollup("notify_only"), .ask)
        XCTAssertEqual(MenuPanelData.rollup("auto_upload"), .armed)
        XCTAssertEqual(MenuPanelData.rollup("ignore"), .never)
        XCTAssertEqual(MenuPanelData.rollup("mixed"), .mixed)
        XCTAssertEqual(MenuPanelData.rollup("a-mode-from-a-later-daemon"), .none)
        XCTAssertEqual(MenuPanelData.rollup(nil), .none)
    }

    /// The pill reads the daemon's roll-up from `status` (#1208), in the
    /// core's words, with the partial line only when the status says so.
    func test_thePillReadsTheCoresContributionMode() async throws {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let status = try XCTUnwrap(store.status)
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        let rollup = MenuPanelData.rollup(status.contributionMode)
        XCTAssertNotEqual(rollup, .none, "the recorded status carries contribution_mode")
        XCTAssertEqual(MenuPanelData.modeValue(rollup, mode: status.contributionMode, copy: copy),
                       rollup == .mixed ? copy.mixed : copy.choice(for: status.contributionMode)?.label)
        XCTAssertEqual(MenuPanelData.modeValue(.none, mode: nil, copy: copy), "—")
        XCTAssertEqual(MenuPanelData.modeValue(.ask, mode: "notify_only", copy: nil), "—")
        XCTAssertEqual(MenuPanelData.partialLine("auto_upload", status: status, copy: copy),
                       status.contributionModePartial == true ? copy.autoPartial : nil)
        XCTAssertNil(MenuPanelData.partialLine("notify_only", status: status, copy: copy))
        // The panel never works the roll-up out from the folders.
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertFalse(source.contains("projects.map(\\.mode)"))
        // The pill itself is gone from the popover (owner, 2026-10-19).
        XCTAssertFalse(source.contains("store.status?.contributionMode"))
    }

    /// The list agrees with the pill: with no override it checks the
    /// core's roll-up (Mixed only when the folders differ, otherwise that
    /// mode's row), and while one is in force it checks the override's own
    /// mode alone. Nothing is checked before the status is read or while
    /// the core is down, even with a status still in hand.
    func test_theListChecksTheRollupUnlessAnOverrideIsInForce() async throws {
        let client = SampleDaemonClient(.normalDay)
        let modes: [String?] = [nil, "notify_only", "auto_upload", "ignore"]
        func checked(_ status: DaemonData.Status?, stale: Bool = false) -> [String?] {
            modes.filter { MenuPanelData.listChecks($0, status: status, stale: stale) }
        }
        XCTAssertEqual(checked(nil), [])
        let before = try await client.status()
        XCTAssertEqual(before.contributionMode, "notify_only")
        XCTAssertEqual(checked(before), ["notify_only"], "every folder on Ask checks Ask, not Mixed")
        XCTAssertEqual(checked(try Self.status(before, contributionMode: "mixed")), [nil])
        XCTAssertEqual(checked(try Self.status(before, contributionMode: "auto_upload")), ["auto_upload"])
        XCTAssertEqual(checked(before, stale: true), [])
        _ = try await client.setContributionOverride(mode: .ignore, confirm: false)
        let overridden = try await client.status()
        XCTAssertEqual(checked(overridden), ["ignore"])
        XCTAssertEqual(checked(overridden, stale: true), [], "a core-down panel shows no override as known")
        _ = try await client.clearContributionOverride()
        let cleared = try await client.status()
        XCTAssertEqual(checked(cleared), ["notify_only"])
    }

    /// `status` with the core's roll-up replaced, through its own coding.
    private static func status(_ status: DaemonData.Status, contributionMode: String) throws -> DaemonData.Status {
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(status)) as? [String: Any])
        let key = try XCTUnwrap(object.first { $0.value as? String == status.contributionMode && $0.key.lowercased().hasPrefix("contribution") }?.key)
        object[key] = contributionMode
        let patched = try JSONDecoder().decode(DaemonData.Status.self, from: JSONSerialization.data(withJSONObject: object))
        XCTAssertEqual(patched.contributionMode, contributionMode)
        return patched
    }

    // MARK: Graph

    /// The popover's bars are the Traces graph's (owner, 2026-10-22): its
    /// days and counts, every other weekday left out counting back from
    /// today's, so the labels fit the popover's narrower columns.
    func test_theGraphBarsAreTheAppsWithEveryOtherLabel() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let day: TimeInterval = 86_400
        let buckets = TracesGraphModel.buckets(
            shared: [now, now - 60, now - day, now - 40 * day], kept: [now - 2 * day],
            offset: 0, now: now, range: .days, calendar: calendar)
        let bars = MenuPanelData.bars(buckets, words: MonitorShellWords.graphWords)
        XCTAssertEqual(bars.count, TracesGraphModel.days)
        XCTAssertEqual(bars.map(\.id), buckets.map { TracesGraphFooter.bar($0, words: nil).id })
        XCTAssertEqual(bars.last?.up, 2)
        XCTAssertEqual(bars[bars.count - 2].up, 1)
        XCTAssertEqual(bars[bars.count - 3].down, 1)
        XCTAssertEqual(bars.reduce(0) { $0 + $1.up }, 3, "a day outside the window is not drawn")
        XCTAssertEqual(bars.last?.label, buckets.last?.label, "today is labelled")
        XCTAssertFalse(bars.last?.label.isEmpty ?? true)
        XCTAssertEqual(bars[bars.count - 2].label, "")
        XCTAssertEqual(bars[bars.count - 3].label, buckets[buckets.count - 3].label)
        XCTAssertEqual(bars.filter { !$0.label.isEmpty }.count, (TracesGraphModel.days + 1) / 2)
    }

    // MARK: Menu-bar item

    /// The item's trace bars count per four-hour step over the last 24
    /// hours: a contribution or kept session lands in its step, one stamped
    /// after the span's end in the last, one before the span nowhere.
    func test_theItemCountsTracesPerFourHourStep() {
        let end = Date(timeIntervalSince1970: 1_790_000_000)
        let hour: TimeInterval = 3_600
        let steps = MenuPanelData.steps(
            shared: [end - 60, end - 5 * hour, end + 30, end - 25 * hour],
            kept: [end - 21 * hour, end - 24 * hour], span: MenuPanelData.Span(end: end))
        XCTAssertEqual(steps.count, 6)
        XCTAssertEqual(steps[5].up, 2, "the newest step, and one stamped after the end")
        XCTAssertEqual(steps[4].up, 1)
        XCTAssertEqual(steps.reduce(0) { $0 + $1.up }, 3, "a contribution before the span is not drawn")
        XCTAssertEqual(steps[0].down, 2, "the oldest step starts 24 hours before the end")
    }

    /// The Private AI steps are verified (up) over all calls (down), their
    /// span ends one step after the last step's start, and a ledger that
    /// did not answer is unknown, never six zeros.
    func test_thePrivateAIStepsAreVerifiedOverCalls() throws {
        let decoder = DaemonDataDecoding.decoder()
        let activity = try decoder.decode(DaemonData.InferenceActivity.self, from: Data(#"""
            {"readable":true,"window_hours":24,"step_hours":2,"steps":[
             {"start":"2026-10-21T08:00:00+00:00","calls":5,"verified":3},
             {"start":"2026-10-21T10:00:00+00:00","calls":2,"verified":2}]}
            """#.utf8))
        let steps = try XCTUnwrap(MenuPanelData.privateAISteps(activity))
        XCTAssertEqual(steps.map(\.up), [3, 2])
        XCTAssertEqual(steps.map(\.down), [5, 2])
        let span = try XCTUnwrap(MenuPanelData.span(activity))
        XCTAssertEqual(span.count, 2)
        XCTAssertEqual(span.stepHours, 2)
        XCTAssertEqual(span.end, DaemonTimestamp.parse("2026-10-21T12:00:00Z"))

        let unread = try decoder.decode(DaemonData.InferenceActivity.self, from: Data(
            #"{"readable":false,"window_hours":24,"step_hours":2,"steps":[]}"#.utf8))
        XCTAssertNil(MenuPanelData.privateAISteps(unread))
        XCTAssertNil(MenuPanelData.span(unread))

        // A step that is no length of time gives no clock to count on.
        let zero = try decoder.decode(DaemonData.InferenceActivity.self, from: Data(#"""
            {"readable":true,"window_hours":24,"step_hours":0,"steps":[
             {"start":"2026-10-21T10:00:00+00:00","calls":1,"verified":1}]}
            """#.utf8))
        XCTAssertNil(MenuPanelData.span(zero))
        XCTAssertNil(
            MenuPanelData.privateAISteps(zero),
            "steps with no clock are not drawn, or they would pair with trace steps on another clock")
        XCTAssertEqual(
            MenuPanelData.steps(shared: [Date()], kept: [], span: MenuPanelData.Span(end: Date(), stepHours: 0)), [])
    }

    // MARK: Recent activity

    func test_theMenuHasNoSecondLookCategoryOrAggregateCount() throws {
        let panel = try GlassSurfaceRulesTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        let store = try GlassSurfaceRulesTests.text("Views/Monitor/MenuPanelStore.swift")
        XCTAssertFalse(panel.contains("flaggedRow"))
        XCTAssertFalse(panel.contains("MenuWords.flagged"))
        XCTAssertFalse(store.contains("flaggedCount"))
        XCTAssertFalse(store.contains("static func flagged("))
        XCTAssertTrue(store.contains("client.listPending(projectId: nil)"), "Kept still reads waiting traces")
    }

    /// Recent activity is newest first, at most three, and an outside call
    /// carries its proof label unless it was verified.
    func test_recentActivityIsNewestFirst() async throws {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let rows = MenuPanelData.recent(
            pending: store.pending, history: store.history, calls: store.calls, statusLabel: { _ in nil })
        XCTAssertLessThanOrEqual(rows.count, 3)
        XCTAssertEqual(rows.map(\.at), rows.map(\.at).sorted(by: >))
    }

    /// With no word for a status the row names the project only, never the
    /// raw wire token.
    func test_recentActivityNeverShowsTheRawStatusToken() async throws {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let statuses = Set(store.history.compactMap(\.status))
        let rows = MenuPanelData.recent(
            pending: [], history: store.history, calls: [], statusLabel: { _ in nil }, limit: 50)
        XCTAssertFalse(rows.isEmpty)
        for row in rows {
            for status in statuses { XCTAssertFalse(row.text.contains(status), row.text) }
        }
    }

    /// A row with no status is still recent activity, read with the shared
    /// status table's unknown word, as the History list reads it.
    func test_aRowWithNoStatusReadsStatusUnavailable() throws {
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
        let row = try DaemonDataDecoding.decoder().decode(DaemonData.HistoryRow.self, from: Data(
            #"{"submission_id":"a","submitted_at":"2026-09-30T09:00:00Z","project_label":"repo","status":null}"#.utf8))
        let rows = MenuPanelData.recent(
            pending: [], history: [row], calls: [],
            statusLabel: { HomeFormat.historyStatusLabel(copy: copy, $0) })
        XCTAssertEqual(rows.map(\.text), ["repo · \(copy.contributionStatusUnavailable)"])
    }

    /// An outside call carries its proof label unless it was verified; a
    /// routed call is not recent activity.
    func test_anOutsideCallCarriesItsProofLabel() throws {
        let decode = { (json: String) in
            try DaemonDataDecoding.decoder().decode(DaemonData.InferenceCall.self, from: Data(json.utf8))
        }
        let unproven = try decode(#"{"id":1,"at":"2026-09-30T09:00:00Z","tool":"codex","family":"openai","model":"m","route":"outside","cost":null,"proof":"unattested"}"#)
        let verified = try decode(#"{"id":2,"at":"2026-09-30T08:00:00Z","tool":"codex","family":"openai","model":"m","route":"outside","cost":null,"proof":"verified"}"#)
        let routed = try decode(#"{"id":3,"at":"2026-09-30T10:00:00Z","tool":"codex","family":"openai","model":"m","route":"routed","cost":null,"proof":"unattested"}"#)
        let rows = MenuPanelData.recent(pending: [], history: [], calls: [unproven, verified, routed], statusLabel: { _ in nil })
        XCTAssertEqual(rows.map(\.id), ["call:1", "call:2"])
        XCTAssertEqual(rows.first?.trailing, InferenceWords.proof(.unattested))
        XCTAssertNil(rows.last?.trailing)
    }

    // MARK: Live data and the strip

    /// With no client, after a failed read, or once the event stream ends,
    /// the store's data is stale; a full read clears it.
    func test_theStoreMarksItsDataStale() async {
        let none = MenuPanelStore(client: nil)
        await none.load()
        XCTAssertTrue(none.stale)

        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        XCTAssertTrue(store.stale, "nothing read yet")
        await store.load()
        XCTAssertFalse(store.stale)
        store.attach(SampleDaemonClient(.coreDown))
        XCTAssertTrue(store.stale)
        await store.load()
        XCTAssertTrue(store.stale, "a failed read is stale, never the last values as current")
    }

    /// The strip is never live unless the daemon runs, the data is current
    /// and the core is healthy with a known count.
    func test_theStripIsDownWhenTheCoreIs() {
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: false, paused: false, available: false, stale: false), .unavailable)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: false, paused: false, available: true, stale: true), .unavailable)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 2, unhealthy: true, paused: false, available: true, stale: false), .attention)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: nil, unhealthy: false, paused: false, available: true, stale: false), .attention)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 0, unhealthy: false, paused: true, available: true, stale: false), .paused)
        XCTAssertEqual(MenuPanelStatus.condition(decisionsOwed: 0, unhealthy: false, paused: false, available: true, stale: false), .live)
    }

    /// The popover's store is the app's live client, never sample data,
    /// outside tests and previews.
    func test_thePanelUsesTheLiveClient() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
        let main = try String(contentsOf: root.appendingPathComponent("TraceCommonsAppMain.swift"), encoding: .utf8)
        XCTAssertTrue(main.contains("MenuPanelStore(client: nil)"))
        XCTAssertFalse(main.contains("MenuPanelStore(client: MonitorWindowView.dataClient())"))
        let panel = try String(contentsOf: root.appendingPathComponent("Views/Monitor/MenuBarGlassPanel.swift"), encoding: .utf8)
        XCTAssertTrue(panel.contains("store.attach(model.daemonData, configDirectory: model.configDirectory)"))
    }

    // MARK: Private AI pill

    /// The pill reads what the listener reports, never the switch alone: a
    /// switch left on over a listener that refused to start, crashed or is
    /// stopping is never drawn On. Unknown is unknown, never Off.
    func test_thePrivateAIPillFollowsTheListenerNotTheSwitch() throws {
        let calls = PrivateInferenceMenuBarTests.coreCalls
        let running = PrivateInferenceSurface.menuState(
            PrivateInferenceState(label: "running", port: 8080), calls: calls)
        XCTAssertEqual(MenuPanelStatus.privateAI(on: true, menu: running), .listener(word: "On", tone: .clear))
        for label in ["port_in_use", "start_failed", "crashed", "stopping", "unknown_state", ""] {
            let menu = PrivateInferenceSurface.menuState(PrivateInferenceState(label: label, port: nil), calls: calls)
            XCTAssertNotEqual(MenuPanelStatus.privateAIDot(MenuPanelStatus.privateAI(on: true, menu: menu)), .on,
                              "a switch on over \(label) must not read green")
        }
        XCTAssertEqual(MenuPanelStatus.privateAI(on: false, menu: running), .off)
        XCTAssertEqual(MenuPanelStatus.privateAI(on: nil, menu: running), .unknown)
        // The pill's fill and value come from that, not from the switch.
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // The popover draws no Private AI pill any more (owner, 2026-10-19):
        // the switch is the Private AI tab's.
        XCTAssertFalse(source.contains("privateAIPill"))
        XCTAssertFalse(source.contains("privateAIOn == true ? GlassTokens.Color.dataShared"))
    }

    // MARK: Badge

    func test_theBadgeCountsDecisionsOwedOnly() {
        XCTAssertEqual(MenuPanelStatus.badge(9), 9)
        XCTAssertNil(MenuPanelStatus.badge(0))
        XCTAssertNil(MenuPanelStatus.badge(nil))
    }

    // MARK: Rules

    /// Nothing is sent from the popover, and nothing is turned on from it:
    /// its only writes are pause and resume (owner, 2026-10-19: the mode
    /// and Private AI pills, and with them the override and Private AI
    /// off, are gone).
    func test_thePopoverMakesOnlyTheShippingMenusWrites() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // No credit figure of any kind: no projected or pending credit.
        for forbidden in ["setProjectMode", "approve(", "applyPrivateInference(true)", "setPrivateAI(",
                          "setContributionOverride(", "clearContributionOverride(", ".disabled(true)",
                          "setInsightsLedgerFeed(",
                          "creditPoints", "creditPending", "creditFinal", "creditRange", "commonsCreditSummary"] {
            XCTAssertFalse(source.contains(forbidden), "the popover contains \(forbidden)")
        }
        for gone in ["modeOptions", "store.choose(", "store.resolveConfirmation(", "applyPrivateInference(",
                     "PrivateInferenceTray.perform("] {
            XCTAssertFalse(source.contains(gone), "the popover writes through \(gone) again")
        }
        XCTAssertTrue(source.contains("model.pause(until: MenuBarWords.pauseUntil(index))"))
        XCTAssertTrue(source.contains("model.resume()"))
    }

    // MARK: Contribution override (#1208)

    private func loadedStore(_ set: SampleDaemonClient.SampleSet) async -> (MenuPanelStore, SampleDaemonClient) {
        let client = SampleDaemonClient(set)
        let store = MenuPanelStore(client: client)
        // A fresh contributor directory with no configuration yet: the core
        // words Automatic's arming disclosure for it.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("tc-pill-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        store.configDirectory = dir.path
        await store.load()
        return (store, client)
    }

    /// Choose shows the core's confirmation and sends nothing; confirm sends
    /// the override and the pill reads it back from `status`.
    func test_chooseThenConfirmSetsTheOverrideFromStatus() async throws {
        let (store, client) = await loadedStore(.normalDay)
        XCTAssertTrue(store.canChooseOverride)
        store.choose("ignore")
        let confirming = try XCTUnwrap(store.confirming)
        XCTAssertEqual(confirming, ContributionOverrideConfirmCopy.decode(
            fromJSON: TCCoreCopy.contributionOverrideConfirmJSON(mode: "ignore", configDir: nil)))
        XCTAssertEqual(client.overrideCalls, [], "choosing sends nothing")
        XCTAssertFalse(store.canChooseOverride, "no second choice while one is being confirmed")

        await store.resolveConfirmation(confirmed: true)
        XCTAssertNil(store.confirming)
        XCTAssertEqual(client.overrideCalls, ["set_contribution_override ignore"])
        XCTAssertEqual(store.status?.contributionMode, "ignore")
        XCTAssertEqual(store.status?.contributionOverride?.mode, "ignore")
        XCTAssertNil(store.overrideRefusal)

        await store.clearOverride()
        XCTAssertEqual(client.overrideCalls.last, "clear_contribution_override")
        XCTAssertNil(store.status?.contributionOverride)
    }

    /// Mixed clears an override at once when no folder's own setting is
    /// Automatic, and does nothing while no override is in force.
    func test_mixedClearsAtOnceWhenNoFolderIsAutomatic() async throws {
        let (store, client) = await loadedStore(.normalDay)
        await store.chooseMixed()
        XCTAssertEqual(client.overrideCalls, [], "nothing to clear")
        store.choose("ignore")
        await store.resolveConfirmation(confirmed: true)
        await store.chooseMixed()
        XCTAssertNil(store.confirming)
        XCTAssertEqual(client.overrideCalls.last, "clear_contribution_override")
        XCTAssertNil(store.status?.contributionOverride)
    }

    /// When a folder's own setting is Automatic, Mixed shows the core's
    /// clear confirmation and sends nothing; cancel keeps the override;
    /// confirm clears it.
    func test_mixedConfirmsBeforeAnAutomaticFolderSendsAgain() async throws {
        let (store, client) = await loadedStore(.armedFolder)
        store.choose("ignore")
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(store.status?.contributionOverride?.mode, "ignore")
        let sent = client.overrideCalls

        await store.chooseMixed()
        let confirming = try XCTUnwrap(store.confirming)
        XCTAssertEqual(confirming.mode, ContributionOverrideConfirmCopy.clearMode)
        XCTAssertEqual(confirming, ContributionOverrideConfirmCopy.decode(
            fromJSON: TCCoreCopy.contributionOverrideConfirmJSON(mode: "clear", configDir: nil)))
        XCTAssertNil(confirming.arming)
        XCTAssertEqual(client.overrideCalls, sent, "choosing Mixed sent nothing")

        await store.resolveConfirmation(confirmed: false)
        XCTAssertEqual(client.overrideCalls, sent, "a cancelled clear sent a write")
        XCTAssertEqual(store.status?.contributionOverride?.mode, "ignore")

        await store.chooseMixed()
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(client.overrideCalls.last, "clear_contribution_override")
        XCTAssertNil(store.status?.contributionOverride)
    }

    /// Clearing is confirmed when a folder is Automatic, and when the
    /// folders or a folder's own setting are not known (fail closed).
    func test_clearNeedsConfirmationFailsClosed() {
        let ask = ProjectRow(projectId: "a", projectLabel: "a", mode: .ignore, folderMode: .ask)
        let never = ProjectRow(projectId: "n", projectLabel: "n", mode: .ignore, folderMode: .ignore)
        let auto = ProjectRow(projectId: "u", projectLabel: "u", mode: .ignore, folderMode: .autoUpload)
        let unknown = ProjectRow(projectId: "x", projectLabel: "x", mode: .ignore)
        XCTAssertFalse(MenuPanelData.clearNeedsConfirmation([], override: "ignore"))
        XCTAssertFalse(MenuPanelData.clearNeedsConfirmation([ask, never], override: "ignore"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation([ask, auto], override: "ignore"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation([ask, unknown], override: "ignore"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation(nil, override: "ignore"))
    }

    /// Under an Automatic override, clearing starts no new unattended
    /// sending (a folder set to Automatic already sends under its own
    /// arming, and Ask folders go back to asking), so Mixed clears at
    /// once; under Ask or Never it is still confirmed (#1256 review).
    func test_mixedClearsAnAutomaticOverrideAtOnce() async throws {
        let ask = ProjectRow(projectId: "a", projectLabel: "a", mode: .ignore, folderMode: .ask)
        let auto = ProjectRow(projectId: "u", projectLabel: "u", mode: .ignore, folderMode: .autoUpload)
        XCTAssertFalse(MenuPanelData.clearNeedsConfirmation([ask, auto], override: "auto_upload"))
        XCTAssertFalse(MenuPanelData.clearNeedsConfirmation(nil, override: "auto_upload"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation([ask, auto], override: "ignore"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation([ask, auto], override: "notify_only"))
        XCTAssertTrue(MenuPanelData.clearNeedsConfirmation([ask, auto], override: nil))

        let (store, client) = await loadedStore(.armedFolder)
        store.choose("auto_upload")
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(store.status?.contributionOverride?.mode, "auto_upload")
        await store.chooseMixed()
        XCTAssertNil(store.confirming)
        XCTAssertEqual(client.overrideCalls.last, "clear_contribution_override")
        XCTAssertNil(store.status?.contributionOverride)
    }

    /// A stale store sends nothing from Mixed, even with an override in
    /// force and an Automatic folder.
    func test_mixedDoesNothingWhileStale() async throws {
        let (store, client) = await loadedStore(.armedFolder)
        store.choose("ignore")
        await store.resolveConfirmation(confirmed: true)
        let sent = client.overrideCalls
        store.attach(client)
        XCTAssertFalse(store.canChooseOverride)
        await store.chooseMixed()
        XCTAssertNil(store.confirming)
        XCTAssertEqual(client.overrideCalls, sent)
    }

    /// With no override in force, the checked row is the roll-up itself:
    /// pressing it only closes the list and sets no override. While an
    /// override is in force, or before the status is read, every row is a
    /// real choice (#1255 review).
    func test_pressingTheRollupsOwnRowOnlyClosesTheList() async throws {
        let (store, _) = await loadedStore(.normalDay)
        let rollup = try XCTUnwrap(store.status?.contributionMode)
        XCTAssertEqual(rollup, "notify_only")
        XCTAssertTrue(MenuPanelData.pressOnlyCloses(rollup, status: store.status, stale: false))
        XCTAssertFalse(MenuPanelData.pressOnlyCloses("ignore", status: store.status, stale: false))
        XCTAssertFalse(MenuPanelData.pressOnlyCloses(rollup, status: store.status, stale: true))
        XCTAssertFalse(MenuPanelData.pressOnlyCloses(rollup, status: nil, stale: false))
        store.choose("ignore")
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(store.status?.contributionOverride?.mode, "ignore")
        XCTAssertFalse(MenuPanelData.pressOnlyCloses("ignore", status: store.status, stale: false),
                       "under an override the checked row is the override, not a no-op")

        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // The popover draws no override list any more (owner, 2026-10-19).
        XCTAssertFalse(source.contains("MenuPanelData.pressOnlyCloses("))
    }

    /// The Mixed row goes through `chooseMixed`, which confirms; the panel
    /// never calls the unconfirmed `clearOverride` (#1256 review).
    func test_theMixedRowGoesThroughChooseMixed() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // The Mixed row left with the mode pill (owner, 2026-10-19), and
        // the unconfirmed clear is never called.
        XCTAssertFalse(source.contains("store.chooseMixed()"))
        XCTAssertFalse(source.contains("clearOverride()"))
    }

    /// Automatic's confirmation carries the arming disclosure, and its
    /// confirm sends `confirm: true`.
    func test_autoContributeConfirmsWithTheArmingDisclosure() async throws {
        let (store, client) = await loadedStore(.normalDay)
        store.choose("auto_upload")
        let confirming = try XCTUnwrap(store.confirming)
        let arming = try XCTUnwrap(confirming.arming)
        XCTAssertFalse(arming.lines.isEmpty)
        XCTAssertTrue(confirming.paragraphs.contains(arming.lines[0]))
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(client.overrideCalls, ["set_contribution_override auto_upload confirm"])
        XCTAssertEqual(store.status?.contributionOverride?.mode, "auto_upload")
    }

    /// Cancel sends nothing and changes nothing.
    func test_cancelSendsNothing() async throws {
        let (store, client) = await loadedStore(.normalDay)
        let before = store.status
        for mode in ["notify_only", "auto_upload", "ignore"] {
            store.choose(mode)
            XCTAssertNotNil(store.confirming, mode)
            await store.resolveConfirmation(confirmed: false)
            XCTAssertNil(store.confirming, mode)
        }
        XCTAssertEqual(client.overrideCalls, [], "a cancelled confirmation sent a write")
        XCTAssertEqual(store.status, before)
    }

    /// A refusal shows the core's line for its label, never the error's
    /// description, and the pill keeps reading `status`.
    func test_aRefusalShowsItsCoreLine() async throws {
        let (store, client) = await loadedStore(.empty)
        store.choose("auto_upload")
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(client.overrideCalls, ["set_contribution_override auto_upload confirm"])
        let line = try XCTUnwrap(store.overrideRefusal)
        XCTAssertEqual(line, TCCoreCopy.contributionOverrideRefusalLine(label: "arming-terms-unavailable"))
        XCTAssertFalse(line.contains("arming-terms-unavailable"))
        XCTAssertNil(store.status?.contributionOverride)
        // The next choice clears it.
        store.choose("ignore")
        XCTAssertNil(store.overrideRefusal)
    }

    /// Without a configuration the core can word the arming disclosure for,
    /// Automatic shows no confirmation and so cannot be confirmed: the
    /// refusal's line instead, and nothing is sent.
    func test_autoContributeWithoutItsDisclosureCannotBeConfirmed() async throws {
        let (store, client) = await loadedStore(.normalDay)
        store.configDirectory = nil
        store.choose("auto_upload")
        XCTAssertNil(store.confirming)
        XCTAssertEqual(store.overrideRefusal, TCCoreCopy.contributionOverrideRefusalLine(label: "arming-terms-unavailable"))
        await store.resolveConfirmation(confirmed: true)
        XCTAssertEqual(client.overrideCalls, [])
    }

    /// Core-down, never-loaded and no-client stores never let a write
    /// through.
    func test_theChoicesAreDisabledWithoutACurrentStatus() async {
        let none = MenuPanelStore(client: nil)
        XCTAssertFalse(none.canChooseOverride)
        let unloaded = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        XCTAssertFalse(unloaded.canChooseOverride, "nothing read yet")
        unloaded.choose("ignore")
        XCTAssertNil(unloaded.confirming)
        let (down, client) = await loadedStore(.coreDown)
        XCTAssertFalse(down.canChooseOverride)
        down.choose("ignore")
        XCTAssertNil(down.confirming)
        await down.clearOverride()
        XCTAssertEqual(client.overrideCalls, [])
    }

    /// The popover's words come from the core's table, not from Swift.
    func test_theWordsComeFromTheCore() {
        XCTAssertNotNil(MonitorWords.table)
        XCTAssertEqual(MenuWords.on, MonitorWords.table?.on)
        XCTAssertFalse(MenuWords.quit.isEmpty)
    }

    /// The popover keeps its top edge under the menu bar when its content
    /// grows or shrinks: the window takes the content's height and extends
    /// downward from the menu bar, whichever edge AppKit kept.
    func test_thePopoverKeepsItsTopEdgeWhenItsHeightChanges() {
        let top: CGFloat = 870
        // Shrunk with its bottom edge kept: the top fell 120pt.
        let shrunk = CGRect(x: 900, y: 400, width: 380, height: 350)
        XCTAssertEqual(MenuBarTopEdgePin.frame(current: shrunk, content: CGSize(width: 380, height: 350), top: top),
                       CGRect(x: 900, y: 520, width: 380, height: 350))
        // Content that grew: the top is put back too, and the window extends downward.
        XCTAssertEqual(MenuBarTopEdgePin.frame(current: shrunk, content: CGSize(width: 380, height: 520), top: top),
                       CGRect(x: 900, y: 350, width: 380, height: 520))
        // Already in place: nothing moves.
        XCTAssertNil(MenuBarTopEdgePin.frame(
            current: CGRect(x: 900, y: 400, width: 380, height: 470), content: CGSize(width: 380, height: 470), top: top))
    }

    /// The pinned top is the menu bar's bottom edge on the window's screen,
    /// never a top the window happened to have: a panel that shrank before
    /// it became key no longer hangs below the bar.
    func test_thePopoverPinsToTheMenuBarsBottomEdge() {
        let screen = CGRect(x: 0, y: 0, width: 1512, height: 982)
        // The menu bar shown: the visible frame stops under it.
        XCTAssertEqual(MenuBarTopEdgePin.menuBarBottom(
            screen: screen, visible: CGRect(x: 0, y: 0, width: 1512, height: 945), thickness: 24), 945)
        // The menu bar hidden: the visible frame reaches the top.
        XCTAssertEqual(MenuBarTopEdgePin.menuBarBottom(screen: screen, visible: screen, thickness: 24), 958)
    }

    /// A notification can arrive inside a resize of the panel, so the pin
    /// sets the window's frame on the next turn of the run loop rather
    /// than there, the shape that crashed the edge rail (#1402).
    func test_thePinMovesTheWindowOutsideTheResizePass() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("GlassWindowFrame.onNextTurn { [weak self] in"))
    }


    // MARK: The settlement estimate line

    private static let settlementCopy = NudgeCopy(table: [
        "ESTIMATE_TOTAL_LABEL": "Estimate", "ESTIMATE_TOTAL_ACTION": "Review traces",
    ])

    private static let settlementStatus = #"""
        {"settlement_estimate":{"low_usd_micros":3500000,"high_usd_micros":9000000,"up_to":false,
         "known":4,"unknown":1,"table_version":"t1","pricing_version":"p1","estimated_at":"2026-10-15T09:00:00Z",
         "text":"About $3.50 to $9 of inference in traces on this Mac"}}
        """#

    private func decodeStatus(_ json: String) throws -> DaemonData.Status {
        try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
    }

    func testSettlementLineRendersFromAStatusFixture() throws {
        let status = try decodeStatus(Self.settlementStatus)
        let estimate = try XCTUnwrap(status.settlementEstimate)
        XCTAssertEqual(estimate.lowUsdMicros, 3_500_000)
        XCTAssertEqual(estimate.highUsdMicros, 9_000_000)
        XCTAssertEqual(estimate.known, 4)
        XCTAssertEqual(estimate.pricingVersion, "p1")
        let line = MenuPanelStatus.settlementLine(status, available: true, copy: Self.settlementCopy)
        XCTAssertEqual(line, MenuPanelStatus.SettlementLine(
            text: "About $3.50 to $9 of inference in traces on this Mac", label: "Estimate", action: "Review traces"))
    }

    func testSettlementLineIsAbsentWhenNotDrawable() throws {
        // No key: nothing is drawn, never a $0 line.
        XCTAssertNil(MenuPanelStatus.settlementLine(try decodeStatus("{}"), available: true, copy: Self.settlementCopy))
        XCTAssertNil(MenuPanelStatus.settlementLine(nil, available: true, copy: Self.settlementCopy))
        // A panel that cannot vouch for its data draws nothing.
        let status = try decodeStatus(Self.settlementStatus)
        XCTAssertNil(MenuPanelStatus.settlementLine(status, available: false, copy: Self.settlementCopy))
        // The core's words missing: nothing is drawn.
        XCTAssertNil(MenuPanelStatus.settlementLine(status, available: true, copy: NudgeCopy(table: [:])))
        XCTAssertNil(MenuPanelStatus.settlementLine(status, available: true, copy: nil))
    }

    /// The popover offers "Check for Updates…" only where Sparkle runs, and
    /// disabled while the updater cannot check (owner, 2026-10-29).
    func test_theMenuChecksForUpdatesOnlyWhenSelfUpdating() throws {
        let panel = try GlassSurfaceRulesTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("@ObservedObject private var updates = UpdateController.shared"))
        XCTAssertTrue(panel.contains("if updates.mode == .selfUpdating, let title = MenuWords.checkForUpdates {"))
        XCTAssertTrue(panel.contains("updates.checkNow()"))
        XCTAssertTrue(panel.contains(".disabled(!updates.canCheckNow)"))
        // Sparkle's window opens in front of a menu-bar app.
        XCTAssertTrue(panel.contains("NSApp.activate(ignoringOtherApps: true)\n                    updates.checkNow()"))
    }

    /// The label is the core's word, with a real ellipsis.
    func test_theUpdateCheckWordComesFromTheCore() throws {
        let copy = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        XCTAssertEqual(copy.checkForUpdates, "Check for Updates\u{2026}")
    }
}
