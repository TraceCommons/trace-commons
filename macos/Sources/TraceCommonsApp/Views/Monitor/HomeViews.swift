#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The Home tab (R9 of #1173): watching state, the counts, and the way
/// into History (spec, "Screens": History is Home, then History).
///
/// Counts come from the core: waiting is `decisions_owed` (a dash when the
/// core did not say), contributed is the rollup's accepted count. Pending
/// credit is shown only beside the commons' own statement of what it waits
/// on (D6), and never as earned.
struct HomeTabView: View {
    enum Page: String {
        case overview
        case history
    }

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String) -> String?
    @Binding var page: Page

    var body: some View {
        switch page {
        case .overview:
            HomeOverview(store: store, traces: traces, statusLabel: statusLabel, openHistory: { page = .history })
        case .history:
            HistoryPage(store: store, statusLabel: statusLabel, back: { page = .overview })
        }
    }
}

private struct HomeOverview: View {
    let store: HomeStore
    let traces: TracesStore
    let statusLabel: (String) -> String?
    let openHistory: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                watching
                HStack(spacing: GlassTokens.Space.s3) {
                    GlassLegendCell(MonitorWords.waiting, value: HomeFormat.count(store.status?.decisionsOwed), status: .ask)
                    GlassLegendCell(MonitorWords.contributed, value: HomeFormat.count(store.rollup?.allTime?.accepted), status: .shared)
                }
                GlassEyebrowCard(MonitorWords.history, action: openHistory) {
                    Image(systemName: "chevron.right")
                        .glassGlyph(10, weight: .semibold)
                        .foregroundStyle(GlassColor.textTertiary)
                } content: {
                    recent
                }
            }
        }
        .scrollIndicators(.never)
    }

    /// Paused, or watching N tools, from the core's status and the tree.
    /// The stack-wide state (ScreenState) for the watching row: the core's
    /// line when it does not answer, a dash before the first read or when
    /// the core did not say whether watching is paused, Paused, or
    /// watching N tools. A missing signal is never drawn as watching.
    private var state: ScreenState {
        HomeFormat.watchingState(store)
    }

    private var watching: some View {
        let state = state
        return GlassCard {
            HStack(spacing: GlassTokens.Space.s4) {
                switch state {
                case .ready:
                    switch HomeFormat.watching(store.status, destinations: store.destinations) {
                    case .signedOut:
                        GlassStatusDot(.ask, ring: true)
                        Text(MonitorWords.signedOut)
                    case .unhealthy(let label):
                        GlassStatusDot(.outside, ring: true)
                        Text(HealthCopy.forLabel(label).title)
                    case .watching(let tools):
                        GlassStatusDot(.on, ring: true)
                        Text(FlowMapScene.pair(MonitorWords.watching, tools))
                    }
                case .paused:
                    GlassStatusDot(.ask, ring: true)
                    Text(MonitorWords.paused)
                case .coreDown:
                    GlassStatusDot(.outside, ring: true)
                    Text(store.failures["status"].flatMap { MonitorWords.table?.line(for: $0) } ?? "—")
                case .loading, .unknown:
                    // No dot: unknown is never drawn as on, or as off.
                    Text("—").accessibilityLabel(MonitorWords.unknown)
                }
                Spacer(minLength: 0)
            }
            .glassType(GlassTokens.TypeScale.bodyStrong)
            .foregroundStyle(GlassColor.textPrimary)
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var recent: some View {
        let rows = Array((store.history ?? []).prefix(2))
        if store.history == nil {
            Text("—").glassType(GlassTokens.TypeScale.label).foregroundStyle(GlassColor.textTertiary)
        } else if rows.isEmpty {
            Text("0").glassType(GlassTokens.TypeScale.label).foregroundStyle(GlassColor.textTertiary)
        } else {
            VStack(spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                    GlassTableRow(first: index == 0) { HistoryRowView(row: row, statusLabel: statusLabel, compact: true) }
                }
            }
        }
    }
}

/// History: every contribution from this machine, newest first, with its
/// status and how it was approved. Withdrawing a contribution stays in the
/// shipping window until C1 carries the withdrawal call.
private struct HistoryPage: View {
    let store: HomeStore
    let statusLabel: (String) -> String?
    let back: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassBreadcrumb(
                [GlassCrumb(MonitorWindowView.Tab.home.title, action: back), GlassCrumb(MonitorWords.history)],
                backLabel: MonitorWindowView.Tab.home.title, onBack: back)
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    if let failure = store.failures["list_history"] {
                        GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                    }
                    if let rows = store.history {
                        if let cap = HomeFormat.cap(rows.count, rollup: store.rollup) {
                            Text(cap)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                        }
                        if rows.isEmpty {
                            Text("0")
                                .glassType(GlassTokens.TypeScale.number)
                                .foregroundStyle(GlassColor.textTertiary)
                        } else {
                            GlassCard {
                                VStack(spacing: 0) {
                                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                                        GlassTableRow(first: index == 0) { HistoryRowView(row: row, statusLabel: statusLabel, compact: false) }
                                    }
                                }
                            }
                        }
                    } else if store.failures["list_history"] == nil {
                        ProgressView().controlSize(.small).frame(maxWidth: .infinity)
                    }
                }
            }
            .scrollIndicators(.never)
        }
    }
}

