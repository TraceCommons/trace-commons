import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The inspector with nothing selected (Ron's #1146 `SummaryInspector`,
/// `waiting-page.tsx:175-273`): what is waiting, what went, today's upload
/// budget and the queue's safeguards, then the certificates held and why
/// sessions stopped waiting.
///
/// Every sentence is the core's (`summary_panel`, `counts`, `safeguards`,
/// the screens table); every count is the core's, and one it did not give
/// is a dash. Pending credit is shown only beside the commons' statement of
/// what it waits on (D6). The health banners are not repeated here: they
/// are drawn above the Traces tree (owner, 2026-10-07).
struct SummaryInspector: View {
    let traces: TracesStore
    let home: HomeStore
    /// The legacy queue's entries, for the certificate list.
    let awaitingDecision: [QueueEntry]
    /// Whether the queue has answered at all: an unread queue is not an
    /// empty one, so no certificate list is drawn before it.
    let queueAnswered: Bool
    /// `queue_outcome_counts`, by outcome label.
    let outcomeCounts: [String: Int]

    private var words: MonitorTracesCopy? { traces.words }
    /// Counts come from a tree the core gave this load; a failed or
    /// unfinished load counts nothing.
    private var loaded: Bool { traces.phase == .loaded }
    private var sessions: [DaemonData.QueueEntry] { traces.tree.allSessions }
    /// History, credit and the rollup as of the last read only when that
    /// read worked: HomeStore keeps the last good value on a failure, and
    /// a stale one is not shown as current.
    private var history: [DaemonData.HistoryRow]? {
        SummaryFacts.fresh(home.history, unless: home.failures["list_history"])
    }
    private var credit: DaemonData.CommonsCreditSummary? {
        SummaryFacts.fresh(home.credit, unless: home.failures["commons_credit_summary"])
    }
    private var rollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(home.rollup, unless: home.failures["history_rollup"])
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                header
                // The legend says its words or is not drawn, as Decisions.
                if let table = MonitorWords.table {
                    HStack(spacing: GlassTokens.Space.s3) {
                        GlassLegendCell(
                            table.shared, value: SummaryFacts.contributed(rollup), status: .shared)
                        GlassLegendCell(
                            table.kept, value: SummaryFacts.kept(loaded: loaded, sessions: sessions), status: .kept)
                    }
                }
                if let failure = home.failures["history_rollup"], let table = MonitorWords.table {
                    GlassNotice(tone: .outside, title: table.line(for: failure)) { EmptyView() }
                }
                if let words {
                    InspectorSection(words.inspector.decisions, collapsible: true) { decisions(words) }
                    statistics(words)
                }
                safeguards
                // An unread queue is not an empty one: before the daemon has
                // answered, nothing (`CertificateSection` says when there
                // are none).
                if queueAnswered {
                    CertificateSection(entries: awaitingDecision)
                }
                NotOfferedGlassDisclosure(counts: outcomeCounts, words: words?.summaryPanel)
            }
        }
        .scrollIndicators(.never)
    }

    /// Ron's `InspectorHeader`: "Summary" over the tools watched, projects
    /// and sessions waiting, each only once the core has said it.
    private var header: some View {
        InspectorHeader(
            title: MonitorWords.summary,
            sub: words.map {
                SummaryFacts.subline(
                    words: $0, destinations: traces.destinations, loaded: loaded,
                    folders: traces.tree.allFolders.count, sessions: sessions.count)
            })
    }

    /// Waiting, worth a second look, contributed with its pending credit,
    /// and today's uploads, in Ron's order.
    @ViewBuilder
    private func decisions(_ words: MonitorTracesCopy) -> some View {
        let fill = { (template: String, count: String) in FirstRunCopy.fill(template, ["count": count]) }
        // Ron's list: 8 between lines (`gap-2`).
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            SummaryDecisionLine(glyph: "tray", tone: .ask, text: fill(
                words.summaryPanel.waitingForYou, SummaryFacts.waiting(traces.status)))
            SummaryDecisionLine(glyph: "exclamationmark.triangle", tone: .ask, text: fill(
                words.summaryPanel.worthASecondLook, SummaryFacts.secondLook(loaded: loaded, sessions: sessions)))
            SummaryDecisionLine(glyph: "checkmark", tone: .on, text: fill(
                words.counts.contributedCount, SummaryFacts.contributed(rollup)))
            pending
            if let uploads = SummaryFacts.uploads(traces.status) {
                SummaryDecisionLine(glyph: "pause", tone: nil, text: FirstRunCopy.fill(
                    words.summaryPanel.uploadsToday, ["count": uploads.count, "max": uploads.max]))
            }
        }
    }

    /// Pending credit, as a figure only beside the commons' condition for
    /// it (D6); a dash otherwise.
    private var pending: some View {
        let pending = SummaryFacts.pending(rollup: rollup, credit: credit)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            GlassKeyValueList([.init(MonitorWords.pending, pending.value)])
            if let condition = pending.condition {
                Group {
                    Text(condition)
                    // Credit is a record, not currency: the core's
                    // sentence, beside every figure.
                    Text(MonitorWords.creditNotCurrency)
                }
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// Top projects and top tools, by sessions waiting plus contributed.
    /// Tools are counted from each session's own tool, not a tree level.
    @ViewBuilder
    private func statistics(_ words: MonitorTracesCopy) -> some View {
        if loaded {
            let projects = SummaryFacts.topProjects(
                folders: traces.tree.allFolders, history: history, counts: words.counts)
            let tools = SummaryFacts.topTools(sessions: sessions, history: history, counts: words.counts)
            if !projects.isEmpty || !tools.isEmpty {
                InspectorSection(words.summaryPanel.statistics, collapsible: true) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                        if !projects.isEmpty { SummaryStat(label: words.summaryPanel.topProjects, lines: projects) }
                        if !tools.isEmpty { SummaryStat(label: words.summaryPanel.topTools, lines: tools) }
                    }
                }
            }
        }
    }

    /// The daily limit and inference routing (Ron's `QueueStatusPanel`).
    /// What is holding sessions back is the host's banners, drawn above.
    @ViewBuilder
    private var safeguards: some View {
        if let copy = MonitorWords.table?.safeguards, let status = traces.status,
            status.dailyBudget != nil || status.routing != nil
        {
            GlassEyebrowCard(copy.eyebrow) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(copy.heading)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    GlassKeyValueList(SummaryFacts.safeguardRows(status, copy: copy))
                    if let held = SummaryFacts.heldByLimit(status.dailyBudget, copy: copy) {
                        Text(held)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                    if let unreadable = SummaryFacts.unreadableRows(status.routing, copy: copy) {
                        Text(unreadable)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                }
            }
        }
    }
}

