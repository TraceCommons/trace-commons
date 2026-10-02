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
    /// A review action (R7) in flight, by entry id.
    private(set) var acting: Set<String> = []
    /// The session last kept on this Mac, while its undo is offered.
    private(set) var lastKept: String?
    /// The last review action the core refused, by entry id.
    private(set) var actionError: (entryId: String, error: DaemonDataError)?

    /// The Traces badge: decisions owed, unknown (a dash) when the core did
    /// not say, never a count derived from the queue.
    var decisionsOwed: Int? { status?.decisionsOwed }

    let client: any DaemonDataClient
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

    /// The shared eligibility table, as `AppModel` wires it.
    static let eligibilityCalls = EligibilityCalls(
        stateLine: { TCContributionEligibility.stateLine(state: $0) },
        stateTone: { TCContributionEligibility.stateTone(state: $0) },
        control: { TCContributionEligibility.control(state: $0) },
        reasonLine: { TCContributionEligibility.reasonLine(reason: $0) },
        withheldLine: { TCContributionEligibility.withheldLine(withheld: $0) },
        groupControl: { TCContributionEligibility.groupControl(pending: $0, contributable: $1) }
    )

    /// A Contribute the core took, while its hold lets it be taken back:
    /// the core's toast for it, and whether Undo (`cancel`) is offered.
    struct Contributed: Equatable {
        let entryId: String
        let toast: SubmitToast
    }

    /// The last contribution, until it is undone or another is made.
    private(set) var lastContributed: Contributed?

    init(client: any DaemonDataClient) {
        self.client = client
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
    /// cancelled and the stream with it.
    func run() async {
        await load()
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                await load()
            case .digestDue, .previewReady, .inferenceCallAdded, .unknown:
                break
            }
        }
    }

    func load() async {
        generation += 1
        let mine = generation
        do {
            async let entries = client.listPending(projectId: nil)
            async let projects = client.listProjects()
            // Settings only decide the tool switches. Unreadable settings
            // are unknown: no switch, never off, and the row says so.
            async let settings = try? client.settings()
            async let status = try? client.status()
            let built = TracesTree.build(
                entries: try await entries, projects: try await projects.projects, settings: await settings,
                scansWhenUnset: Self.scansWhenUnset)
            let read = await status
            guard mine == generation else { return }
            tree = built
            self.status = read
            phase = .loaded
        } catch {
            guard mine == generation else { return }
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "list_pending"))
            status = nil
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

    /// A queue entry's eligibility, as the shared table reads it; nil when
    /// the daemon sent none (eligibility does not apply).
    static func eligibility(_ entry: DaemonData.QueueEntry) -> ContributionEligibility? {
        entry.eligibility.map { ContributionEligibility(state: $0, reason: entry.eligibilityReason) }
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
            switch action {
            case .contribute:
                // The core's answer is kept, not thrown away: its toast, and
                // the hold Undo can still reach. A skipped approve throws
                // `notApproved` and is said as a refusal, never as success.
                let response = try await client.approve(entryId: entryId)
                lastContributed = Contributed(entryId: entryId, toast: response.toast)
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
            }
        } catch {
            actionError = (entryId, error as? DaemonDataError ?? .undecodable(method: "\(action)"))
            return
        }
        await load()
    }

    /// A tool's source declaration, from its switch after the core's
    /// explanation was shown: `.off` is "I do not use this tool", `.watch`
    /// names the folder chosen for it. The core's answer is reloaded.
    func setSource(_ kind: SourceKind, _ choice: SourceChoice) async {
        guard !writing.contains(kind.rawValue) else { return }
        writing.insert(kind.rawValue)
        defer { writing.remove(kind.rawValue) }
        do {
            _ = try await client.setSource(kind, choice)
        } catch {
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "set_settings"))
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
        do {
            let result = try await client.setProjectMode(projectId: folder.id, mode: mode, includeBacklog: nil)
            if mode == .ignore {
                folderNotice = TCCoreCopy.projectIgnoreReconciled(
                    project: folder.label, promised: promised, purged: result.purged ?? promised)
            }
        } catch {
            // The picker redraws from the core's answer, so a refused write
            // shows the folder as it still is; the error is kept.
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "set_project_mode"))
            return
        }
        await load()
    }
}
