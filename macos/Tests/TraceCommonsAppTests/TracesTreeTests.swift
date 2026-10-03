import TCBridge
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R6 of #1173: the Traces tree is built only from what the core reports,
/// against C1's sample sets.
@MainActor
final class TracesTreeTests: XCTestCase {
    /// The tools the core says are read from their usual folder while
    /// unset (`unset_scans_conventional`): Claude Code and Codex.
    static let scansWhenUnset: Set<SourceKind> = [.claudeCode, .codex]

    private func tree(_ set: SampleDaemonClient.SampleSet) async throws -> TracesTree {
        let client = SampleDaemonClient(set)
        return TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: try? await client.settings(),
            scansWhenUnset: Self.scansWhenUnset)
    }

    /// Every waiting session appears in the tree exactly once.
    func test_everySessionIsInTheTreeOnce() async throws {
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let client = SampleDaemonClient(set)
            let pending = try await client.listPending(projectId: nil).map(\.entryId)
            let drawn = try await tree(set).allSessions.map(\.entryId)
            XCTAssertEqual(drawn.sorted(), pending.sorted(), "\(set)")
        }
    }

    /// A folder sits under the tool most of its sessions came from.
    func test_aFolderSitsUnderItsMajorityTool() async throws {
        let tree = try await tree(.normalDay)
        let claude = try XCTUnwrap(tree.tools.first { $0.kind == .claudeCode })
        // api has two Claude Code sessions and one Codex session.
        XCTAssertTrue(claude.folders.contains { $0.label == "api" })
        XCTAssertFalse(tree.tools.first { $0.kind == .codex }?.folders.contains { $0.label == "api" } ?? false)
    }

    /// Unset is never drawn as off. A tool the core reads from its usual
    /// folder while unset (Claude Code, Codex) is always drawn, nothing
    /// waiting or not, because it IS being read; a tool that opens nothing
    /// while unset, with nothing waiting, is not drawn.
    func test_unsetIsNeverOff() async throws {
        let tree = try await tree(.normalDay)
        XCTAssertNil(tree.tools.first { $0.kind == .opencode }, "opencode is unset with nothing waiting")
        XCTAssertNil(tree.tools.first { $0.kind == .cline }, "cline is unset with nothing waiting")
        // The K2 recording leaves Gemini CLI unset: it is never drawn off.
        XCTAssertNotEqual(tree.tools.first { $0.kind == .geminiCli }?.mode, .off)
        XCTAssertFalse(tree.tools.contains { $0.mode == .off }, "nothing in normalDay is set off")
        XCTAssertEqual(TracesTree.SourceMode("unset"), .unset)
        XCTAssertEqual(TracesTree.SourceMode(nil), .unset)

        // Unset with nothing waiting: drawn for a tool scanned while unset.
        XCTAssertTrue(TracesTree.drawsTool(.claudeCode, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
        XCTAssertTrue(TracesTree.drawsTool(.codex, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
        XCTAssertFalse(TracesTree.drawsTool(.cline, mode: .unset, hasFolders: false, scansWhenUnset: Self.scansWhenUnset))
    }

    /// The set comes from the core's source copy, and is the two tools
    /// this file's fixture assumes.
    func test_theCoreSaysWhichToolsAreReadWhileUnset() {
        XCTAssertEqual(TracesStore.scansWhenUnset, Self.scansWhenUnset)
    }

    /// Unreadable settings are unknown, not unset: no switch, and the
    /// tools that may be being read are still drawn, so the tab can say the
    /// declaration could not be confirmed rather than drop them.
    func test_unreadableSettingsAreUnknownNotUnset() async throws {
        let tree = TracesTree.build(entries: [], projects: [], settings: nil, scansWhenUnset: Self.scansWhenUnset)
        XCTAssertEqual(Set(tree.tools.map(\.kind)), [.claudeCode, .codex])
        XCTAssertTrue(tree.tools.allSatisfy { $0.mode == .unknown })
    }

    /// Ignored folders are left out, and their sessions with them.
    func test_ignoredFoldersAreLeftOut() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "quiet", projectPath: "/x", mode: .ignore)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertTrue(tree.tools.isEmpty)
        XCTAssertTrue(tree.unplaced.isEmpty)
    }

    /// A folder with nothing waiting cannot be placed until the core says
    /// which tool it belongs to (K11): it is listed on its own.
    func test_aFolderWithNothingWaitingIsUnplaced() throws {
        let project = ProjectRow(projectId: "p1", projectLabel: "docs", projectPath: "/x", mode: .ask)
        let tree = TracesTree.build(entries: [], projects: [project], settings: nil, scansWhenUnset: [])
        XCTAssertEqual(tree.unplaced.map(\.label), ["docs"])
    }

    /// The tab's words are the core's: the store holds the decoded export,
    /// and the inspector's row labels are its fields, not Swift strings.
    func test_theTabsWordsComeFromTheCore() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        let words = try XCTUnwrap(store.words)
        XCTAssertEqual(words, MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let client = SampleDaemonClient(.normalDay)
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        let keys = SessionInspectorView.rows(entry, nil, words: words).map(\.label)
        XCTAssertEqual(keys, [words.tool, words.folder, words.started, words.length, words.prompts,
                              words.size, words.sends, words.marks, words.unsure])
        // A failure is said in the core's line, not the error's label.
        XCTAssertEqual(words.line(for: .unreachable), words.coreUnreachable)
    }

    /// The core being down is a failure the tab says, not an empty tree.
    func test_coreDownIsAFailureNotAnEmptyTree() async {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        await store.load()
        XCTAssertEqual(store.phase, .failed(.unreachable))
    }

    func test_theStoreLoadsTheSampleDay() async {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(store.phase, .loaded)
        XCTAssertEqual(store.tree.allSessions.count, 3)
    }
    /// Left and right find the selected session's folder (and tool) to
    /// collapse and expand; a session not in the tree finds nothing.
    func test_arrowKeysFindTheSelectedSessionsFolder() async throws {
        let built = try await tree(.normalDay)
        let session = try XCTUnwrap(built.allSessions.first)
        let path = try XCTUnwrap(TracesTreeView.path(to: session.entryId, in: built))
        let folders: [TracesTree.FolderNode] = built.tools.flatMap { $0.folders } + built.unplaced
        let folder = try XCTUnwrap(folders.first { $0.id == path.folder })
        XCTAssertTrue(folder.sessions.contains { $0.entryId == session.entryId })
        XCTAssertNil(TracesTreeView.path(to: "no-such-entry", in: built))
    }
}

/// R7: the badge and the review actions, against C1's sample sets.
@MainActor
final class TracesReviewTests: XCTestCase {
    /// The badge is `decisions_owed`; unknown stays unknown, never a count
    /// derived from the queue.
    func test_theBadgeIsDecisionsOwedOrUnknown() async {
        // The armed set owes one decision with three sessions waiting: the
        // badge is the core's count, not the queue's depth.
        let armed = TracesStore(client: SampleDaemonClient(.armedFolder))
        await armed.load()
        XCTAssertEqual(armed.decisionsOwed, 1)
        XCTAssertEqual(armed.status?.queueDepth, 3)

        let unknown = TracesStore(client: SampleDaemonClient(.unknownCounts))
        await unknown.load()
        XCTAssertNotNil(unknown.status, "status was read")
        XCTAssertNil(unknown.decisionsOwed, "decisions_owed absent is unknown")
        XCTAssertFalse(unknown.tree.allSessions.isEmpty, "and the queue is not where a count comes from")

        let down = TracesStore(client: SampleDaemonClient(.coreDown))
        await down.load()
        XCTAssertNil(down.decisionsOwed)
    }

    /// Keep offers its undo, and the undo withdraws it. The sample core's
    /// queue does not move, so what is checked is that each answer was
    /// taken and the tree reloaded from the core, with nothing refused.
    func test_keepOffersAnUndo() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.keep, on: id)
        XCTAssertEqual(store.lastKept, id)
        XCTAssertNil(store.actionError?.error)
        XCTAssertEqual(store.phase, .loaded)
        await store.perform(.undoKeep, on: id)
        XCTAssertNil(store.lastKept)
        XCTAssertNil(store.actionError?.error)
        XCTAssertTrue(store.acting.isEmpty)
        XCTAssertTrue(store.tree.allSessions.contains { $0.entryId == id })
    }

    /// Contribute keeps the core's answer: its toast, and an Undo that is
    /// `cancel`. The sample core refuses the cancel as not cancelable, and
    /// that refusal is kept, not shown as an undone contribution.
    func test_contributeShowsTheCoresToastAndUndoIsCancel() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.contribute, on: id)
        let contributed = try XCTUnwrap(store.lastContributed)
        XCTAssertEqual(contributed.entryId, id)
        XCTAssertTrue(contributed.toast.offerUndo)
        XCTAssertFalse(contributed.toast.line.isEmpty)

        await store.perform(.undoContribute, on: id)
        XCTAssertEqual(store.actionError?.entryId, id)
        XCTAssertEqual(store.actionError?.error, .daemon(code: "bad_params", message: "not-cancelable"))
        XCTAssertNotNil(store.lastContributed, "a refused undo leaves the contribution standing")
    }

    /// A skipped approve throws `notApproved`: never success, and said in
    /// the toast's words, not as the error's fixed label.
    func test_aSkippedApproveIsNotSuccess() async throws {
        let client = SampleDaemonClient(.normalDay)
        let kept = try await client.listKept()
        let id = try XCTUnwrap(kept.first?.entryId)
        let store = TracesStore(client: client)
        await store.load()
        await store.perform(.contribute, on: id)
        XCTAssertNil(store.lastContributed)
        let refused = try XCTUnwrap(store.actionError)
        XCTAssertEqual(refused.error, .notApproved(reasonLabel: "not-pending"))
        let message = try XCTUnwrap(store.message(for: refused.error))
        XCTAssertNotEqual(message, refused.error.description)
        XCTAssertFalse(message.isEmpty)
    }

    /// Contribute arms only on a preview that pinned an enrollment, with the
    /// core's consent words in hand and the session eligible; the summary is
    /// the one asked for this session (`PreviewSlot`).
    func test_contributeArmsOnlyOnAnEnrolledPreviewWithConsent() throws {
        let consent = try XCTUnwrap(TracesStore(client: SampleDaemonClient(.normalDay)).consent)
        let calls = TracesStore.eligibilityCalls
        XCTAssertTrue(TracesStore.contributeArmed(enrolled: true, consent: consent, eligibility: nil, calls: calls))
        XCTAssertFalse(TracesStore.contributeArmed(enrolled: false, consent: consent, eligibility: nil, calls: calls))
        XCTAssertFalse(TracesStore.contributeArmed(enrolled: nil, consent: consent, eligibility: nil, calls: calls))
        XCTAssertFalse(TracesStore.contributeArmed(enrolled: true, consent: nil, eligibility: nil, calls: calls))
        XCTAssertFalse(TracesStore.contributeArmed(
            enrolled: true, consent: consent,
            eligibility: ContributionEligibility(state: "ineligible", reason: "attestation-missing"), calls: calls))

        // Another session's summary never arms this one.
        var slot = PreviewSlot()
        slot.begin("B")
        XCTAssertNil(slot.summary(for: "A")?.enrolled)
    }

    /// The badge has a text equivalent from the core, and caps at 99+ as
    /// the menu bar does.
    func test_theBadgeHasWordsAndACap() throws {
        XCTAssertEqual(GlassBadge.text(for: 3), "3")
        XCTAssertEqual(GlassBadge.text(for: 120), "\(MenuBarStatus.badgeCap)+")
        XCTAssertEqual(GlassBadge.text(for: nil), "—")
        XCTAssertEqual(GlassBadge.cap, MenuBarStatus.badgeCap)
        let unknown = try XCTUnwrap(MonitorWindowView.tracesDescription(nil))
        XCTAssertFalse(unknown.isEmpty)
        XCTAssertNil(MonitorWindowView.tracesDescription(0))
        XCTAssertEqual(MonitorWindowView.tracesDescription(3), TCCoreCopy.decisionsOwedText(3))
    }

    /// A refused action is kept against its session; nothing is applied.
    func test_aRefusedActionIsKept() async {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        await store.perform(.contribute, on: "e1")
        XCTAssertEqual(store.actionError?.entryId, "e1")
        XCTAssertEqual(store.actionError?.error, .unreachable)
        XCTAssertNil(store.lastKept)
    }
}