/// One decision line: a glyph in its status colour, and the core's line.
private struct SummaryDecisionLine: View {
    let glyph: String
    let tone: GlassStatus?
    let text: String

    var body: some View {
        // Ron's `DecisionLine`: the glyph in a 20 column, 10 before the line.
        HStack(spacing: GlassTokens.Space.s5) {
            Image(systemName: glyph)
                .glassGlyph(12, weight: .semibold)
                .foregroundStyle(tone?.textColor ?? GlassColor.textTertiary)
                .frame(width: 20)
                .accessibilityHidden(true)
            Text(text)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A statistic: its label, then up to two named lines (Ron's `Stat`).
private struct SummaryStat: View {
    let label: String
    let lines: [SummaryFacts.Stat]

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(label)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
            // Ron's `Stat`: 2 under the label, 6 between its lines.
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                ForEach(lines) { line in
                    VStack(alignment: .leading, spacing: 0) {
                        Text(line.title)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(line.sub)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                }
            }
        }
    }
}

/// The Summary's figures, pure so the rules are tested: a count the core
/// did not give is a dash, never zero and never derived from another.
@MainActor
enum SummaryFacts {
    /// A statistic's line: a project or a tool, and its counts.
    struct Stat: Equatable, Identifiable {
        let id: String
        let title: String
        let sub: String
    }

    /// The history statuses that still stand as contributed: in the
    /// commons, waiting to be scored, or held for privacy review (Ron's
    /// `isContributed`). Withdrawn and expired records are not counted.
    static let contributedStatuses: Set<String> = ["accepted", "submitted", "quarantined"]

    /// Decisions owed, the badge's count; a dash when the core did not say.
    /// Never `queue_depth`.
    static func waiting(_ status: DaemonData.Status?) -> String {
        HomeFormat.count(status?.decisionsOwed)
    }

    /// Sessions waiting that are worth a second look, as the queue shield
    /// counts them; a dash before the tree is read.
    static func secondLook(loaded: Bool, sessions: [DaemonData.QueueEntry]) -> String {
        guard loaded else { return "—" }
        return String(sessions.filter { !($0.secondLook ?? []).isEmpty }.count)
    }

    /// Sessions waiting in the tree; a dash before it is read.
    static func kept(loaded: Bool, sessions: [DaemonData.QueueEntry]) -> String {
        loaded ? String(sessions.count) : "—"
    }

    /// Contributed, all time, as Ron's `isContributed` counts it and as
    /// the statistics below count it: accepted, waiting to be scored and
    /// held for privacy review. A part the core did not give is a dash.
    static func contributed(_ rollup: DaemonData.HistoryRollup?) -> String {
        let all = rollup?.allTime
        let parts = [all?.accepted, all?.submitted, all?.quarantined]
        return HomeFormat.count(parts.contains(nil) ? nil : parts.compactMap { $0 }.reduce(0, +))
    }

    /// A value the store holds, or nil when its last read failed: the
    /// store keeps the previous value, which is no longer current.
    static func fresh<Value>(_ value: Value?, unless failure: DaemonDataError?) -> Value? {
        failure == nil ? value : nil
    }

    /// The history page the statistics count, or nil when it is unread or
    /// the daemon capped it (`HomeStore.historyLimit`): a capped page is
    /// not a whole count, so it counts nothing rather than a part.
    static func wholeHistory(_ history: [DaemonData.HistoryRow]?) -> [DaemonData.HistoryRow]? {
        guard let history, history.count < HomeStore.historyLimit else { return nil }
        return history
    }

    /// Pending credit and the commons' condition for it (D6): a figure only
    /// with the condition, a dash without.
    static func pending(
        rollup: DaemonData.HistoryRollup?, credit: DaemonData.CommonsCreditSummary?
    ) -> (value: String, condition: String?) {
        let condition = HomeFormat.pendingCondition(credit)
        guard condition != nil, let figure = rollup?.creditPending else { return ("—", condition) }
        return (HomeFormat.points(figure), condition)
    }

    /// Today's uploads and the daily maximum; nil when the core reported no
    /// budget, a dash for a count it left out.
    static func uploads(_ status: DaemonData.Status?) -> (count: String, max: String)? {
        guard let budget = status?.dailyBudget else { return nil }
        return (HomeFormat.count(budget.uploadsToday), HomeFormat.count(budget.maxUploadsPerDay))
    }

    /// The header's sub line: tools watched, projects, sessions waiting. A
    /// part the core has not said is left out, never filled with a dash.
    static func subline(
        words: MonitorTracesCopy, destinations: DaemonData.ToolDestinations?, loaded: Bool, folders: Int,
        sessions: Int
    ) -> String {
        var parts: [String] = []
        if let destinations {
            parts.append(FirstRunCopy.fill(words.summaryPanel.toolsWatched, [
                "count": String(destinations.watchedCount), "total": String(destinations.tools.count),
            ]))
        }
        if loaded {
            parts.append(folders == 1
                ? words.counts.projectCountOne
                : FirstRunCopy.fill(words.counts.projectCount, ["count": String(folders)]))
            parts.append(sessions == 1
                ? words.counts.sessionsWaitingOne
                : FirstRunCopy.fill(words.counts.sessionsWaiting, ["count": String(sessions)]))
        }
        return parts.joined(separator: " · ")
    }

    /// The two projects with the most waiting and contributed, with the
    /// folder's mode. Contributed is a dash when history is unread or
    /// capped, and the ranking is then on waiting alone.
    static func topProjects(
        folders: [TracesTree.FolderNode], history: [DaemonData.HistoryRow]?, counts: MonitorCountsCopy
    ) -> [Stat] {
        let history = wholeHistory(history)
        let ranked = folders.map { folder -> (TracesTree.FolderNode, Int?) in
            (folder, history.map { rows in
                rows.filter { $0.projectId == folder.id && contributedStatuses.contains($0.status ?? "") }.count
            })
        }
        .filter { $0.0.sessions.count + ($0.1 ?? 0) > 0 }
        .sorted {
            let left = $0.0.sessions.count + ($0.1 ?? 0)
            let right = $1.0.sessions.count + ($1.1 ?? 0)
            return left != right ? left > right : $0.0.label < $1.0.label
        }
        return ranked.prefix(2).map { folder, contributed in
            var parts = [
                FirstRunCopy.fill(counts.waitingCount, ["count": String(folder.sessions.count)]),
                FirstRunCopy.fill(counts.contributedCount, ["count": HomeFormat.count(contributed)]),
            ]
            if let mode = folder.mode {
                let word = ProjectCopy.modeChoiceLabel(mode)
                if !word.isEmpty { parts.append(word) }
            }
            return Stat(id: folder.id, title: folder.label, sub: parts.joined(separator: " · "))
        }
    }

    /// The two tools with the most waiting and contributed, each session
    /// counted under its own tool. Contributed is a dash when history is
    /// unread or capped, and the ranking is then on waiting alone.
    static func topTools(
        sessions: [DaemonData.QueueEntry], history: [DaemonData.HistoryRow]?, counts: MonitorCountsCopy
    ) -> [Stat] {
        let history = wholeHistory(history)
        var waiting: [String: Int] = [:]
        for session in sessions { waiting[session.declaredSource ?? session.source, default: 0] += 1 }
        var contributed: [String: Int] = [:]
        for row in history ?? [] where contributedStatuses.contains(row.status ?? "") {
            guard let source = row.source else { continue }
            contributed[source, default: 0] += 1
        }
        let tools = Set(waiting.keys).union(contributed.keys)
            .map { ($0, (waiting[$0] ?? 0) + (contributed[$0] ?? 0)) }
            .filter { $0.1 > 0 }
            .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : InferenceTabView.toolName($0.0) < InferenceTabView.toolName($1.0) }
        return tools.prefix(2).map { tool, _ in
            Stat(
                id: tool, title: InferenceTabView.toolName(tool),
                sub: [
                    FirstRunCopy.fill(counts.waitingCount, ["count": String(waiting[tool] ?? 0)]),
                    FirstRunCopy.fill(counts.contributedCount, [
                        "count": history == nil ? "—" : String(contributed[tool] ?? 0),
                    ]),
                ].joined(separator: " · "))
        }
    }

    /// The safeguards panel's rows: the daily limit as uploads and
    /// megabytes left (Ron's), and inference routing in the core's state
    /// sentence.
    static func safeguardRows(_ status: DaemonData.Status, copy: MonitorSafeguardsCopy) -> [GlassKeyValueList.Item] {
        var rows: [GlassKeyValueList.Item] = []
        if let budget = status.dailyBudget {
            rows.append(.init(copy.dailyLimit, FirstRunCopy.fill(copy.remaining, [
                "uploads": HomeFormat.count(budget.uploadsRemaining),
                "megabytes": HomeFormat.count(budget.bytesRemaining.map(megabytes)),
            ])))
        }
        if let routing = status.routing {
            let state = TCRoutingCopy.stateLine(state: routing.state) ?? "—"
            rows.append(.init(copy.inferenceRouting, routing.derived == true ? "\(state) · \(copy.daemonOwned)" : state))
        }
        return rows
    }

    /// Whole megabytes, rounded as Ron rounds them.
    static func megabytes(_ bytes: Int) -> Int {
        Int((Double(bytes) / 1024 / 1024).rounded())
    }

    /// The approved sessions a spent daily limit holds back, when it holds
    /// any; nil when the limit is not reached or the count is unsaid (the
    /// spent-limit banner above says so in that case).
    static func heldByLimit(_ budget: DaemonData.DailyBudget?, copy: MonitorSafeguardsCopy) -> String? {
        guard let budget, budget.blocked == true, let held = budget.blockedEntries, held > 0 else { return nil }
        return held == 1 ? copy.heldByLimitOne : FirstRunCopy.fill(copy.heldByLimit, ["count": String(held)])
    }

    /// Ledger rows the daemon could not read, when there are any.
    static func unreadableRows(_ routing: DaemonData.Routing?, copy: MonitorSafeguardsCopy) -> String? {
        guard let rows = routing?.unreadableRows, rows > 0 else { return nil }
        return rows == 1 ? copy.rowsUnavailableOne : FirstRunCopy.fill(copy.rowsUnavailable, ["count": String(rows)])
    }
}
