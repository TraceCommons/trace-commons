import Foundation
import Observation
import TCBridge
import TCShellCore

/// The Traces tab's data (R6 of #1173), read through `DaemonDataClient`.
///
/// It reloads on every event that can change the tree. The tree is redrawn
/// only from a successful read: a failed refresh keeps the last tree and
/// says the core is not answering, rather than drawing an empty one.
@MainActor
@Observable
final class TracesStore {
    enum Phase: Equatable {
        case loading
        case loaded
        case failed(DaemonDataError)
    }

    private(set) var phase: Phase = .loading
    private(set) var tree = TracesTree(tools: [], unplaced: [])
    /// The `set_project_mode` write in flight, by folder id.
    private(set) var writing: Set<String> = []
    /// The last `status` read; nil when it has not been read or failed. The
    /// badge is its `decisions_owed`, never `queue_depth`.
    private(set) var status: DaemonData.Status?
    /// The last `tool_destinations` read: the core's verdict on where each
    /// tool's sessions go. Nil when unread or unreadable, and then the map
    /// draws no session flow at all.
    private(set) var destinations: DaemonData.ToolDestinations?
    /// A review action (R7) in flight, by entry id.
    private(set) var acting: Set<String> = []
    /// The session last kept on this Mac, while its undo is offered.
    private(set) var lastKept: String?
    /// The last review action the core refused, by entry id.
    private(set) var actionError: (entryId: String, error: DaemonDataError)?

    /// The Traces badge: decisions owed, unknown (a dash) when the core did
    /// not say, never a count derived from the queue.
    var decisionsOwed: Int? { status?.decisionsOwed }
    /// The last write the core refused, by folder id (or tool id for a
    /// source declaration). It stays beside that row until the next write to
    /// it: a reload does not clear it, and it never stands in for the core
    /// being down.
    private(set) var writeErrors: [String: DaemonDataError] = [:]

    /// The app's live client (`AppModel.daemonData`), attached by the
    /// window when the daemon starts; nil while it is not running.
    private(set) var client: (any DaemonDataClient)?
    /// The last folder change whose result differed from what the
    /// confirmation promised, in the core's words
    /// (`tc_project_ignore_reconciled_text`).
    private(set) var folderNotice: String?
    /// Each load's number; a load that finishes after a newer one started is
    /// dropped, so an older read never overwrites a newer tree.
    private var generation = 0