/// Review of #1183: a folder keeps its three modes, and ignoring one says
/// what the core actually cleared.
@MainActor
final class TracesFolderModeTests: XCTestCase {
    func test_foldersCarryTheModesTheyCanBeSetTo() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folders = store.tree.tools.flatMap(\.folders) + store.tree.unplaced
        XCTAssertFalse(folders.isEmpty)
        for folder in folders where folder.mode != nil {
            XCTAssertEqual(Set(folder.offerableModes), [.ask, .autoUpload, .ignore], folder.label)
        }
    }

    /// The core's `purged` is the authority. When it differs from the count
    /// the confirmation named, the difference is said in the core's words.
    func test_ignoringSaysWhenTheQueueChanged() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folder = try XCTUnwrap(store.tree.tools.flatMap(\.folders).first { !$0.sessions.isEmpty })
        // The sample core answers purged 0; the confirmation promised more.
        // The notice is the core's, naming the folder and both counts.
        XCTAssertGreaterThan(folder.sessions.count, 0)
        await store.setFolderMode(folder, .ignore, promised: folder.sessions.count)
        let notice = try XCTUnwrap(store.folderNotice)
        XCTAssertTrue(notice.contains(folder.label), notice)
        XCTAssertTrue(notice.contains("0 "), notice)
        XCTAssertTrue(notice.contains(String(folder.sessions.count)), notice)

        // When they agree, nothing is said.
        await store.setFolderMode(folder, .ignore, promised: 0)
        XCTAssertNil(store.folderNotice)
    }

    /// The tool switch writes the source declaration through `setSource`,
    /// and the tree reloads from the core's answer.
    func test_theToolSwitchWritesTheSourceDeclaration() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        await store.setSource(.codex, .off)
        XCTAssertEqual(store.phase, .loaded)
        XCTAssertTrue(store.writing.isEmpty)

        // A watch with no folder is not an answer. The refusal is kept beside
        // the tool's row, not drawn as the core being down, and a reload
        // does not clear it.
        await store.setSource(.codex, .watch(path: ""))
        XCTAssertEqual(store.phase, .loaded)
        XCTAssertNotNil(store.writeErrors[SourceKind.codex.rawValue])
        await store.load()
        XCTAssertNotNil(store.writeErrors[SourceKind.codex.rawValue])
        // The next write to that row starts clean.
        await store.setSource(.codex, .off)
        XCTAssertNil(store.writeErrors[SourceKind.codex.rawValue])
    }

    /// A refused write keeps its error and leaves the tree as the core has it.
    func test_aRefusedModeWriteKeepsItsError() async throws {
        let store = TracesStore(client: SampleDaemonClient(.coreDown))
        let folder = TracesTree.FolderNode(id: "p1", label: "docs", mode: .ask, offerableModes: [.ask, .ignore], sessions: [])
        await store.setFolderMode(folder, .ignore, promised: 0)
        XCTAssertEqual(store.phase, .failed(.unreachable))
        XCTAssertTrue(store.writing.isEmpty)
    }

}

