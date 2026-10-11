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
    /// Whether the tree draws folders set to ignore (the View menu's "Show
    /// ignored folders"; shown by default, as #1146). Changing it redraws
    /// the tree from the last read, without asking the core again.
    var showsIgnored = true {
        didSet {
            guard showsIgnored != oldValue, let read = lastRead else { return }
            tree = build(read)
        }
    }

    // MARK: The nudge (re-engagement)

    /// The order chosen with the order control (`list_pending {order}`).
    /// Suggested first until another is chosen (Ron, 2026-10-09): a
    /// segmented control always shows which order the list is in.
    private(set) var order: DaemonData.PendingOrder? = .suggested
    /// Narrowed to the idle sessions the idle card named
    /// (`list_pending {filter: "idle_sessions"}`), by its Review.
    private(set) var idleOnly = false
    /// An idle filter requested before any client was attached
    /// (`requestIdleOnly`), applied by the first attach to one.
    private var idleRequested = false
    /// A row's tags in the core's words (`tc_nudge_entry_tags_json`), by
    /// entry id; rows with nothing to draw are absent.
    private(set) var rowTags: [String: NudgeEntryTags] = [:]
    /// A nudge request in flight.
    private(set) var nudgeBusy = false
    /// The last nudge request the core refused, until the next one.
    private(set) var nudgeError: DaemonDataError?
    /// The core's fixed nudge words, decoded once.
    let nudgeCopy: NudgeCopy? = NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON())

    /// Whether the tree keeps the core's order: whenever an order is set.
    var keepsOrder: Bool { order != nil }

    /// The idle or backlog card, in the daemon's words; nil when it has
    /// none to show here. The idle card is not drawn while its own Review
    /// has the list narrowed to the traces it named: the list is already
    /// the answer to it, and its Show all is the way back.
    var nudgeCard: NudgeSurface.Card? {
        guard let card = NudgeSurface.card(status?.nudge, on: .traces) else { return nil }
        return idleOnly && card.kind == .idleSessions ? nil : card
    }

    private func build(
        _ read: (entries: [DaemonData.QueueEntry], projects: [ProjectRow], settings: DaemonData.Settings?)
    ) -> TracesTree {
        TracesTree.build(
            entries: read.entries, projects: read.projects, settings: read.settings,
            scansWhenUnset: Self.scansWhenUnset, showsIgnored: showsIgnored,
            keepsOrder: keepsOrder, onlyWithSessions: idleOnly)
    }

    /// Asks the core for the list in `order`.
    func setOrder(_ order: DaemonData.PendingOrder) async {
        self.order = order
        await load()
    }

    /// Narrows the list to the idle sessions, or leaves the filter.
    func showIdleOnly(_ on: Bool) async {
        idleOnly = on
        await load()
    }

    /// The idle filter asked for from outside the list -- a notification's
    /// Review, or the menu-bar panel row -- which can arrive before a
    /// freshly opened window has attached its client. With no client yet it
    /// is held, and the first attach to a client applies it instead of
    /// clearing it.
    func requestIdleOnly() async {
        guard client != nil else {
            idleRequested = true
            return
        }
        await showIdleOnly(true)
    }

    /// A card button: its request, then its place. Review narrows (or, on
    /// the backlog card, widens) the list here; Not now goes nowhere. A
    /// refusal is kept and said by the card; a Review still opens its list,
    /// since looking changes nothing.
    func perform(_ intent: NudgeSurface.Intent) async {
        guard !nudgeBusy else { return }
        let effect = NudgeSurface.effect(intent)
        let mine = attachment
        nudgeBusy = true
        defer { if mine == attachment { nudgeBusy = false } }
        nudgeError = nil
        do {
            try await NudgeSurface.send(effect, through: try attached())
        } catch {
            guard mine == attachment else { return }
            nudgeError = error as? DaemonDataError ?? .undecodable(method: "nudge")
            guard case .traces? = effect.destination else { return }
        }
        guard mine == attachment else { return }
        if case .traces(let idle)? = effect.destination { idleOnly = idle }
        await load()
    }

    /// Each row's tags, in the core's words, by entry id.
    static func rowTags(_ entries: [DaemonData.QueueEntry]) -> [String: NudgeEntryTags] {
        var tags: [String: NudgeEntryTags] = [:]
        for entry in entries {
            guard let input = NudgeEntryTags.input(for: entry),
                  let decoded = NudgeEntryTags.decode(fromJSON: TCCoreCopy.nudgeEntryTagsJSON(entryJSON: input)),
                  !decoded.isEmpty
            else { continue }
            tags[entry.entryId] = decoded
        }
        return tags
    }
    /// The last successful read the tree was built from.
    @ObservationIgnored private var lastRead: (entries: [DaemonData.QueueEntry], projects: [ProjectRow], settings: DaemonData.Settings?)?
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
    /// Bumped only by `attach`, not by a load: a write that started against
    /// an older client is dropped when it answers, whatever it says, so its
    /// refusal, notice or undo is never drawn on the new client's tab. (A
    /// load on the same client must not drop a write's answer.)
    private var attachment = 0
    /// True while no client is attached because the daemon is still
    /// starting (set by `attach`). `run` then reads nothing and the screen
    /// stays loading: start-up is not the core being down. Any other nil
    /// client fails as an unreachable core.
    private(set) var awaiting = false

    /// The tab's words, from the core (`tc_monitor_traces_copy_json`),
    /// decoded once rather than on every redraw. Nil leaves a label out
    /// rather than writing one here.
    let words: MonitorTracesCopy? = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())
    /// The consent gate's words, from the core, decoded once rather than on
    /// every redraw. Without them Contribute stays disarmed: the shell never
    /// words consent itself.
    let consent: ConsentCopy? = TCConsentCopy.copyJSON().flatMap(ConsentCopy.decode(fromJSON:))
    /// The core's disclosure bundle (`tc_contributor_disclosure_copy_json`),
    /// decoded once for the whole app: the review card's verdict and
    /// correction words and the longest correction the daemon accepts, the
    /// folder inspector's and the tree's folder mode names, Submit all as's
    /// outcome words, and History's filter labels. Every one of those reads
    /// this copy, so no two can hold different words. Without it the card
    /// draws no verdict and Contribute stays disarmed.
    nonisolated static let disclosureCopy: ContributorDisclosureCopy? =
        ContributorDisclosureCopy.decode(fromJSON: TCCoreCopy.contributorDisclosureCopyJSON())
    /// `disclosureCopy`, for a view that holds this store.
    var disclosure: ContributorDisclosureCopy? { Self.disclosureCopy }
    /// The session whose last Contribute the daemon refused because its
    /// correction looked like it held a credential. Nothing was sent; the
    /// card says the core's headline and body for it, in place of the
    /// generic refusal, until the next action on that session.
    private(set) var correctionRefused: String?

    /// A Contribute the core took, while its hold lets it be taken back:
    /// the core's toast for it, and whether Undo (`cancel`) is offered.
    struct Contributed: Equatable {
        let entryId: String
        let toast: SubmitToast
        /// The session's folder name, for #1146's "{label} approved"; nil
        /// when the tree no longer lists the session.
        var label: String? = nil
        /// When the core took it, on this machine's clock: the card counts
        /// up from here (the accepted count-up, never a countdown).
        var at = Date()
    }

    /// The last contribution, until it is undone or another is made.
    private(set) var lastContributed: Contributed?

    /// A folder's Submit all the core took, while its hold lets it be taken
    /// back (`cancel` with the project id).
    struct ContributedFolder: Equatable {
        let projectId: String
        let toast: SubmitToast
        var label: String? = nil
        var at = Date()
    }

    private(set) var lastContributedFolder: ContributedFolder?
    /// The folder whose Submit all is in flight, apart from `writing`,
    /// which a mode change also holds: only a bulk approve is a demand on
    /// the inspector (Ron's `submit:` key in `useInspectorDemand`).
    private(set) var submittingFolder: String?

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
    /// or write still in flight from the old one is dropped. Its refusals,
    /// notices and undo offers go too: Undo would otherwise send `cancel` to
    /// the new daemon. (A failed read from the same client still keeps the
    /// last tree.)
    func attach(_ client: (any DaemonDataClient)?, awaiting: Bool = false) {
        self.client = client
        self.awaiting = awaiting && client == nil
        generation += 1
        attachment += 1
        phase = .loading
        tree = TracesTree(tools: [], unplaced: [])
        lastRead = nil
        status = nil
        destinations = nil
        acting = []
        writing = []
        lastKept = nil
        lastContributed = nil
        lastContributedFolder = nil
        submittingFolder = nil
        correctionRefused = nil
        actionError = nil
        writeErrors = [:]
        folderNotice = nil
        // A new daemon starts unfiltered, unless the filter was asked for
        // before any client was here to load it.
        idleOnly = client != nil && idleRequested
        if client != nil { idleRequested = false }
        rowTags = [:]
        nudgeBusy = false
        nudgeError = nil
        // A new client is a new daemon: no undo, toast or refusal from the
        // old one survives into it.
        lastKept = nil
        lastContributed = nil
        actionError = nil
        writeErrors = [:]
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

    /// Secrets the scan found and left in what would be sent, in the core's
    /// words (`tc_residual_secret_line_text`), as the queue card said them.
    /// Counted by detection site, never by secret, and the sites are named.
    /// Nil before the preview is in, and when nothing survived.
    static func survivorLine(_ summary: DaemonData.PreviewSummary?) -> String? {
        guard let redactions = summary?.redactions else { return nil }
        let total = RedactionLabels.survivorTotal(redactions)
        guard total > 0 else { return nil }
        return TCCoreCopy.residualSecretLine(count: total, sites: RedactionLabels.survivors(redactions).map(\.site))
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
        let mark = Self.attestationMark(entry)
        return [
            AttestationSurface.markLine(mark, copy: copy, calls: Self.attestationCalls),
            AttestationSurface.reasonLine(mark, calls: Self.attestationCalls),
        ].compactMap { $0 }.joined(separator: " ")
    }

    /// A queue entry's attestation mark, as the shared table reads it.
    static func attestationMark(_ entry: DaemonData.QueueEntry) -> AttestationMark {
        AttestationMark(
            mark: entry.attestation ?? "",
            reason: (entry.attestationReason?.isEmpty ?? true) ? nil : entry.attestationReason)
    }

    /// The session row's attestation sentence (#1146's
    /// `attestation_copy.state_line`): the core's sentence for the mark,
    /// without its reason. Nil only when the copy would not decode.
    func attestationLine(_ entry: DaemonData.QueueEntry) -> String? {
        guard let copy = inferenceCopy else { return nil }
        return AttestationSurface.markLine(Self.attestationMark(entry), copy: copy, calls: Self.attestationCalls)
    }

    /// The tone the core gives a session's attestation mark. A mark this
    /// build cannot read is `.neutral`, never `.clear`.
    static func attestationTone(_ entry: DaemonData.QueueEntry) -> PrivateInferenceTone {
        AttestationSurface.tone(attestationMark(entry), calls: attestationCalls)
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
    /// an unreachable core, and there is no stream to follow -- unless the
    /// daemon is still starting (`awaiting`), when the tab stays loading.
    func run() async {
        guard !awaiting else { return }
        await load()
        guard let client else { return }
        for await event in client.events() {
            if Task.isCancelled { return }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                await load()
            case .digestDue, .reengageDue, .previewReady, .inferenceCallAdded, .usageChanged, .unknown:
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
            async let entries = client.listPending(
                projectId: nil, filter: idleOnly ? .idleSessions : nil, order: order)
            async let projects = client.listProjects()
            // Settings only decide the tool switches. Unreadable settings
            // are unknown: no switch, never off, and the row says so.
            async let settings = try? client.settings()
            async let status = try? client.status()
            async let destinations = try? client.toolDestinations()
            let read = (entries: try await entries, projects: try await projects.projects, settings: await settings)
            let built = build(read)
            let tags = Self.rowTags(read.entries)
            let statusRead = await status
            let routes = await destinations
            guard mine == generation else { return }
            tree = built
            rowTags = tags
            lastRead = read
            self.status = statusRead
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

    // MARK: Selection

    /// The selection while it names something in the current tree; nil
    /// otherwise, and then the inspector shows the Summary. While a new
    /// client's tree loads nothing resolves, and the stored selection is
    /// kept for when it does.
    func resolve(_ selection: MonitorSelection?) -> MonitorSelection? {
        tree.resolve(selection)
    }

    /// The selected session's queue entry while it is still waiting; nil for
    /// a folder, for nothing, and for a session that has gone. Its card and
    /// Contribute are drawn only from this.
    func selectedSession(_ selection: MonitorSelection?) -> DaemonData.QueueEntry? {
        guard let entryID = resolve(selection)?.entryID else { return nil }
        return tree.allSessions.first { $0.entryId == entryID }
    }

    /// The selected folder while it is still in the tree; nil for a
    /// session, for nothing, and for a folder that has gone. Its inspector
    /// is drawn only from this.
    func selectedFolder(_ selection: MonitorSelection?) -> TracesTree.FolderNode? {
        guard let projectID = resolve(selection)?.projectID else { return nil }
        return tree.folders.first { $0.id == projectID }
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
    /// optimistically: the tree redraws from the core's answer. An answer
    /// from a client that has since been replaced changes nothing.
    ///
    /// `verdict` and `correction` go with `.contribute` only: the review
    /// card's answer to the outcome question, and what it wrote under
    /// Partly or Failed (nil sends no key, never an empty one).
    func perform(
        _ action: ReviewAction, on entryId: String, verdict: ContributorVerdict? = nil, correction: String? = nil
    ) async {
        guard !acting.contains(entryId) else { return }
        let mine = attachment
        acting.insert(entryId)
        defer { if mine == attachment { acting.remove(entryId) } }
        actionError = nil
        if correctionRefused == entryId { correctionRefused = nil }
        do {
            let client = try attached()
            switch action {
            case .contribute:
                // The core's answer is kept, not thrown away: its toast, and
                // the hold Undo can still reach. A skipped approve throws
                // `notApproved` and is said as a refusal, never as success.
                let response = try await client.approve(entryId: entryId, verdict: verdict, correction: correction)
                guard mine == attachment else { return }
                lastContributed = Contributed(
                    entryId: entryId, toast: response.toast,
                    label: tree.allSessions.first { $0.entryId == entryId }?.projectLabel)
                // One undo slot: a single-session contribute ends the folder's undo.
                lastContributedFolder = nil
                // A newer decision ends the older Keep's undo.
                lastKept = nil
            case .undoContribute:
                try await client.cancel(entryId: entryId)
                guard mine == attachment else { return }
                lastContributed = nil
            case .keep:
                _ = try await client.keep(entryId: entryId)
                guard mine == attachment else { return }
                lastKept = entryId
            case .undoKeep:
                _ = try await client.undoKeep(entryId: entryId)
                guard mine == attachment else { return }
                lastKept = nil
            case .dismiss:
                try await client.dismiss(entryId: entryId)
                guard mine == attachment else { return }
                lastKept = nil
            }
        } catch {
            guard mine == attachment else { return }
            let error = error as? DaemonDataError ?? .undecodable(method: "\(action)")
            // A core that did not answer is the tab's state, and its last
            // count is no longer known.
            if case .unreachable = error { lost() }
            // A credential in the correction is the one refusal the
            // contributor caused and can fix: said in its own words, not
            // as the generic refusal.
            if action == .contribute, error == .notApproved(reasonLabel: CorrectionCopy.credentialRefusalLabel) {
                correctionRefused = entryId
                return
            }
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

    /// `maxQueueEntries` is the daemon's configured queue limit, which only
    /// the queue-full line counts from (`HealthCopy.core`).
    ///
    /// `capacityUnreadable` is the screens table's line for a capacity this
    /// build cannot read; without it the core's witness-saturated line says
    /// the capacity instead, never nothing.
    static func safeguards(
        _ status: DaemonData.Status?, maxQueueEntries: Int? = nil,
        capacityUnreadable: String? = MonitorWords.table?.safeguards.capacityUnreadable
    ) -> [Safeguard] {
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
        if let capacity = status.witnessCapacity {
            // Reported but not readable (no count, a negative one, or one
            // the core could not word) is never "none waiting" (Ron's
            // `QueueStatusPanel`): sessions may be held, so it is said.
            let waiting = capacity.waitingSessions
            if let waiting, waiting == 0 {
                // None waiting.
            } else if let waiting, waiting > 0, let wire = Self.wire(capacity),
                let notice = TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: wire)
                    .flatMap(WitnessCapacityNotice.decode(fromJSON:))
            {
                out.append(Safeguard(title: notice.title, body: notice.body, severity: .waiting))
                said.insert("witness-saturated")
            } else {
                if let unreadable = capacityUnreadable {
                    out.append(Safeguard(title: unreadable, body: nil, severity: .waiting))
                } else {
                    // No screens table: the core's saturated line, which is
                    // always there, rather than a silence read as "none
                    // waiting".
                    let saturated = HealthCopy.core(label: "witness-saturated", maxQueueEntries: maxQueueEntries)
                    out.append(Safeguard(title: saturated.title, body: saturated.detail, severity: .waiting))
                }
                // It says what the saturated label would; that line steps
                // aside for it (Ron's `saturatedShownByNotice`).
                said.insert("witness-saturated")
            }
        }
        if let held = status.automaticContributionHeld, (held.heldSessions ?? 0) > 0,
            let wire = Self.wire(held),
            let notice = TCConsentCopy.gateHeldNoticeJSON(forHeld: wire).flatMap(GateHeldNotice.decode(fromJSON:))
        {
            out.append(Safeguard(title: notice.title, body: notice.body, severity: .waiting))
            said.insert(GateHeld.label)
        }
        if let label = status.health?.lastErrorLabel, !said.contains(label) {
            let health = HealthCopy.core(label: label, maxQueueEntries: maxQueueEntries)
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
        let mine = attachment
        writing.insert(kind.rawValue)
        defer { if mine == attachment { writing.remove(kind.rawValue) } }
        writeErrors[kind.rawValue] = nil
        do {
            _ = try await attached().setSource(kind, choice)
        } catch {
            guard mine == attachment else { return }
            refused(kind.rawValue, error, method: "set_settings")
            return
        }
        guard mine == attachment else { return }
        await load()
    }

    /// A folder's mode, chosen from its three-way picker after any
    /// confirmation the view showed (arming, or ignoring a folder with
    /// sessions waiting). `promised` is the waiting count that confirmation
    /// named; the core's `purged` is the authority, and a difference is said.
    func setFolderMode(_ folder: TracesTree.FolderNode, _ mode: ProjectMode, promised: Int) async {
        guard !writing.contains(folder.id) else { return }
        let mine = attachment
        writing.insert(folder.id)
        defer { if mine == attachment { writing.remove(folder.id) } }
        folderNotice = nil
        writeErrors[folder.id] = nil
        do {
            let result = try await attached().setProjectMode(projectId: folder.id, mode: mode, includeBacklog: nil)
            guard mine == attachment else { return }
            if mode == .ignore {
                folderNotice = TCCoreCopy.projectIgnoreReconciled(
                    project: folder.label, promised: promised, purged: result.purged ?? promised)
            }
        } catch {
            // The picker redraws from the core's answer, so a refused write
            // shows the folder as it still is; the error is kept beside it.
            guard mine == attachment else { return }
            refused(folder.id, error, method: "set_project_mode")
            return
        }
        await load()
    }

    // MARK: Submit all

    /// What the folder's group control offers, from the daemon's counts on
    /// its `list_projects` row and the shared table (`groupSubmit`); never a
    /// count compared to zero here. Under the idle filter those counts are
    /// the whole folder's, so the offer counts the traces drawn under it
    /// instead -- less any held for a person, which no group sends -- and
    /// Submit sends only those (`approveFolder(filter:)`); what the daemon
    /// then leaves out as ineligible is said after, in its own line.
    func groupOffer(_ folder: TracesTree.FolderNode) -> GroupSubmitOffer {
        if idleOnly {
            return EligibilitySurface.groupSubmit(
                pendingCount: folder.sessions.filter { !$0.heldForReview }.count,
                contributableCount: nil,
                fallbackPending: folder.sessions.count,
                calls: Self.eligibilityCalls)
        }
        return EligibilitySurface.groupSubmit(
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
    /// session ("Submit all as"); under the idle filter, only the folder's
    /// traces the filter shows. The core's toast is kept for its Undo, and
    /// what it left out as ineligible is said in the core's words.
    func contributeFolder(_ folder: TracesTree.FolderNode, verdict: ContributorVerdict?) async {
        guard !writing.contains(folder.id), mayContributeFolder(folder), let client else { return }
        let mine = attachment
        writing.insert(folder.id)
        submittingFolder = folder.id
        defer {
            if mine == attachment {
                writing.remove(folder.id)
                submittingFolder = nil
            }
        }
        folderNotice = nil
        writeErrors[folder.id] = nil
        do {
            let response = try await client.approveFolder(
                projectId: folder.id, verdict: verdict, filter: idleOnly ? .idleSessions : nil)
            guard mine == attachment else { return }
            lastContributedFolder = ContributedFolder(projectId: folder.id, toast: response.toast, label: folder.label)
            lastContributed = nil
            folderNotice = Self.eligibilityCalls
                .withheldLine(Int64(clamping: response.excludedIneligible ?? 0))
                .flatMap { $0.isEmpty ? nil : $0 }
        } catch {
            guard mine == attachment else { return }
            refused(folder.id, error, method: "approve")
            return
        }
        await load()
    }

    /// Takes a folder's Submit all back inside its hold. A refusal is kept
    /// beside the folder and the contribution stands.
    func undoFolder(_ projectId: String) async {
        guard !writing.contains(projectId), let client else { return }
        let mine = attachment
        writing.insert(projectId)
        defer { if mine == attachment { writing.remove(projectId) } }
        writeErrors[projectId] = nil
        do {
            _ = try await client.cancelFolder(projectId: projectId)
            guard mine == attachment else { return }
            lastContributedFolder = nil
        } catch {
            guard mine == attachment else { return }
            refused(projectId, error, method: "cancel")
            return
        }
        await load()
    }

    /// Closes the contribution's undo card (Ron's `UndoBar` Dismiss). The
    /// contribution stands; only the offer to take it back goes.
    func dismissContributed() {
        lastContributed = nil
    }

    /// Closes the folder's Submit all undo card. The contribution stands.
    func dismissContributedFolder() {
        lastContributedFolder = nil
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
