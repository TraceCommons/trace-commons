import Foundation
import Observation
import TCBridge
import TCDesign
import TCShellCore

/// The menu-bar popover's data (R13 of #1173), read through
/// `DaemonDataClient` like the other glass screens. The contribution
/// override (#1208) is written here, through the same client, after its
/// core confirmation; the popover's other actions (pause, resume, Private
/// AI off) stay on `AppModel`, which the shipping menu already uses.
///
/// The client is the app's live one (`AppModel.daemonData`), attached by
/// the menu-bar label when the daemon starts and detached when it stops;
/// sample data reaches this store only from tests. With no client, or after
/// any read fails, the data is `stale` and drawn as unavailable rather than
/// as the last values.
@MainActor
@Observable
final class MenuPanelStore {
    private(set) var projects: [ProjectRow]?
    /// `status`: the core's pill roll-up (`contribution_mode`), whether an
    /// override is in force, and whether Auto contribute is partial (#1208).
    private(set) var status: DaemonData.Status?
    private(set) var pending: [DaemonData.QueueEntry] = []
    private(set) var kept: [DaemonData.QueueEntry] = []
    private(set) var history: [DaemonData.HistoryRow] = []
    private(set) var calls: [DaemonData.InferenceCall] = []
    /// True until a load has read everything, and again whenever a read
    /// fails, there is no client, or the client's event stream ends: what
    /// is held is no longer current.
    private(set) var stale = true
    /// Today's routed calls per tool (`insights_glance`). Read on its own:
    /// no answer here ever stales the popover, since the glance is off by
    /// default and an older daemon does not serve it.
    private(set) var glance: MenuPanelData.GlanceState = .notRead
    /// How often an open popover re-reads the glance: the ledger can stop
    /// refreshing with no event, and only a re-read sees it go stale.
    static let glanceRefresh: Duration = .seconds(60)

    private(set) var client: (any DaemonDataClient)?

    /// The contributor configuration directory, for the Auto contribute
    /// confirmation's arming disclosure, which the core words for it.
    var configDirectory: String?

    /// The override confirmation on screen, in the core's words; nil when
    /// none is.
    private(set) var confirming: ContributionOverrideConfirmCopy?
    /// The core's line for the last refused override write, until the next
    /// choice.
    private(set) var overrideRefusal: String?
    /// An override write is in flight.
    private(set) var writingOverride = false

    init(client: (any DaemonDataClient)?) {
        self.client = client
    }

    /// Follows a new client (or none): the old data is stale until the
    /// new one has been read.
    func attach(_ client: (any DaemonDataClient)?, configDirectory: String? = nil) {
        self.client = client
        if let configDirectory { self.configDirectory = configDirectory }
        stale = true
        glance = .notRead
    }

    // MARK: The contribution override (#1208)

    /// Whether the pill's choices can be used: only on positive evidence
    /// that the core is up and its status was read. Core-down, loading,
    /// stale and a write in flight all keep them disabled, so an absent
    /// signal never lets a write through.
    var canChooseOverride: Bool {
        client != nil && !stale && status != nil && !writingOverride && confirming == nil
    }

    /// A choice was pressed: show that override's confirmation, in the
    /// core's words. Nothing is sent until it is confirmed. Without the
    /// core's confirmation (for Auto contribute, a configuration whose
    /// arming disclosure cannot be read) nothing can be confirmed, and the
    /// refusal the daemon would give is shown instead.
    func choose(_ mode: String) {
        guard canChooseOverride else { return }
        overrideRefusal = nil
        let copy = ContributionOverrideConfirmCopy.decode(
            fromJSON: TCCoreCopy.contributionOverrideConfirmJSON(mode: mode, configDir: configDirectory))
        guard let copy, copy.mode == mode else {
            overrideRefusal = TCCoreCopy.contributionOverrideRefusalLine(
                label: mode == ProjectMode.autoUpload.rawValue ? "arming-terms-unavailable" : "")
            return
        }
        confirming = copy
    }