/// The #1183 review's row findings: what a row says, beyond its colour.
@MainActor
final class TracesRowWordsTests: XCTestCase {
    /// A sample entry, held for a second look or carrying `extra` fields.
    private func entry(_ extra: String = "", held: Bool = false) throws -> DaemonData.QueueEntry {
        var fields: [String: Any] = [
            "state": held ? "held" : "pending",
            "reason_label": held ? DaemonData.ReasonLabel.secondLookReviewRequired : NSNull(),
        ]
        if !extra.isEmpty {
            let more = try XCTUnwrap(JSONSerialization.jsonObject(with: Data("{\(extra)}".utf8)) as? [String: Any])
            fields.merge(more) { _, new in new }
        }
        return try recordedEntry(fields)
    }

    /// The unresolvable bucket is drawn under its shared name, never the
    /// `unknown-project` slug, is never offered automatic, and says why.
    func test_theBucketIsNamedAndExplained() throws {
        let bucket = ProjectRow(
            projectId: "bucket", projectLabel: "unknown-project", projectPath: "", mode: .ask, isUnresolvedBucket: true)
        let tree = TracesTree.build(entries: [], projects: [bucket], settings: nil, scansWhenUnset: [])
        let folder = try XCTUnwrap(tree.unplaced.first)
        XCTAssertEqual(folder.label, ProjectCopy.unresolvedBucketLabel)
        XCTAssertTrue(folder.isBucket)
        XCTAssertFalse(folder.offerableModes.contains(.autoUpload))
        let view = TracesTreeView(store: TracesStore(client: SampleDaemonClient(.empty)), selection: .constant(""))
        XCTAssertEqual(view.folderNotes(folder), [ProjectCopy.unresolvedBucketNote])
    }