    /// The tab's words, from the core (`tc_monitor_traces_copy_json`),
    /// decoded once rather than on every redraw. Nil leaves a label out
    /// rather than writing one here.
    let words: MonitorTracesCopy? = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())
    /// The consent gate's words, from the core, decoded once rather than on
    /// every redraw. Without them Contribute stays disarmed: the shell never
    /// words consent itself.
    let consent: ConsentCopy? = TCConsentCopy.copyJSON().flatMap(ConsentCopy.decode(fromJSON:))

    /// A Contribute the core took, while its hold lets it be taken back:
    /// the core's toast for it, and whether Undo (`cancel`) is offered.
    struct Contributed: Equatable {
        let entryId: String
        let toast: SubmitToast
    }

    /// The last contribution, until it is undone or another is made.
    private(set) var lastContributed: Contributed?

    /// A folder's Submit all the core took, while its hold lets it be taken
    /// back (`cancel` with the project id).
    struct ContributedFolder: Equatable {
        let projectId: String
        let toast: SubmitToast
    }

    private(set) var lastContributedFolder: ContributedFolder?

    /// The sample set drawn, in a debug build over sample data; nil over
    /// the daemon. `sampleUnknown` is a `TRACE_COMMONS_SAMPLE` that named no
    /// set, so the fallback is never silent.
    private(set) var sample: String?
    private(set) var sampleUnknown: Bool

    init(client: (any DaemonDataClient)?, sample: String? = nil, sampleUnknown: Bool = false) {
        self.client = client
        self.sample = sample
        self.sampleUnknown = sampleUnknown
    }

    /// Follows a new client (or none). Nothing the old one reported is
    /// drawn or acted on: the tree empties, the badge and routes read
    /// unknown, and the tab is loading until the new client answers; a load
    /// still in flight from the old one is dropped. (A failed read from the
    /// same client still keeps the last tree.)
    func attach(_ client: (any DaemonDataClient)?) {
        self.client = client
        generation += 1
        phase = .loading
        tree = TracesTree(tools: [], unplaced: [])
        status = nil
        destinations = nil
        lastContributedFolder = nil
    }

    /// Marks the data as a sample set in a debug build; nil over the daemon.
    func markSample(_ sample: String?, unknown: Bool) {
        self.sample = sample
        sampleUnknown = unknown
    }

    /// The attached client, or the core-down error when there is none, so
    /// every read and write without a daemon fails as an unreachable core.
    func attached() throws -> any DaemonDataClient {
        guard let client else { throw DaemonDataError.unreachable }
        return client
    }

    // MARK: The core's words for a row

    /// The core's words for an armed folder's disclosure, by the name the
    /// daemon chose (`automatic_disclosure`). Empty for a folder that is not
    /// armed, or a name the core does not know: nothing is said rather than
    /// a guess at which wording is true.
    func disclosureLines(_ disclosure: String?) -> [String] {
        guard let disclosure else { return [] }
        if let known = disclosureWords[disclosure] { return known }
        let lines = AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: disclosure))?.lines ?? []
        disclosureWords[disclosure] = lines
        return lines
    }

    @ObservationIgnored private var disclosureWords: [String: [String]] = [:]

    /// The eligibility and attestation sentences' fallbacks, from the core.
    let inferenceCopy: PrivateInferenceCopy? = PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? "")

    /// The shared eligibility table, as `AppModel` wires it.
    static let eligibilityCalls = EligibilityCalls(
        stateLine: { TCContributionEligibility.stateLine(state: $0) },
        stateTone: { TCContributionEligibility.stateTone(state: $0) },
        control: { TCContributionEligibility.control(state: $0) },
        reasonLine: { TCContributionEligibility.reasonLine(reason: $0) },
        withheldLine: { TCContributionEligibility.withheldLine(withheld: $0) },
        groupControl: { TCContributionEligibility.groupControl(pending: $0, contributable: $1) }
    )

    /// The shared attestation table, as `AppModel` wires it.
    static let attestationCalls = AttestationCalls(
        markLine: { TCAttestation.markLine(mark: $0) },
        markTone: { TCAttestation.markTone(mark: $0) },
        reasonLine: { TCAttestation.reasonLine(reason: $0) }
    )

    /// A queue entry's eligibility, as the shared table reads it; nil when
    /// the daemon sent none (eligibility does not apply).
    static func eligibility(_ entry: DaemonData.QueueEntry) -> ContributionEligibility? {
        entry.eligibility.map { ContributionEligibility(state: $0, reason: entry.eligibilityReason) }
    }

    /// The core's sentence for a session that cannot be contributed as it
    /// stands, with its reason; nil when it can, or eligibility does not
    /// apply. A row says this so an ineligible session never looks like one
    /// that can go.
    func ineligibleLine(_ entry: DaemonData.QueueEntry) -> String? {
        let eligibility = Self.eligibility(entry)
        guard eligibility != nil, !EligibilitySurface.offersContribute(eligibility, calls: Self.eligibilityCalls),
              let copy = inferenceCopy
        else { return nil }
        return EligibilitySurface.stateLine(eligibility, copy: copy, calls: Self.eligibilityCalls)
    }

    /// The inspector's eligibility value: the core's state sentence and its
    /// reason. Nil when eligibility does not apply, so no row is drawn.
    func eligibilityValue(_ entry: DaemonData.QueueEntry) -> String? {
        let eligibility = Self.eligibility(entry)
        guard let copy = inferenceCopy,
              let state = EligibilitySurface.stateLine(eligibility, copy: copy, calls: Self.eligibilityCalls)
        else { return nil }
        return [state, EligibilitySurface.reasonLine(eligibility, calls: Self.eligibilityCalls)]
            .compactMap { $0 }.joined(separator: " ")
    }

    /// The inspector's attestation value: the core's sentence for the mark
    /// and its reason. Every entry carries a mark, so this is nil only when
    /// the copy would not decode.
    func attestationValue(_ entry: DaemonData.QueueEntry) -> String? {
        guard let copy = inferenceCopy else { return nil }
        let mark = AttestationMark(
            mark: entry.attestation ?? "",
            reason: (entry.attestationReason?.isEmpty ?? true) ? nil : entry.attestationReason)
        return [
            AttestationSurface.markLine(mark, copy: copy, calls: Self.attestationCalls),
            AttestationSurface.reasonLine(mark, calls: Self.attestationCalls),
        ].compactMap { $0 }.joined(separator: " ")
    }

    /// The tools the core reads from their usual folder while unset, from
    /// its source copy. Empty if the copy is unavailable: then only tools
    /// with a declaration or something waiting are drawn.
    static let scansWhenUnset: Set<SourceKind> = {
        guard let copy = TCSourceChecks.settingsCopy() else { return [] }
        return Set(SourceKind.allCases.filter { copy.tools[$0.rawValue]?.unsetScansConventional == true })
    }()

    /// A tool row's sub-line: the core's sentence for its declaration, or
    /// its "could not be confirmed" sentence when settings were unreadable.
    static func sourceLine(_ tool: TracesTree.ToolNode) -> String? {
        guard let copy = TCSourceChecks.settingsCopy(), let entry = copy.tools[tool.kind.rawValue] else { return nil }
        guard let wire = tool.mode.wire else { return copy.unavailable }
        return TCSourceChecks.checkLine(tool: entry.key, sourceMode: wire)
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs. Call it from a view's `.task`: when the view goes, the task is
    /// cancelled and the stream with it. With no client the load fails as
    /// an unreachable core, and there is no stream to follow.
    func run() async {
        await load()
        guard let client else { return }
        for await event in client.events() {
            if Task.isCancelled { return }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                await load()
            case .digestDue, .previewReady, .inferenceCallAdded, .unknown:
                break
            }
        }
        guard !Task.isCancelled else { return }
        lost()
    }

    /// The event stream ended without this view going: the core went away
    /// (`LiveDaemonClient.finishEvents()`). The badge reads unknown rather
    /// than keeping its last count, and the tab says the core is not
    /// answering over the last tree it reported.
    func lost() {
        generation += 1
        status = nil
        destinations = nil
        phase = .failed(.unreachable)
    }

    func load() async {
        generation += 1
        let mine = generation
        do {
            let client = try attached()
            async let entries = client.listPending(projectId: nil)
            async let projects = client.listProjects()
            // Settings only decide the tool switches. Unreadable settings
            // are unknown: no switch, never off, and the row says so.
            async let settings = try? client.settings()
            async let status = try? client.status()
            async let destinations = try? client.toolDestinations()
            let built = TracesTree.build(
                entries: try await entries, projects: try await projects.projects, settings: await settings,
                scansWhenUnset: Self.scansWhenUnset)
            let read = await status
            let routes = await destinations
            guard mine == generation else { return }
            tree = built
            self.status = read
            self.destinations = routes
            phase = .loaded
        } catch {
            // An older failure never overwrites a newer read either.
            guard mine == generation else { return }
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "list_pending"))
            status = nil
            destinations = nil
        }
    }

    // MARK: Review (R7)

    enum ReviewAction: Equatable {
        case contribute, undoContribute, keep, undoKeep, dismiss
    }

    /// Whether Contribute is armed, as the review sheet decides it: the
    /// preview pinned an enrollment (`enrolled`, not merely a summary --
    /// and the caller passes the summary asked for THIS session), the core's
    /// consent words are in hand, and the session is one the shared table
    /// offers Contribute for. Asked again when it is pressed.
    static func contributeArmed(
        enrolled: Bool?, consent: ConsentCopy?, eligibility: ContributionEligibility?, calls: EligibilityCalls
    ) -> Bool {
        consent != nil && ReadGate.canContribute(hasPinnedPreview: enrolled == true)
            && EligibilitySurface.offersContribute(eligibility, calls: calls)
    }

    /// What the tab says for a refused action: a skipped approve in the
    /// submit toast's words, anything else in the core's line. Never the
    /// error's fixed label.
    func message(for error: DaemonDataError) -> String? {
        if case .notApproved(let reason) = error {
            return SubmitToast.render(
                approved: 0, redactions: 0, flagged: 0, skipped: reason.map { [$0] } ?? []
            ).line
        }
        return words?.line(for: error)
    }

    /// One review action on one session, then a reload. Nothing is applied
    /// optimistically: the tree redraws from the core's answer.
    func perform(_ action: ReviewAction, on entryId: String) async {
        guard !acting.contains(entryId) else { return }
        acting.insert(entryId)
        defer { acting.remove(entryId) }
        actionError = nil
        do {
            let client = try attached()
            switch action {
            case .contribute:
                // The core's answer is kept, not thrown away: its toast, and
                // the hold Undo can still reach. A skipped approve throws
                // `notApproved` and is said as a refusal, never as success.
                let response = try await client.approve(entryId: entryId)
                lastContributed = Contributed(entryId: entryId, toast: response.toast)
                // A newer decision ends the older Keep's undo.
                lastKept = nil
            case .undoContribute:
                try await client.cancel(entryId: entryId)
                lastContributed = nil
            case .keep:
                _ = try await client.keep(entryId: entryId)
                lastKept = entryId
            case .undoKeep:
                _ = try await client.undoKeep(entryId: entryId)
                lastKept = nil
            case .dismiss:
                try await client.dismiss(entryId: entryId)
                lastKept = nil
            }
        } catch {
            let error = error as? DaemonDataError ?? .undecodable(method: "\(action)")
            // A core that did not answer is the tab's state, and its last
            // count is no longer known.
            if case .unreachable = error { lost() }
            actionError = (entryId, error)
            return
        }
        await load()
    }

    // MARK: Queue safeguards

    /// A reason approved sessions are not moving, in the words the main
    /// window shows for the same status.
    struct Safeguard: Equatable {
        let title: String
        let body: String?
        /// The core's severity for a label line; budget, witness and
        /// gate-held lines are `.waiting`.
        let severity: HealthCopy.Severity
    }

    /// What the queue's safeguards say right now: a spent daily budget, a
    /// busy privacy witness, folders the automatic gate is holding, and the
    /// daemon's health label when none of those already says it. Each is
    /// drawn independently, as the main window draws them, because the
    /// daemon's one health slot can hide the others.
    var safeguards: [Safeguard] { Self.safeguards(status) }

    static func safeguards(_ status: DaemonData.Status?) -> [Safeguard] {
        guard let status else { return [] }
        var out: [Safeguard] = []
        var said: Set<String> = []
        if let budget = status.dailyBudget, budget.blocked == true {
            out.append(Safeguard(
                title: DailyBudgetCopy.title,
                body: DailyBudgetCopy.detail(blockedEntries: budget.blockedEntries ?? 0, resetsAt: budget.resetsAt),
                severity: .waiting))
            said.insert("daily-cap-reached")
        }
        if let capacity = status.witnessCapacity, (capacity.waitingSessions ?? 0) > 0,
            let wire = Self.wire(capacity),
            let notice = TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: wire).flatMap(WitnessCapacityNotice.decode(fromJSON:))
        {
            out.append(Safeguard(title: notice.title, body: notice.body, severity: .waiting))
            said.insert("witness-saturated")
        }
        if let held = status.automaticContributionHeld, (held.heldSessions ?? 0) > 0,
            let wire = Self.wire(held),
            let notice = TCConsentCopy.gateHeldNoticeJSON(forHeld: wire).flatMap(GateHeldNotice.decode(fromJSON:))
        {
            out.append(Safeguard(title: notice.title, body: notice.body, severity: .waiting))
            said.insert(GateHeld.label)
        }
        if let label = status.health?.lastErrorLabel, !said.contains(label) {
            let health = HealthCopy.core(label: label, maxQueueEntries: nil)
            out.insert(Safeguard(title: health.title, body: health.detail, severity: health.severity), at: 0)
        }
        return out
    }

    /// A status part as the wire JSON the core's notice functions read.
    static func wire(_ value: some Encodable) -> String? {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return (try? encoder.encode(value)).map { String(decoding: $0, as: UTF8.self) }
    }

    /// Whether something waiting is worth a second look (nothing matched, or
    /// trimmed to fit), as the main window's queue shield reads it. The
    /// badge pairs with it, so it never says only a count.
    var shield: QueueShieldState { Self.shield(tree.allSessions) }

    static func shield(_ sessions: [DaemonData.QueueEntry]) -> QueueShieldState {
        let reasons = sessions.map { Set($0.secondLook ?? []) }
        return QueueShieldState.state(
            waiting: sessions.count,
            nothingMatched: reasons.filter { $0.contains("nothing-matched") }.count,
            trimmed: reasons.filter { $0.contains("trimmed-to-fit") }.count)
    }

    /// A tool's source declaration, from its switch after the core's
    /// explanation was shown: `.off` is "I do not use this tool", `.watch`
    /// names the folder chosen for it. The core's answer is reloaded.
    func setSource(_ kind: SourceKind, _ choice: SourceChoice) async {
        guard !writing.contains(kind.rawValue) else { return }
        writing.insert(kind.rawValue)
        defer { writing.remove(kind.rawValue) }
        writeErrors[kind.rawValue] = nil
        do {
            _ = try await attached().setSource(kind, choice)
        } catch {
            refused(kind.rawValue, error, method: "set_settings")
            return
        }
        await load()
    }

    /// A folder's mode, chosen from its three-way picker after any
    /// confirmation the view showed (arming, or ignoring a folder with
    /// sessions waiting). `promised` is the waiting count that confirmation
    /// named; the core's `purged` is the authority, and a difference is said.
    func setFolderMode(_ folder: TracesTree.FolderNode, _ mode: ProjectMode, promised: Int) async {
        guard !writing.contains(folder.id) else { return }
        writing.insert(folder.id)
        defer { writing.remove(folder.id) }
        folderNotice = nil
        writeErrors[folder.id] = nil
        do {
            let result = try await attached().setProjectMode(projectId: folder.id, mode: mode, includeBacklog: nil)
            if mode == .ignore {
                folderNotice = TCCoreCopy.projectIgnoreReconciled(
                    project: folder.label, promised: promised, purged: result.purged ?? promised)
            }
        } catch {
            // The picker redraws from the core's answer, so a refused write
            // shows the folder as it still is; the error is kept beside it.
            refused(folder.id, error, method: "set_project_mode")
            return
        }
        await load()
    }

    // MARK: Submit all

    /// What the folder's group control offers, from the daemon's counts on
    /// its `list_projects` row and the shared table (`groupSubmit`); never a
    /// count compared to zero here.
    func groupOffer(_ folder: TracesTree.FolderNode) -> GroupSubmitOffer {
        EligibilitySurface.groupSubmit(
            pendingCount: folder.pendingCount,
            contributableCount: folder.contributableCount,
            fallbackPending: folder.sessions.count,
            calls: Self.eligibilityCalls)
    }

    /// Whether Submit all may be drawn at all, besides the table's answer:
    /// the core is attached, and the folder is not one that is never sent.
    func mayContributeFolder(_ folder: TracesTree.FolderNode) -> Bool {
        client != nil && folder.mode != .ignore
    }

    /// Contribute for a whole folder, optionally with one verdict for every
    /// session ("Submit all as"). The core's toast is kept for its Undo, and
    /// what it left out as ineligible is said in the core's words.
    func contributeFolder(_ folder: TracesTree.FolderNode, verdict: ContributorVerdict?) async {
        guard !writing.contains(folder.id), mayContributeFolder(folder), let client else { return }
        writing.insert(folder.id)
        defer { writing.remove(folder.id) }
        folderNotice = nil
        writeErrors[folder.id] = nil
        do {
            let response = try await client.approveFolder(projectId: folder.id, verdict: verdict)
            lastContributedFolder = ContributedFolder(projectId: folder.id, toast: response.toast)
            lastContributed = nil
            folderNotice = Self.eligibilityCalls
                .withheldLine(Int64(clamping: response.excludedIneligible ?? 0))
                .flatMap { $0.isEmpty ? nil : $0 }
        } catch {
            refused(folder.id, error, method: "approve")
            return
        }
        await load()
    }

    /// Takes a folder's Submit all back inside its hold. A refusal is kept
    /// beside the folder and the contribution stands.
    func undoFolder(_ projectId: String) async {
        guard !writing.contains(projectId), let client else { return }
        writing.insert(projectId)
        defer { writing.remove(projectId) }
        writeErrors[projectId] = nil
        do {
            _ = try await client.cancelFolder(projectId: projectId)
            lastContributedFolder = nil
        } catch {
            refused(projectId, error, method: "cancel")
            return
        }
        await load()
    }

    /// A refused write is said beside its row. A core that did not answer
    /// at all is the whole tab's state, as a failed load is.
    private func refused(_ id: String, _ error: any Error, method: String) {
        let error = error as? DaemonDataError ?? .undecodable(method: method)
        if case .unreachable = error {
            phase = .failed(error)
        } else {
            writeErrors[id] = error
        }
    }
}