    /// Mixed was pressed while an override is in force. Clearing hands
    /// every folder back to its own setting, so when the override is Ask me
    /// or Never and any folder's own setting is Automatic (or is not known),
    /// the core's clear confirmation is shown first, as for an override:
    /// unattended sending never resumes from a single menu press. Under an
    /// Automatic override clearing resumes nothing, so it, like a clear with
    /// no Automatic folder, happens at once.
    /// Without the core's confirmation nothing is cleared.
    func chooseMixed() async {
        guard canChooseOverride, status?.contributionOverride != nil else { return }
        overrideRefusal = nil
        guard MenuPanelData.clearNeedsConfirmation(projects, override: status?.contributionOverride?.mode) else {
            await writeOverride { _ = try await $0.clearContributionOverride() }
            return
        }
        let copy = ContributionOverrideConfirmCopy.decode(
            fromJSON: TCCoreCopy.contributionOverrideConfirmJSON(mode: ContributionOverrideConfirmCopy.clearMode, configDir: nil))
        guard let copy, copy.mode == ContributionOverrideConfirmCopy.clearMode else {
            overrideRefusal = TCCoreCopy.contributionOverrideRefusalLine(label: "")
            return
        }
        confirming = copy
    }

    /// The confirmation was answered. Cancel sends nothing; confirm sends
    /// `clear_contribution_override` for the clear confirmation, or else
    /// `set_contribution_override`, with `confirm: true` for Auto
    /// contribute (the core's arming disclosure was just shown), then
    /// re-reads `status`: the pill shows what the daemon says, never a guess.
    func resolveConfirmation(confirmed: Bool) async {
        guard let copy = confirming else { return }
        confirming = nil
        guard confirmed else { return }
        if copy.mode == ContributionOverrideConfirmCopy.clearMode {
            await writeOverride { _ = try await $0.clearContributionOverride() }
            return
        }
        guard let mode = ProjectMode(rawValue: copy.mode) else { return }
        await writeOverride { try await $0.setContributionOverride(mode: mode, confirm: mode == .autoUpload) }
    }

    /// The core's clear action, unconfirmed: every folder back on its own
    /// setting. The panel goes through `chooseMixed`, which confirms first
    /// when a folder is Automatic.
    func clearOverride() async {
        guard canChooseOverride else { return }
        overrideRefusal = nil
        await writeOverride { _ = try await $0.clearContributionOverride() }
    }

    private func writeOverride(_ write: (any DaemonDataClient) async throws -> Void) async {
        guard let client else { return }
        writingOverride = true
        defer { writingOverride = false }
        do {
            try await write(client)
        } catch {
            overrideRefusal = TCCoreCopy.contributionOverrideRefusalLine(label: Self.refusalLabel(error))
        }
        await load()
    }

    /// The daemon's label for a refused write, for the core's line; never
    /// `error.description`. Anything without one gets the core's fallback.
    static func refusalLabel(_ error: any Error) -> String {
        if case .daemon(_, let message)? = error as? DaemonDataError { return message }
        return ""
    }

    func run() async {
        await load()
        guard let client else { return }
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired, .inferenceCallAdded:
                await load()
            case .digestDue, .previewReady, .unknown:
                break
            case .usageChanged:
                await loadGlance()
            }
        }
        // The stream ended: the daemon went away.
        if !Task.isCancelled { stale = true }
    }

    /// Reads everything. A failed read marks the data stale rather than
    /// keeping the last value as if it were current; a method the daemon
    /// does not have yet is not a failure.
    func load() async {
        guard let client else {
            stale = true
            glance = .unavailable
            return
        }
        var failed = false
        func read<T>(_ call: () async throws -> T) async -> T? {
            do { return try await call() } catch DaemonDataError.notAvailableYet {
                return nil
            } catch {
                failed = true
                return nil
            }
        }
        if let value = await read({ try await client.status() }) { status = value }
        if let value = await read({ try await client.listProjects() }) { projects = value.projects }
        if let value = await read({ try await client.listPending(projectId: nil) }) { pending = value }
        if let value = await read({ try await client.listKept() }) { kept = value }
        if let value = await read({ try await client.listHistory(limit: HomeStore.historyLimit) }) { history = value }
        if let value = await read({ try await client.inferenceCalls(limit: 20, cursor: nil) }) {
            calls = value.readable ? value.calls : []
        }
        stale = failed
        await loadGlance()
    }

    /// Reads the glance alone. Any failure, an older daemon's
    /// `unknown_method` included, is no glance; it never marks the
    /// popover's other data stale.
    func loadGlance() async {
        guard let client else {
            glance = .unavailable
            return
        }
        let result: Result<DaemonData.InsightsGlance, any Error>
        do {
            result = .success(try await client.insightsGlance(tzSeconds: TimeZone.current.secondsFromGMT()))
        } catch {
            result = .failure(error)
        }
        glance = MenuPanelData.glance(result)
    }

    /// Re-reads the glance while the popover is open, so a ledger that
    /// stops refreshing is seen as stale without an event.
    func followGlance() async {
        while !Task.isCancelled {
            do { try await Task.sleep(for: Self.glanceRefresh) } catch { return }
            await loadGlance()
        }
    }

    /// The graph's columns: contributed (shared) and kept, per day.
    var columns: [GlassDayColumn] {
        MenuPanelData.days(
            shared: history.compactMap(\.submittedAt),
            kept: kept.compactMap { $0.discoveredAt ?? $0.startedAt },
            ending: Date())
    }
}