    /// An armed folder says the core's words for the disclosure the daemon
    /// chose for it; a folder that is not armed says none.
    func test_anArmedFolderSaysWhatAppliesToIt() async throws {
        let store = TracesStore(client: SampleDaemonClient(.armedFolder))
        await store.load()
        let folders = store.tree.tools.flatMap(\.folders) + store.tree.unplaced
        let armed = try XCTUnwrap(folders.first { $0.mode == .autoUpload })
        XCTAssertEqual(armed.disclosure, "patterns_only")
        let words = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "patterns_only")))
        XCTAssertEqual(store.disclosureLines(armed.disclosure), words.lines)
        let view = TracesTreeView(store: store, selection: .constant(""))
        XCTAssertEqual(view.folderNotes(armed), words.lines)
        for folder in folders where folder.mode != .autoUpload {
            XCTAssertNil(folder.disclosure, folder.label)
            XCTAssertTrue(view.folderNotes(folder).isEmpty, folder.label)
        }
        XCTAssertTrue(store.disclosureLines("scrubbed").isEmpty)
    }

    /// A held session says the core's word for it, so the amber flag is
    /// never the only signal.
    func test_aHeldSessionSaysSoInWords() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let held = try entry(held: true)
        XCTAssertEqual(TracesTreeView.flag(held), .ask)
        XCTAssertEqual(TracesTreeView.sub(held, held: words.held, ineligible: nil)?.hasPrefix(words.held), true)
        let plain = try entry()
        XCTAssertNil(TracesTreeView.flag(plain))
        XCTAssertEqual(TracesTreeView.sub(plain, held: words.held, ineligible: nil), TracesTreeView.measures(plain))
    }

    /// A session that cannot be contributed as it stands says the core's
    /// sentence on its row and is flagged; an eligible one, or one with no
    /// eligibility question, says nothing extra.
    func test_anIneligibleSessionLooksDifferent() throws {
        let store = TracesStore(client: SampleDaemonClient(.empty))
        let ineligible = try entry(#""eligibility":"ineligible_configuration","eligibility_reason":"capture_off""#)
        let line = try XCTUnwrap(store.ineligibleLine(ineligible))
        XCTAssertFalse(line.isEmpty)
        XCTAssertEqual(TracesTreeView.flag(ineligible, ineligible: true), .ask)
        XCTAssertNotNil(store.eligibilityValue(ineligible))

        let eligible = try entry(#""eligibility":"eligible""#)
        XCTAssertNil(store.ineligibleLine(eligible))
        XCTAssertNil(store.ineligibleLine(try entry()))
        XCTAssertNil(store.eligibilityValue(try entry()))
    }

    /// The inspector carries eligibility and attestation in the core's
    /// sentences, under the core's labels.
    func test_theInspectorSaysEligibilityAndAttestation() throws {
        let store = TracesStore(client: SampleDaemonClient(.empty))
        let words = try XCTUnwrap(store.words)
        let ineligible = try entry(#""eligibility":"ineligible_permanent""#)
        let rows = SessionInspectorView.rows(
            ineligible, nil, words: words,
            eligibility: store.eligibilityValue(ineligible), attestation: store.attestationValue(ineligible))
        XCTAssertTrue(rows.contains { $0.label == words.eligibility })
        XCTAssertTrue(rows.contains { $0.label == words.attestation })
        let bare = SessionInspectorView.rows(ineligible, nil, words: words)
        XCTAssertFalse(bare.contains { $0.label == words.eligibility })
    }

    /// The debug window's sample set: unset is the default, a set's name is
    /// that set, and anything else falls back and says it did.
    func test_aMisspelledSampleIsNeverSilent() {
        XCTAssertEqual(MonitorWindowView.sampleChoice(nil).set, .normalDay)
        XCTAssertFalse(MonitorWindowView.sampleChoice(nil).unknown)
        XCTAssertFalse(MonitorWindowView.sampleChoice("").unknown)
        XCTAssertEqual(MonitorWindowView.sampleChoice("busyQueue").set, .busyQueue)
        XCTAssertFalse(MonitorWindowView.sampleChoice("busyQueue").unknown)
        XCTAssertEqual(MonitorWindowView.sampleChoice("busyqueue").set, .normalDay)
        XCTAssertTrue(MonitorWindowView.sampleChoice("busyqueue").unknown)
    }
}

/// The #1184 review's open findings: a count kept after the core went
/// away, the queue's safeguards, and the badge's second-look pairing.
@MainActor
final class TracesQueueStateTests: XCTestCase {
    /// Answers every call as a daemon that has gone away.
    private final class GoneTransport: DaemonTransport {
        func call(_ method: String, params paramsJSON: String) -> String {
            #"{"error":{"code":"unavailable","message":"daemon-disconnected"}}"#
        }
    }

    /// A disconnect finishes every open stream, so nothing keeps waiting on
    /// a daemon that is not there.
    func test_aDisconnectEndsTheEventStreams() async {
        let live = LiveDaemonClient(transport: GoneTransport())
        let stream = live.events()
        let ended = Task { for await _ in stream {}; return true }
        live.finishEvents()
        let finished = await ended.value
        XCTAssertTrue(finished)
    }

    /// The badge reads unknown once the core has gone, never its last count.
    func test_aLostCoreLeavesNoCount() async {
        let store = TracesStore(client: SampleDaemonClient(.armedFolder))
        await store.load()
        XCTAssertEqual(store.decisionsOwed, 1)
        store.lost()
        XCTAssertNil(store.decisionsOwed)
        XCTAssertEqual(store.phase, .failed(.unreachable))
    }

    /// Following a live client, the stream ending is a lost core.
    func test_theStreamEndingIsALostCore() async {
        let live = LiveDaemonClient(transport: GoneTransport())
        let store = TracesStore(client: live)
        let running = Task { await store.run() }
        // Let `run` load and open its stream before the daemon goes.
        while store.phase == .loading { await Task.yield() }
        for _ in 0..<50 { await Task.yield() }
        live.finishEvents()
        await running.value
        XCTAssertNil(store.status)
        XCTAssertEqual(store.phase, .failed(.unreachable))
    }

    /// The sample day's status, with `edit` applied to its JSON object.
    private func status(_ edit: (inout [String: Any]) -> Void = { _ in }) async throws -> DaemonData.Status {
        let read = try await SampleDaemonClient(.normalDay).status()
        let wire = try XCTUnwrap(TracesStore.wire(read))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(wire.utf8)) as? [String: Any])
        edit(&object)
        let data = try JSONSerialization.data(withJSONObject: object)
        return try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: data)
    }

    private static func spend(_ object: inout [String: Any]) {
        var budget = object["daily_budget"] as? [String: Any] ?? [:]
        budget["blocked"] = true
        budget["blocked_entries"] = 2
        object["daily_budget"] = budget
    }

    /// A spent budget says so, in the words the main window uses for it; a
    /// healthy queue says nothing.
    func test_aSpentBudgetIsSaid() async throws {
        let healthy = try await status()
        XCTAssertTrue(TracesStore.safeguards(healthy).isEmpty)
        let spent = try await status(Self.spend)
        XCTAssertEqual(TracesStore.safeguards(spent).map(\.title), [DailyBudgetCopy.title])
        XCTAssertTrue(TracesStore.safeguards(nil).isEmpty)
    }

    /// The health label is said when nothing more specific already says it,
    /// and not twice when the budget does.
    func test_theHealthLabelIsNotSaidTwice() async throws {
        let full = try await status { $0["health"] = ["last_error_label": "queue-full"] }
        XCTAssertEqual(TracesStore.safeguards(full).map(\.title), [HealthCopy.core(label: "queue-full", maxQueueEntries: nil).title])
        let capped = try await status {
            $0["health"] = ["last_error_label": "daily-cap-reached"]
            Self.spend(&$0)
        }
        XCTAssertEqual(TracesStore.safeguards(capped).map(\.title), [DailyBudgetCopy.title])
    }

    /// The badge pairs with the queue shield: something waiting that is
    /// worth a second look adds the core's words to its text equivalent.
    func test_theBadgeSaysWhenSomethingIsWorthASecondLook() throws {
        let plain = try recordedEntry(["second_look": [String]()])
        let flagged = try recordedEntry(["entry_id": "00000000-0000-4000-8000-000000000002", "second_look": ["nothing-matched"]])
        XCTAssertEqual(TracesStore.shield([]), .clear)
        XCTAssertEqual(TracesStore.shield([plain]), .waiting)
        XCTAssertEqual(TracesStore.shield([plain, flagged]), .attention)

        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let said = try XCTUnwrap(MonitorWindowView.tracesDescription(2, shield: .attention, secondLook: words.secondLookWaiting))
        XCTAssertTrue(said.hasSuffix(words.secondLookWaiting), said)
        XCTAssertEqual(
            MonitorWindowView.tracesDescription(2, shield: .waiting, secondLook: words.secondLookWaiting),
            TCCoreCopy.decisionsOwedText(2))
    }
}