/// One contribution: folder, day and tool, its status tag, and (in full)
/// how it was approved and its credit.
struct HistoryRowView: View {
    let row: DaemonData.HistoryRow
    let statusLabel: (String) -> String?
    let compact: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            VStack(alignment: .leading, spacing: 1) {
                Text(row.projectLabel ?? "—")
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                Text(HomeFormat.meta(row, compact: compact))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                if !compact {
                    // Its own line: the core's label for contributing
                    // automatically is long, and must not be cut short.
                    Text(HomeFormat.provenance(row.provenance))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: GlassTokens.Space.s4)
            if !compact, let figure = HomeFormat.credit(row) {
                Text(figure)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            if let status = row.status {
                GlassTag(statusLabel(status) ?? status, tone: HomeFormat.tone(status))
                    .fixedSize()
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The inspector on Home: the record as a whole. Counts by period, what is
/// held for review (apart, never as rejected), credit, and standing.
struct HomeSummaryInspector: View {
    let store: HomeStore

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                Text(MonitorWords.summary)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                if let failure = store.failures["history_rollup"] {
                    GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                }
                GlassEyebrowCard(MonitorWords.contributed) {
                    GlassKeyValueList([
                        .init(MonitorWords.week, HomeFormat.count(store.rollup?.week?.accepted)),
                        .init(MonitorWords.month, HomeFormat.count(store.rollup?.month?.accepted)),
                        .init(MonitorWords.allTime, HomeFormat.count(store.rollup?.allTime?.accepted)),
                        .init(MonitorWords.heldForReview, HomeFormat.count(store.rollup?.quarantined)),
                        .init(MonitorWords.withdrawn, HomeFormat.count(store.rollup?.takenBack)),
                    ])
                    // Held is never rejected: the core's sentence for it, beside
                    // the count, whenever something is held.
                    if (store.rollup?.quarantined ?? 0) > 0 {
                        Text(MonitorWords.heldExplanation)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textTertiary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                credit
                if let community = store.rollup?.community {
                    GlassEyebrowCard(MonitorWords.community) {
                        GlassKeyValueList([
                            .init(MonitorWords.rank, community.rank.map { "#\($0)" } ?? "—"),
                            .init(MonitorWords.window, community.windowLabel ?? "—"),
                        ])
                    }
                }
            }
        }
        .scrollIndicators(.never)
    }

    /// Final credit, and pending credit only with the commons' statement of
    /// its condition beside it (D6). Without that statement the pending
    /// figure is a dash: a bare number would read as owed.
    private var credit: some View {
        let condition = HomeFormat.pendingCondition(store.credit)
        return GlassEyebrowCard(MonitorWords.credit) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                GlassKeyValueList([
                    .init(MonitorWords.final, HomeFormat.points(store.rollup?.creditFinal)),
                    .init(MonitorWords.pending, condition == nil ? "—" : HomeFormat.points(store.rollup?.creditPending)),
                ])
                if let condition {
                    Text(condition)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // Credit is a record, not currency: the core's sentence,
                // beside every figure.
                Text(MonitorWords.creditNotCurrency)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// Formatting for Home and History. Pure, so the rules are tested.
enum HomeFormat {
    /// A count, or a dash when the core did not say. Zero is a number.
    static func count(_ value: Int?) -> String {
        value.map(String.init) ?? "—"
    }

    static func points(_ value: Double?) -> String {
        value.map { $0.formatted(.number.precision(.fractionLength(1))) } ?? "—"
    }

    /// What pending credit waits on, in the commons' own words: the only
    /// condition the shell may show it with. Nil when the commons has not
    /// said, and then pending credit is not shown as a figure.
    static func pendingCondition(_ credit: DaemonData.CommonsCreditSummary?) -> String? {
        guard let credit, credit.postureKnown,
              let explanation = credit.commonsSettlementExplanation, !explanation.isEmpty else { return nil }
        return explanation
    }

    /// Day and tool, then (in full) the size uploaded and when a
    /// withdrawal was seen. Month and day even when compact: a weekday
    /// alone reads a months-old row as this week. How it was approved is a
    /// line of its own.
    static func meta(_ row: DaemonData.HistoryRow, compact: Bool) -> String {
        var parts: [String] = []
        if let date = row.submittedAt { parts.append(day(date)) }
        if let source = row.source { parts.append(InferenceTabView.toolName(source)) }
        if !compact {
            if let bytes = row.uploadedBytes {
                parts.append(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))
            }
            if let withdrawn = row.revokedAt ?? row.withdrawnAt {
                parts.append("\(MonitorWords.withdrawn) \(day(withdrawn))")
            }
        }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
    }

    static func day(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day())
    }

    /// What Home's watching row says when the screen is ready.
    enum Watching: Equatable {
        /// The core says the contributor is not signed in.
        case signedOut
        /// The core reports a health label.
        case unhealthy(String)
        /// Watching this many tools, as the core counts them.
        case watching(Int)
    }

    /// The watching row's state (the stack-wide ScreenState): the core's
    /// line when it does not answer, a dash before the first read, when a
    /// read failed, or when the core did not say whether watching is
    /// paused, whether the contributor is signed in, or which tools it
    /// reads. A missing signal is never drawn as watching.
    @MainActor
    static func watchingState(_ store: HomeStore) -> ScreenState {
        let failure = store.failures["status"] ?? store.failures["tool_destinations"]
        return ScreenState.resolve(
            failure: failure, loaded: store.status != nil || failure != nil,
            paused: store.status?.paused,
            known: store.status?.loggedIn != nil && store.destinations != nil)
    }

    /// Signed out, then unhealthy, then watching N: the order
    /// `MenuBarStatus.state` uses. N counts the tools the core reads, unset
    /// ones included (`watchedCount`).
    static func watching(_ status: DaemonData.Status?, destinations: DaemonData.ToolDestinations?) -> Watching {
        if status?.loggedIn == false { return .signedOut }
        if let label = status?.health?.lastErrorLabel, !label.isEmpty { return .unhealthy(label) }
        return .watching(destinations?.watchedCount ?? 0)
    }

    /// History reads one page. When the page is full it says how many of
    /// the total it shows, in the core's words; the total is the rollup's
    /// all-time count, unknown when any part of it is.
    static func cap(_ shown: Int, rollup: DaemonData.HistoryRollup?) -> String? {
        guard shown >= HomeStore.historyLimit else { return nil }
        let all = rollup?.allTime
        let parts = [all?.submitted, all?.accepted, all?.quarantined, all?.withdrawn, all?.other]
        let total = parts.contains(nil) ? nil : parts.compactMap { $0 }.reduce(0, +)
        return MonitorWords.table?.historyCap(shown: shown, total: total)
    }

    /// How a contribution was approved. Not recorded is said as such, never
    /// as approved by the person.
    static func provenance(_ provenance: DaemonData.Provenance) -> String {
        switch provenance {
        case .armed: ProjectCopy.modeChoiceLabel(.autoUpload)
        case .personApproved: MonitorWords.approved
        case .notRecorded: MonitorWords.unrecorded
        }
    }

    /// The row's own credit: final when scored, zero included, otherwise
    /// nothing. A pending figure needs its condition, which lives in the
    /// summary, not per row.
    static func credit(_ row: DaemonData.HistoryRow) -> String? {
        row.creditPointsFinal.map(points)
    }

    /// Accepted reads as done; held for review and submitted as waiting;
    /// withdrawn as neutral. Held is never drawn as rejected.
    static func tone(_ status: String) -> GlassTag.Tone {
        switch status {
        case "accepted": .on
        case "submitted", "quarantined": .ask
        default: .neutral
        }
    }
}

/// Home's and History's words, from the core's table (`MonitorWords.table`).
extension MonitorWords {
    static var history: String { table?.history ?? "" }
    static var contributed: String { table?.contributed ?? "" }
    static var watching: String { table?.watching ?? "" }
    static var paused: String { table?.paused ?? "" }
    static var summary: String { table?.summary ?? "" }
    static var week: String { table?.week ?? "" }
    static var month: String { table?.month ?? "" }
    static var allTime: String { table?.total ?? "" }
    static var held: String { table?.held ?? "" }
    static var heldForReview: String { table?.heldForReview ?? "" }
    static var heldExplanation: String { table?.heldExplanation ?? "" }
    static var creditNotCurrency: String { table?.creditNotCurrency ?? "" }
    static var signedOut: String { table?.signedOut ?? "" }
    static var withdrawn: String { table?.withdrawn ?? "" }
    static var credit: String { table?.credit ?? "" }
    static var final: String { table?.creditFinal ?? "" }
    static var pending: String { table?.pending ?? "" }
    static var community: String { table?.community ?? "" }
    static var rank: String { table?.rank ?? "" }
    static var window: String { table?.window ?? "" }
    static var approved: String { table?.approved ?? "" }
    static var unrecorded: String { table?.unrecorded ?? "" }
}
#endif
