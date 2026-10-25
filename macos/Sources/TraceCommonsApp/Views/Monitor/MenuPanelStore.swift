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
    private(set) var history: [DaemonData.HistoryRow] = []
    private(set) var calls: [DaemonData.InferenceCall] = []
    /// `inference_activity`: Private AI calls and verified calls per
    /// four-hour step, for the menu-bar item's bars. Nil until read.
    private(set) var activity: DaemonData.InferenceActivity?
    /// True until a load has read `status`, and again whenever that read
    /// fails, there is no client, or the client's event stream ends: the
    /// panel cannot vouch for anything it holds. Every value is gated on it.
    private(set) var stale = true
    /// The other reads that failed on the last load. Each gates only the
    /// values drawn from it (`current(_:)`): a failed history read leaves
    /// the mode and the waiting count drawn, and its own values unknown.
    private(set) var unreadable: Set<MenuPanelData.Read> = []
    /// How long `run` waits before re-reading and re-subscribing after the
    /// event stream ends, so a daemon that is truly gone is not hot-looped.
    var resubscribeDelay: Duration = .seconds(5)
    /// Today's routed calls per tool (`insights_glance`). Read on its own:
    /// no answer here ever stales the popover, since the glance is off by
    /// default and an older daemon does not serve it.
    private(set) var glance: MenuPanelData.GlanceState = .notRead
    /// How often `run` re-reads `inference_activity` with no event: its steps
    /// end at the moment of the read, so an idle daemon's would otherwise
    /// stay where the last event left them.
    var activityRefresh: Duration = .seconds(60)
    /// The clock the menu-bar item's span is read from; a test sets its own.
    var clock: () -> Date = { Date() }
    /// When the item's span was last read from `clock`: on each load and
    /// each `activityRefresh`, so the label is redrawn as time passes even
    /// when no read changes anything.
    private(set) var stripNow = Date()
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

    // MARK: The nudge row

    /// The lead suggestion's row, in the daemon's words; nil while the
    /// panel is stale or there is none.
    var nudgeRow: NudgeSurface.PanelRow? {
        stale ? nil : NudgeSurface.panelRow(status?.nudge)
    }

    /// The machine-wide settlement estimate's line; nil while the panel is
    /// stale or the daemon reported none.
    var settlementLine: MenuPanelStatus.SettlementLine? {
        MenuPanelStatus.settlementLine(status, available: !stale)
    }

    /// The row was tapped: records that the suggestion was opened, then
    /// answers where to open the Monitor. The stamp is best effort here;
    /// a refused one still opens the place, since looking changes nothing.
    func open(_ row: NudgeSurface.PanelRow) async -> MonitorDestination? {
        let effect = NudgeSurface.effect(row.intent)
        if let client {
            try? await NudgeSurface.send(effect, through: client)
        }
        return effect.destination.map(MonitorDestination.init(nudge:))
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
    /// every folder back to its own setting, so when the override is Ask
    /// or Never and any folder's own setting is Automatic (or is not known),
    /// the core's clear confirmation is shown first, as for an override:
    /// unattended sending never resumes from a single menu press. Under an
    /// Automatic override clearing resumes nothing, so it, like a clear with
    /// no Automatic folder, happens at once.
    /// Without the core's confirmation nothing is cleared.
    func chooseMixed() async {
        guard canChooseOverride, status?.contributionOverride != nil else { return }
        overrideRefusal = nil
        // A folder list that did not answer on the last read is unknown,
        // which needs the confirmation (fail closed), never the last list.
        let folders = current(.projects) ? projects : nil
        guard MenuPanelData.clearNeedsConfirmation(folders, override: status?.contributionOverride?.mode) else {
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
        await writeOverride { _ = try await $0.setContributionOverride(mode: mode, confirm: mode == .autoUpload) }
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

    /// Reads, then follows the client's events until the task is cancelled
    /// (the label's `.task(id:)` cancels it when the client changes). A
    /// stream that ends marks the data stale and, after `resubscribeDelay`,
    /// is read and opened again: it used to end `run` for good, which left
    /// the panel stale against a daemon that was answering, until a new
    /// client was attached.
    func run() async {
        while !Task.isCancelled {
            await load()
            guard let client else { return }
            await followWhileRefreshingActivity(client)
            if Task.isCancelled { return }
            // The stream ended: the daemon went away, or the stream could
            // not be opened. Stale until a read says otherwise.
            stale = true
            do { try await Task.sleep(for: resubscribeDelay) } catch { return }
        }
    }

    /// Follows the events and, beside them, re-reads `inference_activity`
    /// every `activityRefresh`, until the stream ends; the timer stops with
    /// it.
    private func followWhileRefreshingActivity(_ client: any DaemonDataClient) async {
        let refreshing = Task { await self.followActivity() }
        await follow(client)
        refreshing.cancel()
    }

    /// Re-reads the activity every `activityRefresh` until cancelled.
    private func followActivity() async {
        while !Task.isCancelled {
            do { try await Task.sleep(for: activityRefresh) } catch { return }
            await loadActivity()
        }
    }

    /// Reads `inference_activity` alone, as `load` reads it: a failure marks
    /// it unknown, a method the daemon does not have yet is not a failure.
    /// Moves `stripNow` on whatever the read answers.
    func loadActivity() async {
        defer { stripNow = clock() }
        guard let client else { return }
        do {
            activity = try await client.inferenceActivity()
            unreadable.remove(.activity)
        } catch DaemonDataError.notAvailableYet {
            return
        } catch {
            unreadable.insert(.activity)
        }
    }

    /// Re-reads on each event until the client's stream ends.
    private func follow(_ client: any DaemonDataClient) async {
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired, .inferenceCallAdded:
                await load()
            case .digestDue, .reengageDue, .previewReady, .unknown:
                break
            case .usageChanged:
                await loadGlance()
            }
        }
    }

    /// Reads everything. A failed read marks its own data unknown rather
    /// than keeping the last value as if it were current; only a failed
    /// `status` read makes the whole panel stale. A method the daemon does
    /// not have yet is not a failure.
    func load() async {
        guard let client else {
            stale = true
            glance = .unavailable
            return
        }
        var failed: Set<MenuPanelData.Read> = []
        func read<T>(_ which: MenuPanelData.Read, _ call: () async throws -> T) async -> T? {
            do { return try await call() } catch DaemonDataError.notAvailableYet {
                return nil
            } catch {
                failed.insert(which)
                return nil
            }
        }
        if let value = await read(.status, { try await client.status() }) { status = value }
        if let value = await read(.projects, { try await client.listProjects() }) { projects = value.projects }
        if let value = await read(.pending, { try await client.listPending(projectId: nil) }) {
            pending = value
        } else {
            // Even an unsupported read is unknown, never a fresh empty queue.
            failed.insert(.pending)
        }
        if let value = await read(.history, { try await client.listHistory(limit: HomeStore.historyLimit) }) {
            history = value
        }
        if let value = await read(.calls, { try await client.inferenceCalls(limit: 20, cursor: nil) }) {
            calls = value.readable ? value.calls : []
        }
        if let value = await read(.activity, { try await client.inferenceActivity() }) { activity = value }
        unreadable = failed
        stale = failed.contains(.status)
        stripNow = clock()
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

    // MARK: What each value is drawn from

    /// Whether one read's data is current: the status is (the panel is not
    /// stale) and that read answered last time. Each value is gated on its
    /// own read, so one read that fails never blanks another's value.
    func current(_ read: MenuPanelData.Read) -> Bool {
        !stale && !unreadable.contains(read)
    }

    /// The mode pill's roll-up, from `status`; `none` (a dash) while there
    /// is no current status.
    var modeRollup: MenuPanelData.ModeRollup {
        stale ? .none : MenuPanelData.rollup(status?.contributionMode)
    }

    /// Shared in the graph's days, from `list_history`; nil when unknown.
    var sharedCount: Int? {
        current(.history) ? buckets.reduce(0) { $0 + $1.shared } : nil
    }

    /// Sessions still waiting here in the graph's days, from `list_pending`,
    /// as Traces counts them. Explicit Keep decisions are a separate list.
    /// Nil when the pending read did not answer.
    var keptCount: Int? {
        current(.pending) ? buckets.reduce(0) { $0 + $1.kept } : nil
    }

    /// Whether the trace bars are current: both of the reads they are
    /// drawn from answered. Otherwise the menu-bar item draws them as
    /// unavailable.
    var columnsCurrent: Bool { current(.history) && current(.pending) }

    /// The graph's buckets: shared and kept per day over the Traces graph's
    /// default days, counted as that graph counts them (`TracesGraphModel`,
    /// owner, 2026-10-22: the popover's graph matches the app's). A read
    /// that is not current draws no bars, never its last ones.
    var buckets: [TracesGraphModel.Bucket] {
        TracesGraphModel.buckets(
            shared: current(.history) ? TracesGraphModel.sharedTimes(history, tool: nil) : [],
            kept: current(.pending) ? TracesGraphModel.keptTimes(pending, tool: nil) : [],
            offset: 0, now: clock(), range: .days)
    }

    /// The menu-bar item's Private AI segments: verified calls (up) over
    /// all calls (down) per step. Nil while unknown -- unread, failed,
    /// refused by an older daemon, or no ledger answered -- which draws
    /// none, never empty ones.
    var stripPrivateAI: [GlassDayColumn]? {
        guard current(.activity), let activity else { return nil }
        return MenuPanelData.privateAISteps(activity)
    }

    /// The menu-bar item's trace segments: shared (up) and kept (down) per
    /// step, on the Private AI steps' own clock while they are drawn, so
    /// all four share one span; else the same span ending now.
    var stripTraces: [GlassDayColumn] {
        MenuPanelData.steps(
            shared: history.compactMap(\.submittedAt),
            kept: current(.pending) ? TracesGraphModel.keptTimes(pending, tool: nil) : [],
            span: stripSpan)
    }

    /// The item's span: the Private AI steps' own while they are drawn,
    /// else the same span ending at `stripNow`.
    var stripSpan: MenuPanelData.Span {
        let span = stripPrivateAI == nil ? nil : activity.flatMap(MenuPanelData.span)
        return span ?? MenuPanelData.Span(end: stripNow)
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

    /// The daemon reads the panel makes on each load, each gating only the
    /// values drawn from it (`MenuPanelStore.current(_:)`).
    enum Read: Hashable, Sendable {
        case status, projects, pending, history, calls, activity
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

    /// A count the panel draws: the number, or the core's word for a value
    /// it did not report (`MonitorWords.unknown`), never a bare dash that
    /// reads as a broken panel.
    static func countWord(_ count: Int?, unknown: String) -> String {
        count.map(String.init) ?? unknown
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
        // arming, Ask folders go back to asking).
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

    /// The popover graph's bars: the Traces graph's (`TracesGraphFooter.bar`),
    /// with every other day's label left out, counting back from today's,
    /// so the weekdays fit under the popover's narrower columns.
    static func bars(_ buckets: [TracesGraphModel.Bucket], words: MonitorTracesGraphCopy?) -> [GlassBarBucket] {
        buckets.enumerated().map { index, bucket in
            var bucket = bucket
            if (buckets.count - 1 - index) % 2 != 0 { bucket.label = "" }
            return TracesGraphFooter.bar(bucket, words: words)
        }
    }

    /// The menu-bar item's span: `count` steps of `stepHours`, the last
    /// ending at `end`. Six steps of four hours by default, the window
    /// `inference_activity` counts.
    struct Span: Equatable {
        static let steps = 6
        static let stepHours = 4
        var end: Date
        var count = Span.steps
        var stepHours = Span.stepHours
    }

    /// The Private AI steps' own span: the last step's end, and their count
    /// and step. Nil while no ledger answered, no steps came, or the step
    /// is not a length of time.
    static func span(_ activity: DaemonData.InferenceActivity) -> Span? {
        let stepHours = activity.stepHours ?? Span.stepHours
        guard activity.readable, let last = activity.steps.last, stepHours > 0 else { return nil }
        return Span(
            end: last.start.addingTimeInterval(TimeInterval(stepHours * 3600)),
            count: activity.steps.count, stepHours: stepHours)
    }

    /// The Private AI steps as the strip's columns: verified (up) over all
    /// calls (down). Nil while no ledger answered: unknown, never zero.
    /// Nil too when the steps have no span (`span(_:)`), so they are never
    /// drawn beside trace steps counted on another clock.
    static func privateAISteps(_ activity: DaemonData.InferenceActivity) -> [GlassDayColumn]? {
        guard span(activity) != nil else { return nil }
        return activity.steps.enumerated().map { index, step in
            GlassDayColumn(id: String(index), up: step.verified, down: step.calls)
        }
    }

    /// One column per step of `span`, oldest first: how many sessions were
    /// contributed (up) and kept (down) in it. A date after the span's end
    /// counts in the last step, as `inference_activity` counts a call
    /// stamped after its read; one before the first step is outside it.
    static func steps(shared: [Date], kept: [Date], span: Span) -> [GlassDayColumn] {
        guard span.count > 0, span.stepHours > 0 else { return [] }
        let step = TimeInterval(span.stepHours * 3600)
        let start = span.end.addingTimeInterval(-step * Double(span.count))
        func index(_ date: Date) -> Int? {
            let offset = date.timeIntervalSince(start)
            guard offset >= 0 else { return nil }
            return min(Int(offset / step), span.count - 1)
        }
        var up = Array(repeating: 0, count: span.count)
        var down = Array(repeating: 0, count: span.count)
        for date in shared { if let i = index(date) { up[i] += 1 } }
        for date in kept { if let i = index(date) { down[i] += 1 } }
        return (0 ..< span.count).map { GlassDayColumn(id: String($0), up: up[$0], down: down[$0]) }
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