/// The first pending entry of the K2 `normalDay` recording, with `fields`
/// replacing its own. Main's #1204 replaced the hand-written sample
/// builders with recorded daemon replies.
func recordedEntry(_ fields: [String: Any] = [:]) throws -> DaemonData.QueueEntry {
    let reply = try XCTUnwrap(SampleDaemonData.reply("list_pending", in: .normalDay))
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
    var row = try XCTUnwrap((object["pending"] as? [[String: Any]])?.first)
    row.merge(fields) { _, new in new }
    let data = try JSONSerialization.data(withJSONObject: row)
    return try DaemonDataDecoding.decoder().decode(DaemonData.QueueEntry.self, from: data)
}

/// Submit all and Submit all as on a folder row (B9), against the sample core.
@MainActor
final class TracesFolderSubmitTests: XCTestCase {
    private func loaded() async throws -> (TracesStore, TracesTree.FolderNode) {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let folder = try XCTUnwrap(
            (store.tree.tools.flatMap(\.folders) + store.tree.unplaced).first { !$0.sessions.isEmpty })
        return (store, folder)
    }

    func test_aFolderCarriesTheCoresCountsFromItsProjectRow() async throws {
        let client = SampleDaemonClient(.normalDay)
        let projects = try await client.listProjects().projects
        let tree = TracesTree.build(
            entries: try await client.listPending(projectId: nil), projects: projects, settings: nil,
            scansWhenUnset: [])
        for folder in tree.tools.flatMap(\.folders) + tree.unplaced {
            let row = projects.first { $0.projectId == folder.id }
            XCTAssertEqual(folder.pendingCount, row?.pendingCount, folder.id)
            XCTAssertEqual(folder.contributableCount, row?.contributableCount, folder.id)
        }
    }