/// The popover's rules, as pure functions so they are tested.
enum MenuPanelData {
    /// What the glance read found. Only `data` is drawn; every other state
    /// draws no card, never a zero.
    enum GlanceState: Equatable {
        /// Not read since the client was attached.
        case notRead
        /// The ledger feed is off (`enabled: false`).
        case off
        /// No answer: a failure, no client, or a daemon without the method.
        case unavailable
        /// The ledger did not answer, or the answer lacks its rows.
        case unreadable
        /// The ledger stopped refreshing; a missing `stale` counts as stale.
        case stale
        case data(DaemonData.InsightsGlance)
    }

    /// The glance read's state. Fails closed: only an enabled, readable
    /// answer with rows and coverage that says it is fresh is data.
    static func glance(_ result: Result<DaemonData.InsightsGlance, any Error>) -> GlanceState {
        guard case .success(let glance) = result else { return .unavailable }
        guard glance.enabled else { return .off }
        guard glance.readable == true, glance.tools != nil, glance.coverage != nil else { return .unreadable }
        guard glance.stale == false else { return .stale }
        return .data(glance)
    }

    /// The glance the card draws: fresh data with at least one tool, while
    /// the popover's own data is current. Anything else draws no card.
    static func glanceToDraw(_ state: GlanceState, stale: Bool) -> DaemonData.InsightsGlance? {
        guard !stale, case .data(let glance) = state, let tools = glance.tools, !tools.isEmpty else { return nil }
        return glance
    }

    /// Where "Manage rules" goes: the watched folders in Settings, or first
    /// run while onboarding is required, with no Settings section (R-43).
    static func manageRules(requiresOnboarding: Bool) -> MonitorDestination? {
        requiresOnboarding ? nil : .settings(.watchedFolders)
    }

    /// The roll-up of every listed folder's mode, for the mode pill.
    enum ModeRollup: Equatable {
        case ask, armed, never
        /// The folders do not all share one mode.
        case mixed
        /// No folder is listed yet.
        case none
    }

    /// The pill's roll-up as the daemon computed it (`status
    /// .contribution_mode`, #1208); a shell never derives it from the
    /// folders. Unknown or absent is `none`, drawn as a dash.
    static func rollup(_ contributionMode: String?) -> ModeRollup {
        switch contributionMode {
        case "notify_only": .ask
        case "auto_upload": .armed
        case "ignore": .never
        case "mixed": .mixed
        default: .none
        }
    }

    /// The pill's value: the core's label for its mode, its Mixed word,
    /// or a dash when the core did not say.
    static func modeValue(_ rollup: ModeRollup, mode: String?, copy: ContributionModeCopy?) -> String {
        guard let copy else { return "—" }
        switch rollup {
        case .mixed: return copy.mixed
        case .none: return "—"
        case .ask, .armed, .never: return copy.choice(for: mode)?.label ?? "—"
        }
    }

    /// Whether clearing the override needs the core's confirmation: some
    /// folder's own setting is Automatic, so clearing resumes unattended
    /// sending there. An unread folder list, or a folder whose own setting
    /// the daemon did not report, needs it too (fail closed).
    static func clearNeedsConfirmation(_ projects: [ProjectRow]?, override: String?) -> Bool {
        // Under an Automatic override every folder already sends unattended;
        // clearing starts nothing new (an Automatic folder keeps its own
        // arming, Ask me folders go back to asking).
        if override == "auto_upload" { return false }
        guard let projects else { return true }
        return projects.contains { $0.folderMode == nil || $0.folderMode == .autoUpload }
    }

    /// Whether pressing a mode row only closes the list: with no override
    /// in force the checked row is the core's roll-up itself, so pressing it
    /// changes nothing and must not set a global override (#1255 review).
    /// Under an override, or before a current status, every row is a choice.
    static func pressOnlyCloses(_ mode: String, status: DaemonData.Status?, stale: Bool) -> Bool {
        guard !stale, let status, status.contributionOverride == nil else { return false }
        return listChecks(mode, status: status, stale: stale)
    }

