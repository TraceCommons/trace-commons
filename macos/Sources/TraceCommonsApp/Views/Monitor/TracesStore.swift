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
    /// The last write the core refused, by folder id (or tool id for a
    /// source declaration). It stays beside that row until the next write to
    /// it: a reload does not clear it, and it never stands in for the core
    /// being down.
    private(set) var writeErrors: [String: DaemonDataError] = [:]

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

    /// The sample set drawn, in a debug build over sample data; nil over
    /// the daemon. `sampleUnknown` is a `TRACE_COMMONS_SAMPLE` that named no
    /// set, so the fallback is never silent.
    let sample: String?
    let sampleUnknown: Bool

    init(client: any DaemonDataClient, sample: String? = nil, sampleUnknown: Bool = false) {
        self.client = client
        self.sample = sample
        self.sampleUnknown = sampleUnknown
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
            let built = TracesTree.build(
                entries: try await entries, projects: try await projects.projects, settings: await settings,
                scansWhenUnset: Self.scansWhenUnset)
            guard mine == generation else { return }
            tree = built
            phase = .loaded
        } catch {
            // An older failure never overwrites a newer read either.
            guard mine == generation else { return }
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "list_pending"))
        }
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
            _ = try await client.setSource(kind, choice)
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
            let result = try await client.setProjectMode(projectId: folder.id, mode: mode, includeBacklog: nil)
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
