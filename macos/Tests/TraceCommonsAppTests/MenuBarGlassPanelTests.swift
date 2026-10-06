import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R13 of #1173: the menu-bar popover from the handoff, under its rules.
@MainActor
final class MenuBarGlassPanelTests: XCTestCase {
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
        XCTAssertTrue(source.contains("store.status?.contributionMode"))
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
        XCTAssertEqual(checked(before), ["notify_only"], "every folder on Ask me checks Ask me, not Mixed")
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

    // MARK: Day graph

    /// Contributions go up and kept sessions go down, on the day they
    /// happened; the newest day is the last column; older days fall off.
    func test_theGraphCountsPerDayEndingToday() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let day: TimeInterval = 86_400
        let columns = MenuPanelData.days(
            shared: [now, now - 60, now - day, now - 40 * day],
            kept: [now - 2 * day], ending: now, count: 36, calendar: calendar)
        XCTAssertEqual(columns.count, 36)
        XCTAssertEqual(columns.last?.up, 2)
        XCTAssertEqual(columns[34].up, 1)
        XCTAssertEqual(columns[33].down, 1)
        XCTAssertEqual(columns.reduce(0) { $0 + $1.up }, 3, "a day outside the window is not drawn")
    }

    // MARK: Flagged and recent

    /// Flagged counts the queue's attention reasons only.
    func test_flaggedCountsNothingMatchedAndTrimmed() async throws {
        // A real sample entry, with only its second-look reasons varied.
        let pending = try await SampleDaemonClient(.normalDay).listPending(projectId: nil)
        let sample = try XCTUnwrap(pending.first)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        func entry(_ id: String, _ reasons: [String]) throws -> DaemonData.QueueEntry {
            var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoder.encode(sample)) as? [String: Any])
            object["entry_id"] = id
            object["second_look"] = reasons
            return try decoder.decode(DaemonData.QueueEntry.self, from: JSONSerialization.data(withJSONObject: object))
        }
        let entries = try [
            entry("a", ["nothing-matched"]), entry("b", ["trimmed-to-fit"]),
            entry("c", ["looks-unsure"]), entry("d", []),
        ]
        XCTAssertEqual(MenuPanelData.flagged(entries), 2)
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

    // MARK: Badge

    func test_theBadgeCountsDecisionsOwedOnly() {
        XCTAssertEqual(MenuPanelStatus.badge(9), 9)
        XCTAssertNil(MenuPanelStatus.badge(0))
        XCTAssertNil(MenuPanelStatus.badge(nil))
    }

    // MARK: Rules

    /// Nothing is sent from the popover, and nothing is turned on from it:
    /// its only writes are the shipping menu's pause, resume and Private AI
    /// off, and the contribution override, which goes through the store
    /// after its core confirmation and only while the store allows it.
    func test_thePopoverMakesOnlyTheShippingMenusWrites() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        // No credit figure of any kind: no projected or pending credit.
        for forbidden in ["setProjectMode", "approve(", "applyPrivateInference(true)", "setPrivateAI(",
                          "setContributionOverride(", "clearContributionOverride(", ".disabled(true)",
                          "creditPoints", "creditPending", "creditFinal", "creditRange", "commonsCreditSummary"] {
            XCTAssertFalse(source.contains(forbidden), "the popover contains \(forbidden)")
        }
        XCTAssertTrue(source.contains("modeOptions"))
        XCTAssertTrue(source.contains(".disabled(!store.canChooseOverride)"),
                      "the choices are disabled unless the store has positive evidence the core is up")
        XCTAssertTrue(source.contains("store.resolveConfirmation(confirmed:"))
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
        XCTAssertEqual(MenuBarGlassPanel.modeCaption, MonitorWords.table?.contributionMode)
        XCTAssertFalse(MenuWords.quit.isEmpty)
    }
}