    /// Whether the pill's list checks a row, agreeing with the pill: an
    /// override's own mode while one is in force; with none, the core's
    /// roll-up, which is the Mixed row (`mode` nil) only when the folders
    /// differ. Nothing is checked before the status has been read, or while
    /// the core is down: a status kept from before is not a known mode.
    static func listChecks(_ mode: String?, status: DaemonData.Status?, stale: Bool) -> Bool {
        guard !stale, let status else { return false }
        if let active = status.contributionOverride {
            return mode != nil && active.mode == mode
        }
        switch rollup(status.contributionMode) {
        case .mixed: return mode == nil
        case .none: return false
        case .ask, .armed, .never: return mode != nil && status.contributionMode == mode
        }
    }

    /// The core's partial line, under Auto contribute exactly when the
    /// status says it is partial (#1208).
    static func partialLine(_ mode: String, status: DaemonData.Status?, copy: ContributionModeCopy) -> String? {
        guard mode == "auto_upload", status?.contributionModePartial == true else { return nil }
        return copy.autoPartial
    }

    /// The graph's span in days, and so its number of columns.
    static let days = 36

    /// One column per day, oldest first, ending today: how many sessions
    /// were contributed (up) and kept (down) that day. Counts per day, from
    /// the history and the kept list; the daemon reports no byte rates.
    static func days(
        shared: [Date], kept: [Date], ending now: Date, count: Int = days, calendar: Calendar = .current
    ) -> [GlassDayColumn] {
        let today = calendar.startOfDay(for: now)
        func index(_ date: Date) -> Int? {
            let day = calendar.startOfDay(for: date)
            guard let back = calendar.dateComponents([.day], from: day, to: today).day,
                  back >= 0, back < count else { return nil }
            return count - 1 - back
        }
        var up = Array(repeating: 0, count: count)
        var down = Array(repeating: 0, count: count)
        for date in shared { if let i = index(date) { up[i] += 1 } }
        for date in kept { if let i = index(date) { down[i] += 1 } }
        return (0 ..< count).map { GlassDayColumn(id: String($0), up: up[$0], down: down[$0]) }
    }

    /// Sessions worth a second look: nothing matched in them, or they were
    /// trimmed to fit (`QueueShieldState`'s attention, counted).
    static func flagged(_ pending: [DaemonData.QueueEntry]) -> Int {
        pending.filter { entry in
            let reasons = entry.secondLook ?? []
            return reasons.contains("nothing-matched") || reasons.contains("trimmed-to-fit")
        }.count
    }

    /// One recent-activity row.
    struct Recent: Identifiable, Equatable {
        enum Kind: Equatable { case waiting, contributed, call }
        let id: String
        let kind: Kind
        let at: Date
        let tool: GlassTool?
        let text: String
        /// Set for a call that left without proof.
        let trailing: String?
    }

    /// The newest few things that happened: sessions waiting, contributions
    /// with the core's word for their status, and calls that left for an
    /// outside model with their proof label.
    static func recent(
        pending: [DaemonData.QueueEntry], history: [DaemonData.HistoryRow], calls: [DaemonData.InferenceCall],
        statusLabel: (String?) -> String?, limit: Int = 3
    ) -> [Recent] {
        var rows: [Recent] = []
        for entry in pending {
            guard let at = entry.startedAt ?? entry.discoveredAt else { continue }
            rows.append(Recent(
                id: "pending:\(entry.entryId)", kind: .waiting, at: at, tool: tool(entry.declaredSource ?? entry.source),
                text: "\(entry.projectLabel) · \(MonitorWords.waiting)", trailing: nil))
        }
        for row in history {
            // A row with no status is still activity: the shared table
            // reads it as unavailable, as the History list does.
            guard let at = row.submittedAt else { continue }
            rows.append(Recent(
                id: "history:\(row.submissionId)", kind: .contributed, at: at, tool: row.source.flatMap(tool),
                text: [row.projectLabel ?? "—", statusLabel(row.status)].compactMap { $0 }.joined(separator: " · "),
                trailing: nil))
        }
        for call in calls where call.route == "outside" {
            rows.append(Recent(
                id: "call:\(call.id)", kind: .call, at: call.at, tool: tool(call.tool),
                text: "\(InferenceTabView.toolName(call.tool)) · \(call.model)",
                trailing: call.proofLabel.isProof ? nil : InferenceWords.proof(call.proofLabel)))
        }
        return Array(rows.sorted { $0.at > $1.at }.prefix(limit))
    }

    static func tool(_ source: String) -> GlassTool? {
        SourceKind(rawValue: source).map(TracesTreeView.glassTool)
    }
}
