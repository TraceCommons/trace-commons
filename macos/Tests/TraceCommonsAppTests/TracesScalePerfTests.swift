import AppKit
import Observation
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Traces tab at the owner's scale (231 watched folders, 180 sessions
/// waiting), timed. Measurement harness: every test prints `PERF <key> ...`
/// lines and asserts nothing about time, so it never flakes. The guards are
/// counts, in `TracesTreeScaleTests`, which shares this fixture and host.
///
/// Skipped unless `TC_PERF=1`: it takes about forty seconds.
/// Run: `cargo build -p trace-commons-contributor-ffi` from the repo root,
/// then `TC_PERF=1 swift test --filter TracesScalePerfTests 2>&1 | grep PERF`.
/// Release numbers: build the dylib with `--release`, then
/// `TC_PERF=1 TC_FFI_LIB_DIR=../target/release swift test -c release
/// -Xswiftc -enable-testing -Xswiftc -DDEBUG --filter TracesScalePerfTests`
/// (`-DDEBUG` only because `SampleDaemonClient` is DEBUG-only).
/// `TC_PERF_FOLDERS=<n>` changes the folder count.
@MainActor
final class TracesScalePerfTests: XCTestCase {
    override func setUp() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["TC_PERF"] == "1", "set TC_PERF=1 to time the tab")
    }
    // MARK: Fixture

    struct Scale {
        var folders = ProcessInfo.processInfo.environment["TC_PERF_FOLDERS"].flatMap(Int.init) ?? 231
        var sessions = 180
        /// How many of the folders hold the waiting sessions; the rest are
        /// watched folders with nothing waiting.
        var foldersWithSessions = 48
        /// Every n-th folder is set to ignore.
        var ignoreEvery = 17
    }

    static let words = [
        "trace-commons-server", "ironclaw", "zcash", "zebra", "librustzcash", "orchard", "halo2",
        "sapling-crypto", "zallet", "zingolib", "penumbra", "cosmos-sdk", "ibc-rs", "tendermint-rs",
        "sommelier", "iqlusion", "tmkms", "abscissa", "near-ai", "ironwire", "dotfiles", "notes",
        "site", "blog", "talks", "scratch", "experiments", "hackathon", "pilot-infra", "gate-bench",
    ]

    /// `list_pending`, `list_projects` and `status` replies at `scale`, in
    /// the shapes the daemon sends (busyQueue's recorded rows, widened).
    /// `bump` changes one session's size, for a reload that changes one row.
    static func replies(_ scale: Scale = Scale(), bump: Int = 0) throws -> (pending: String, projects: String, status: String) {
        let base = Date(timeIntervalSince1970: 1_792_000_000)
        let iso = ISO8601DateFormatter()
        var projects: [[String: Any]] = []
        for i in 0..<scale.folders {
            let word = words[i % words.count]
            let label = i < words.count ? word : "\(word)-\(i / words.count)"
            let ignored = scale.ignoreEvery > 0 && i % scale.ignoreEvery == scale.ignoreEvery - 1
            let mode = ignored ? "ignore" : (i % 23 == 5 ? "auto_upload" : "notify_only")
            var row: [String: Any] = [
                "project_id": String(format: "proj_%016x", 0x9c31_04d8_0000 + i),
                "project_label": label,
                "project_path": "/Users/sample/code/\(label)",
                "mode": mode, "folder_mode": mode,
                "added_at": iso.string(from: base.addingTimeInterval(-Double(i) * 3600)),
                "configured": true, "is_unresolved_bucket": false,
                "session_count": 3 + i % 40,
                "last_session_at": iso.string(from: base.addingTimeInterval(-Double(i) * 600)),
                "tools": [["source": "claude-code", "session_count": 3 + i % 40, "answers_at": "Anthropic"]],
            ]
            if mode == "auto_upload" { row["automatic_disclosure"] = "standard" }
            projects.append(row)
        }
        var pending: [[String: Any]] = []
        var perFolder: [Int: Int] = [:]
        var contributable: [Int: Int] = [:]
        for n in 0..<scale.sessions {
            // Skewed: a few folders hold most of the queue, as a real one does.
            let f = n < scale.sessions / 2 ? n % 8 : (n * 7) % scale.foldersWithSessions
            let project = projects[f]
            let eligible = n % 3 != 0
            perFolder[f, default: 0] += 1
            if eligible { contributable[f, default: 0] += 1 }
            var entry: [String: Any] = [
                "entry_id": String(format: "e%07d-2d9c-5b31-afef-faec561993cb", n),
                "session_hash": "perf-\(n)",
                "source": n % 4 == 0 ? "codex" : "claude-code",
                "declared_source": NSNull(),
                "project_id": project["project_id"]!,
                "project_label": project["project_label"]!,
                "project_path": project["project_path"]!,
                "session_path": NSNull(),
                "size_bytes": 12_100 + n * 917 + (n == 0 ? bump : 0),
                "discovered_at": iso.string(from: base.addingTimeInterval(-Double(n) * 1_800)),
                "started_at": iso.string(from: base.addingTimeInterval(-Double(n) * 1_800 - 900)),
                "ended_at": iso.string(from: base.addingTimeInterval(-Double(n) * 1_800 - 60)),
                "duration_secs": 840 + n,
                "state": "pending", "reason_label": NSNull(), "attempts": 0, "retry_after": NSNull(),
                "submission_id": NSNull(), "subagent_count": n % 3, "subagents_dropped": n % 29 == 0 ? 1 : 0,
                "user_turns": 4 + n % 9, "title": "Session \(n)", "would_send_bytes": NSNull(),
                "holds_certificate": false, "attestation": "unknown", "scrub": "not-yet-scrubbed",
                "second_look": n % 11 == 0 ? ["trimmed-to-fit"] : [], "second_look_lines": [],
                "eligibility": eligible ? "eligible" : "ineligible_configuration",
                "mission_fit": n % 5,
            ]
            if !eligible { entry["eligibility_reason"] = "capture_off" }
            pending.append(entry)
        }
        for (f, count) in perFolder {
            projects[f]["pending_count"] = count
            projects[f]["contributable_count"] = contributable[f] ?? 0
        }
        for i in projects.indices where projects[i]["pending_count"] == nil {
            projects[i]["pending_count"] = 0
            projects[i]["contributable_count"] = 0
        }
        // The idle filter lists every waiting session (state (iii) below).
        let recorded = try XCTUnwrap(SampleDaemonClient(.busyQueue).json(for: "status"))
        var status = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(recorded.utf8)) as? [String: Any])
        status["idle_sessions"] = ["count": scale.sessions, "tools": ["Claude Code", "Codex"], "threshold_days": 3]
        func text(_ object: Any) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]), as: UTF8.self)
        }
        return (
            try text(["pending": pending]),
            try text(["projects": projects, "unpurposed_traces": 0]),
            try text(status))
    }

    static func client(_ scale: Scale = Scale(), bump: Int = 0) throws -> SampleDaemonClient {
        let client = SampleDaemonClient(.busyQueue)
        let r = try replies(scale, bump: bump)
        client.replaceReply("list_pending", with: r.pending)
        client.replaceReply("list_projects", with: r.projects)
        client.replaceReply("status", with: r.status)
        return client
    }

    func loadedStore(_ scale: Scale = Scale()) async throws -> (TracesStore, SampleDaemonClient) {
        let client = try Self.client(scale)
        let store = TracesStore(client: client)
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
        return (store, client)
    }

    // MARK: Timing

    static func ms(_ d: Duration) -> Double {
        Double(d.components.seconds) * 1000 + Double(d.components.attoseconds) / 1e15
    }

    /// Min and median of `runs` timed calls, after `warmup` untimed ones.
    @discardableResult
    static func time(_ key: String, runs: Int = 15, warmup: Int = 2, _ body: () -> Void) -> (min: Double, median: Double) {
        for _ in 0..<warmup { body() }
        let clock = ContinuousClock()
        var samples: [Double] = []
        for _ in 0..<runs { samples.append(ms(clock.measure(body))) }
        samples.sort()
        let result = (min: samples[0], median: samples[samples.count / 2])
        print(String(format: "PERF %@ min=%.3fms median=%.3fms runs=%d", key, result.min, result.median, runs))
        return result
    }

    static func report(_ key: String, _ samples: [Double]) {
        let s = samples.sorted()
        print(String(format: "PERF %@ min=%.3fms median=%.3fms max=%.3fms runs=%d",
                     key, s[0], s[s.count / 2], s[s.count - 1], s.count))
    }

    // MARK: (a) derive / build

    func test_a_derive() async throws {
        let (store, client) = try await loadedStore()
        let entries = try await client.listPending(projectId: nil, filter: nil, order: .suggested)
        let projects = try await client.listProjects().projects
        let settings = try? await client.settings()
        let read: TracesStore.Read = (entries: entries, projects: projects, settings: settings)
        print("PERF fixture folders=\(projects.count) sessions=\(entries.count) "
            + "drawnFolders=\(store.tree.folders.count) hiddenIgnored=\(store.tree.hiddenIgnored)")
        let copy = store.inferenceCopy
        Self.time("a.derive.total") {
            _ = TracesStore.derive(read, showsIgnored: true, keepsOrder: true, idleOnly: false, copy: copy)
        }
        Self.time("a.derive.build") {
            _ = TracesStore.build(read, showsIgnored: true, keepsOrder: true, idleOnly: false)
        }
        Self.time("a.derive.rowLines") {
            for entry in entries { _ = TracesStore.rowLines(entry, copy: copy) }
        }
        Self.time("a.derive.rowTags") { _ = TracesStore.rowTags(entries) }
        // Decoding the replies themselves (LiveDaemonClient does this on its
        // work queue, off main).
        let r = try Self.replies()
        // Through the sample client's decoder (LiveDaemonClient runs the
        // same decoder on its work queue, off main).
        let clock0 = ContinuousClock()
        var decodes: [Double] = []
        var projectDecodes: [Double] = []
        for _ in 0..<10 {
            let t0 = clock0.now
            _ = try await client.listPending(projectId: nil, filter: nil, order: nil)
            decodes.append(Self.ms(clock0.now - t0))
            let t1 = clock0.now
            _ = try await client.listProjects()
            projectDecodes.append(Self.ms(clock0.now - t1))
        }
        Self.report("a.decode.list_pending", decodes)
        // AppModel's own read of the same reply (DaemonClient.listPending ->
        // DaemonDecoding.pendingEntries), on a detached task per event.
        let pendingData = Data(try Self.replies().pending.utf8)
        Self.time("a.decode.appModel.pendingEntries", runs: 5) {
            _ = try? DaemonDecoding.pendingEntries(from: pendingData)
        }
        Self.report("a.decode.list_projects", projectDecodes)
        print("PERF a.bytes list_pending=\(r.pending.utf8.count) list_projects=\(r.projects.utf8.count)")
        // On the main actor once the read is in: the tree swap. `tree`'s
        // didSet narrows (a copy with no search), and every
        // `.onChange(of: store.tree)` compares the old tree with the new.
        let a = store.tree
        let b = TracesStore.build(read, showsIgnored: true, keepsOrder: true, idleOnly: false)
        Self.time("a.main.treeEquality") { _ = a == b }
        Self.time("a.main.allSessions") { _ = store.tree.allSessions }
        // A full load through the sample client (its JSON decode included),
        // awaited from the main actor.
        let clock = ContinuousClock()
        var loads: [Double] = []
        for _ in 0..<8 {
            let start = clock.now
            await store.load()
            loads.append(Self.ms(clock.now - start))
        }
        Self.report("a.load.endToEnd(sampleClient)", loads)
    }

    // MARK: Timestamp decoding

    /// `DaemonDataDecoding.parseDate` builds one or two ISO8601DateFormatter
    /// per timestamp. Compared with the stock `.iso8601` strategy.
    func test_h_dateDecode() throws {
        let plain = (0..<1000).map { "\"2026-10-0\($0 % 9 + 1)T09:12:00Z\"" }
        let fraction = (0..<1000).map { "\"2026-10-0\($0 % 9 + 1)T09:12:00.123456Z\"" }
        let plainJSON = Data(("[" + plain.joined(separator: ",") + "]").utf8)
        let fractionJSON = Data(("[" + fraction.joined(separator: ",") + "]").utf8)
        Self.time("h.daemonDecoder 1000 dates (no fraction)", runs: 5) {
            _ = try? DaemonDataDecoding.decoder().decode([Date].self, from: plainJSON)
        }
        Self.time("h.daemonDecoder 1000 dates (fraction)", runs: 5) {
            _ = try? DaemonDataDecoding.decoder().decode([Date].self, from: fractionJSON)
        }
        let stock = JSONDecoder()
        stock.dateDecodingStrategy = .iso8601
        Self.time("h.stockIso8601 1000 dates (no fraction)", runs: 5) {
            _ = try? stock.decode([Date].self, from: plainJSON)
        }
        let style = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
        let styled = JSONDecoder()
        styled.dateDecodingStrategy = .custom { d in
            let text = try d.singleValueContainer().decode(String.self)
            if let date = try? style.parse(text) { return date }
            return try Date.ISO8601FormatStyle().parse(text)
        }
        Self.time("h.cachedFormatStyle 1000 dates (no fraction)", runs: 5) {
            _ = try? styled.decode([Date].self, from: plainJSON)
        }
        Self.time("h.cachedFormatStyle 1000 dates (fraction)", runs: 5) {
            _ = try? styled.decode([Date].self, from: fractionJSON)
        }
    }

    // MARK: (b) search

    func test_b_matching() async throws {
        let (store, _) = try await loadedStore()
        let tree = store.tree
        for query in ["a", "ir", "zcash", "Session 1", "zzz-none"] {
            var shown = 0
            Self.time("b.matching[\(query)]") { shown = tree.matching(query).folders.count }
            print("PERF b.matching[\(query)].folders=\(shown)")
        }
    }

    // MARK: (c)-(e) the tree drawn

    @Observable
    final class SelectionBox {
        var selection: MonitorSelection?
        var renders = 0
        /// The Traces tab shown (false draws nothing in its place).
        var shown = true
        /// Something unrelated the window's body reads.
        var unrelated = 0
    }

    /// The tree as the window hosts it: pane content width, a realistic
    /// tree-pane height, the app model in the environment.
    struct Hosted: View {
        let store: TracesStore
        @Bindable var box: SelectionBox
        let model: AppModel

        var body: some View {
            if box.shown {
                TracesTreeView(store: store, selection: $box.selection, missionsShown: model.missionsShown)
                    .equatable()
                    .environmentObject(model)
                    // The window is never shown, so no scroll animation runs
                    // in it: an animated scroll never moves.
                    .environment(\.tracesScrollAnimates, false)
                    .onChange(of: box.selection) { box.renders += 1 }
                    .onChange(of: store.tree) { box.renders += 1 }
                    .onAppear { box.renders += 1 }
            } else {
                Color.clear.onAppear { box.renders += 1 }
            }
        }
    }

    /// The tree as MonitorWindowView builds it (MonitorWindowView.swift
    /// :218): a fresh `Binding(get:set:)` and closures on every pass of a
    /// parent whose body also reads something unrelated.
    struct WindowLike: View {
        let store: TracesStore
        @Bindable var box: SelectionBox
        let model: AppModel

        var body: some View {
            let _ = box.unrelated
            TracesTreeView(
                store: store,
                selection: Binding(get: { box.selection }, set: { box.selection = $0 }),
                missionsShown: model.missionsShown,
                onReview: { box.selection = .session(entryID: $0) },
                openHistory: {})
                .equatable()
                .environmentObject(model)
                .onChange(of: box.unrelated) { box.renders += 1 }
        }
    }

    nonisolated static let size = CGSize(
        width: InsightsPaneWidthRenderTests.paneContentWidth, height: 760)

    @MainActor final class Host {
        let window: NSWindow
        let hosting: NSView
        init(_ view: some View) {
            _ = NSApplication.shared
            let bounds = NSRect(origin: .zero, size: TracesScalePerfTests.size)
            let hosting = NSHostingView(rootView: view.frame(width: bounds.width, height: bounds.height))
            hosting.frame = bounds
            let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            self.window = window
            self.hosting = hosting
        }
        /// Never ordered on screen: nothing is captured or shown.
        func draw() {
            hosting.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
        }
        func close() { window.close() }
    }

    /// Runs the main run loop until `done`, then lays out and draws: the
    /// work a change costs the main thread before the next frame.
    static func settle(_ host: Host, until done: () -> Bool) -> Double {
        let clock = ContinuousClock()
        let start = clock.now
        let deadline = Date().addingTimeInterval(5)
        while !done(), Date() < deadline {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.0002))
        }
        let pumped = clock.now
        host.hosting.layoutSubtreeIfNeeded()
        let laidOut = clock.now
        host.window.displayIfNeeded()
        let end = clock.now
        breakdown.append((ms(pumped - start), ms(laidOut - pumped), ms(end - laidOut)))
        return ms(end - start)
    }

    /// (run loop incl. body passes, layout, display) per `settle`.
    static var breakdown: [(Double, Double, Double)] = []

    static func reportBreakdown(_ key: String) {
        let b = breakdown.dropFirst(2)
        func med(_ xs: [Double]) -> Double { xs.sorted()[xs.count / 2] }
        print(String(format: "PERF %@.breakdown runloop+update=%.1fms layout=%.1fms display=%.1fms",
                     key, med(b.map(\.0)), med(b.map(\.1)), med(b.map(\.2))))
        breakdown = []
    }

    func test_c_initialLayout() async throws {
        let (store, _) = try await loadedStore()
        let model = AppModel()
        let clock = ContinuousClock()
        var cold: [Double] = []
        for _ in 0..<6 {
            let box = SelectionBox()
            let start = clock.now
            let host = Host(Hosted(store: store, box: box, model: model))
            host.draw()
            cold.append(Self.ms(clock.now - start))
            host.close()
        }
        Self.report("c.initialLayout.allCollapsed(\(store.tree.folders.count) folder rows)", cold)

        // Everything open: the idle filter opens every folder it lists.
        await store.showIdleOnly(true)
        var open: [Double] = []
        for _ in 0..<6 {
            let b = SelectionBox()
            let start = clock.now
            let h = Host(Hosted(store: store, box: b, model: model))
            h.draw()
            open.append(Self.ms(clock.now - start))
            h.close()
        }
        Self.report("c.initialLayout.idleAllOpen(\(store.tree.folders.count) folders, \(store.tree.allSessions.count) sessions)", open)
    }

    /// Switching back to the Traces tab in an existing window: the tree is
    /// rebuilt inside a hosting view that is already up.
    func test_c2_tabSwitch() async throws {
        let (store, _) = try await loadedStore()
        let model = AppModel()
        let box = SelectionBox()
        box.shown = false
        let host = Host(Hosted(store: store, box: box, model: model))
        host.draw()
        _ = Self.settle(host) { true }
        var samples: [Double] = []
        for _ in 0..<8 {
            let before = box.renders
            box.shown = true
            samples.append(Self.settle(host) { box.renders > before })
            box.shown = false
            let hidden = box.renders
            _ = Self.settle(host) { box.renders > hidden }
        }
        Self.report("c2.tabSwitch.toTraces(collapsed, \(store.tree.folders.count) folders)", samples)
        Self.breakdown = []
        host.close()
    }

    func test_d_selectionChange() async throws {
        let (store, _) = try await loadedStore()
        let model = AppModel()
        // Collapsed tree; click folder rows.
        do {
            let box = SelectionBox()
            let host = Host(Hosted(store: store, box: box, model: model))
            host.draw()
            let folders = store.tree.folders.prefix(12).map { MonitorSelection.folder(projectID: $0.id) }
            var samples: [Double] = []
            for (i, next) in folders.enumerated() {
                if i == 0 { Self.breakdown = [] }
                let before = box.renders
                let clock = ContinuousClock()
                let start = clock.now
                box.selection = next
                let total = Self.settle(host) { box.renders > before }
                if i >= 2 { samples.append(total) }
                _ = clock.now - start
            }
            Self.report("d.select.folder(collapsed tree)", samples)
            Self.reportBreakdown("d.select.folder")
            host.close()
        }
        // A session selected, its folder revealed; then arrow through the
        // sessions in the biggest folder.
        do {
            let biggest = try XCTUnwrap(store.tree.folders.max { $0.sessions.count < $1.sessions.count })
            let sessions = biggest.sessions.map { MonitorSelection.session(entryID: $0.entryId) }
            let box = SelectionBox()
            box.selection = sessions[0]
            let host = Host(Hosted(store: store, box: box, model: model))
            host.draw()
            _ = Self.settle(host) { true }
            var samples: [Double] = []
            for next in sessions.dropFirst().prefix(12) {
                let before = box.renders
                box.selection = next
                samples.append(Self.settle(host) { box.renders > before })
            }
            Self.report("d.select.session(one folder of \(sessions.count) open)", samples)
            host.close()
        }
        // Everything open (idle filter).
        do {
            await store.showIdleOnly(true)
            let box = SelectionBox()
            let host = Host(Hosted(store: store, box: box, model: model))
            host.draw()
            _ = Self.settle(host) { true }
            let sessions = store.tree.allSessions.prefix(14).map { MonitorSelection.session(entryID: $0.entryId) }
            var samples: [Double] = []
            for (i, next) in sessions.enumerated() {
                let before = box.renders
                box.selection = next
                let t = Self.settle(host) { box.renders > before }
                if i >= 2 { samples.append(t) }
            }
            Self.report("d.select.session(all \(store.tree.allSessions.count) open)", samples)
            host.close()
        }
    }

    func test_e_reloadChangingOneEntry() async throws {
        let (store, client) = try await loadedStore()
        let model = AppModel()
        let box = SelectionBox()
        let host = Host(Hosted(store: store, box: box, model: model))
        host.draw()
        let clock = ContinuousClock()
        var loads: [Double] = []
        var redraws: [Double] = []
        for bump in 1...10 {
            let r = try Self.replies(bump: bump)
            client.replaceReply("list_pending", with: r.pending)
            let before = box.renders
            let start = clock.now
            await store.load()
            loads.append(Self.ms(clock.now - start))
            redraws.append(Self.settle(host) { box.renders > before })
        }
        Self.report("e.reload.load(awaited, includes off-main derive)", loads)
        Self.report("e.reload.redraw(main thread, collapsed tree)", redraws)
        Self.reportBreakdown("e.reload.collapsed")
        // Same, with every folder open.
        await store.showIdleOnly(true)
        _ = Self.settle(host) { true }
        loads = []
        redraws = []
        for bump in 11...20 {
            let r = try Self.replies(bump: bump)
            client.replaceReply("list_pending", with: r.pending)
            let before = box.renders
            let start = clock.now
            await store.load()
            loads.append(Self.ms(clock.now - start))
            redraws.append(Self.settle(host) { box.renders > before })
        }
        Self.report("e.reload.redraw(main thread, all open)", redraws)
        host.close()
    }

    // MARK: (f) per-row work in a body pass

    /// The calls one tree body pass makes for each row, timed one at a time,
    /// so count x unit cost can be checked against (d).
    func test_f_perRowCalls() async throws {
        let (store, _) = try await loadedStore()
        let folders = store.tree.folders
        let sessions = store.tree.allSessions
        let words = try XCTUnwrap(store.words)
        let labels = store.disclosure?.folderModeLabels ?? [:]
        print("PERF f.counts folders=\(folders.count) sessions=\(sessions.count) "
            + "foldersWithContributable=\(folders.filter { $0.contributableCount != nil }.count)")
        Self.time("f.groupOffer x\(folders.count) (FFI groupControl + withheldLine)") {
            for f in folders { _ = store.groupOffer(f) }
        }
        Self.time("f.groupControl FFI x10000") {
            for i in 0..<10_000 { _ = TCContributionEligibility.groupControl(pending: Int64(i % 9), contributable: Int64(i % 5)) }
        }
        Self.time("f.withheldLine FFI x10000") {
            for i in 0..<10_000 { _ = TCContributionEligibility.withheldLine(withheld: Int64(i % 4)) }
        }
        Self.time("f.folderSub+submitTitle+submitHelp x\(folders.count)") {
            for f in folders {
                _ = TracesTreeView.folderSub(f, words: words, modeLabels: labels)
                _ = TracesTreeView.submitTitle(f.contributableCount ?? 0, words: words)
                _ = TracesTreeView.submitHelp(nil, words: words)
            }
        }
        Self.time("f.unresolvedBucketNote x\(folders.count)") {
            for _ in folders { _ = ProjectCopy.unresolvedBucketNote }
        }
        Self.time("f.sessionRow strings x\(sessions.count) (when+sub+size+flag+lines)") {
            for e in sessions {
                let lines = store.lines(e)
                _ = TracesTreeView.when(e)
                _ = TracesTreeView.sub(e, held: words.held, ineligible: lines.ineligible,
                                       attestation: lines.attestation, words: words)
                _ = TracesTreeView.flag(e, ineligible: lines.ineligible != nil, attestation: lines.tone)
                _ = TracesTreeView.toolName(e)
            }
        }
        Self.time("f.when x\(sessions.count)") { for e in sessions { _ = TracesTreeView.when(e) } }
        Self.time("f.size x\(sessions.count)") { for e in sessions { _ = TracesTreeView.size(e) } }
        Self.time("f.rowID x\(folders.count + sessions.count)") {
            for f in folders { _ = TracesTreeView.rowID(.folder(projectID: f.id)) }
            for e in sessions { _ = TracesTreeView.rowID(.session(entryID: e.entryId)) }
        }
        // MonitorWindowView reads these on every body pass of the window.
        Self.time("f.window.selectedSession") { _ = store.selectedSession(.session(entryID: sessions.last!.entryId)) }
        Self.time("f.window.folderOfSession(reveal)") {
            _ = TracesTreeView.folder(of: sessions.last!.entryId, in: store.tree)
        }
    }
}