    func test_contributingAFolderKeepsTheCoresToastAndUndoClearsIt() async throws {
        let (store, folder) = try await loaded()
        await store.contributeFolder(folder, verdict: .worked)
        let contributed = try XCTUnwrap(store.lastContributedFolder)
        XCTAssertEqual(contributed.projectId, folder.id)
        XCTAssertFalse(contributed.toast.line.isEmpty)
        XCTAssertNil(store.lastContributed, "a newer decision ends the single-session undo")
        await store.undoFolder(folder.id)
        XCTAssertNil(store.lastContributedFolder)
    }

    /// One undo slot, as the legacy queue had: a session contributed after a
    /// folder ends the folder's undo, so its cancel cannot reach that session.
    func test_aSessionContributeEndsTheFoldersUndo() async throws {
        let (store, folder) = try await loaded()
        await store.contributeFolder(folder, verdict: nil)
        XCTAssertNotNil(store.lastContributedFolder)
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.contribute, on: id)
        XCTAssertNil(store.lastContributedFolder)
        XCTAssertNotNil(store.lastContributed)
    }

    func test_attachClearsTheFolderNotice() async throws {
        let (store, _) = try await loaded()
        store.attach(nil)
        XCTAssertNil(store.folderNotice)
        XCTAssertNil(store.lastContributedFolder)
    }

    func test_aFolderRefusalIsKeptBesideTheFolder() async throws {
        let (store, _) = try await loaded()
        let ghost = TracesTree.FolderNode(id: "proj_does_not_exist", label: "ghost", mode: nil, sessions: [])
        await store.contributeFolder(ghost, verdict: nil)
        XCTAssertEqual(store.writeErrors[ghost.id], .daemon(code: "bad_params", message: "project-id-unrecognized"))
        XCTAssertNil(store.lastContributedFolder)
    }

    func test_noFolderIsSubmittedWithoutAClientOrForAnIgnoredOne() async throws {
        let (store, folder) = try await loaded()
        XCTAssertTrue(store.mayContributeFolder(folder))
        let ignored = TracesTree.FolderNode(id: folder.id, label: folder.label, mode: .ignore, sessions: folder.sessions)
        XCTAssertFalse(store.mayContributeFolder(ignored))
        store.attach(nil)
        XCTAssertFalse(store.mayContributeFolder(folder))
        await store.contributeFolder(folder, verdict: nil)
        XCTAssertNil(store.lastContributedFolder)
    }
}
