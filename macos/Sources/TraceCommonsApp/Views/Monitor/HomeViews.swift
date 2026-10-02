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
        case missions
    }

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String) -> String?
    @Binding var page: Page

    var body: some View {
        switch page {
        case .overview:
            HomeOverview(
                store: store, traces: traces, statusLabel: statusLabel,
                openHistory: { page = .history }, openMissions: { page = .missions })
        case .history:
            HistoryPage(store: store, statusLabel: statusLabel, back: { page = .overview })
        case .missions:
            MissionsPage(store: store, back: { page = .overview })
        }
    }
}

private struct HomeOverview: View {
    let store: HomeStore
    let traces: TracesStore
    let statusLabel: (String) -> String?
    let openHistory: () -> Void
    let openMissions: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let failure = store.failures["status"] {
                    GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                }
                watching
                HStack(spacing: GlassTokens.Space.s3) {
                    GlassLegendCell(MonitorWords.waiting, value: HomeFormat.count(store.status?.decisionsOwed), status: .ask)
                    GlassLegendCell(MonitorWords.contributed, value: HomeFormat.count(store.rollup?.allTime?.accepted), status: .shared)
                }
                GlassEyebrowCard(MonitorWords.missions, action: openMissions) {
                    Image(systemName: "chevron.right")
                        .glassGlyph(10, weight: .semibold)
                        .foregroundStyle(GlassColor.textTertiary)
                } content: {
                    // The catalogue's size only: nothing here says a mission
                    // was matched or chosen for this person (#1174 M1).
                    Text(MissionFormat.count(store.missions))
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
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
    private var watching: some View {
        let paused = store.status?.paused == true
        let tools = traces.tree.tools.filter { $0.mode == .watch }.count
        return GlassCard {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassStatusDot(paused ? .ask : .on, ring: true)
                Text(paused ? MonitorWords.paused : FlowMapScene.pair(MonitorWords.watching, tools))
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer(minLength: 0)
            }
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
                [GlassCrumb(MonitorWindowView.Tab.home.rawValue, action: back), GlassCrumb(MonitorWords.history)],
                backLabel: MonitorWindowView.Tab.home.rawValue, onBack: back)
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    if let failure = store.failures["list_history"] {
                        GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                    }
                    if let rows = store.history {
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
                    GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                }
                GlassEyebrowCard(MonitorWords.contributed) {
                    GlassKeyValueList([
                        .init(MonitorWords.week, HomeFormat.count(store.rollup?.week?.accepted)),
                        .init(MonitorWords.month, HomeFormat.count(store.rollup?.month?.accepted)),
                        .init(MonitorWords.allTime, HomeFormat.count(store.rollup?.allTime?.accepted)),
                        .init(MonitorWords.held, HomeFormat.count(store.rollup?.quarantined)),
                        .init(MonitorWords.withdrawn, HomeFormat.count(store.rollup?.takenBack)),
                    ])
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

    /// Day and tool. How it was approved is a line of its own.
    static func meta(_ row: DaemonData.HistoryRow, compact: Bool) -> String {
        var parts: [String] = []
        if let date = row.submittedAt {
            parts.append(compact ? date.formatted(.dateTime.weekday(.abbreviated)) : date.formatted(.dateTime.month(.abbreviated).day()))
        }
        if let source = row.source { parts.append(InferenceTabView.toolName(source)) }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
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

    /// The row's own credit: final when scored, otherwise nothing. A pending
    /// figure needs its condition, which lives in the summary, not per row.
    static func credit(_ row: DaemonData.HistoryRow) -> String? {
        guard let final = row.creditPointsFinal, final > 0 else { return nil }
        return points(final)
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

extension MonitorWords {
    static let history = "History"
    static let contributed = "Contributed"
    static let watching = "Watching"
    static let paused = "Paused"
    static let summary = "Summary"
    static let week = "Week"
    static let month = "Month"
    static let allTime = "Total"
    static let held = "Held"
    static let withdrawn = "Withdrawn"
    static let credit = "Credit"
    static let final = "Final"
    static let pending = "Pending"
    static let community = "Community"
    static let rank = "Rank"
    static let window = "Window"
    static let approved = "Approved"
    static let unrecorded = "Unrecorded"
}
#endif
