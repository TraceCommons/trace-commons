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
    /// Declared top-level (`MonitorNavigation.swift`) so a destination can
    /// name a page in a release build.
    typealias Page = HomePage

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String?) -> String?
    @Binding var page: Page
    /// The selected History row's submission id; empty for none. The
    /// inspector shows its details.
    @Binding var selection: String

    var body: some View {
        switch page {
        case .overview:
            HomeOverview(
                store: store, traces: traces, statusLabel: statusLabel,
                openHistory: { page = .history }, openMissions: { page = .missions })
        case .history:
            HistoryPage(store: store, statusLabel: statusLabel, selection: $selection, back: { page = .overview })
        case .missions:
            MissionsPage(store: store, back: { page = .overview })
        }
    }
}

private struct HomeOverview: View {
    let store: HomeStore
    let traces: TracesStore
    let statusLabel: (String?) -> String?
    let openHistory: () -> Void
    let openMissions: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
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
                        Text(HealthCopy.core(label: label, maxQueueEntries: nil).title)
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
/// status and how it was approved, and what is held for review, explained
/// apart. Withdraw, the session detail and Skills are in the selected
/// row's inspector (`HistoryDetailInspector`).
private struct HistoryPage: View {
    let store: HomeStore
    let statusLabel: (String?) -> String?
    @Binding var selection: String
    let back: () -> Void
    @EnvironmentObject private var model: AppModel

    // The app's own records, which the inspector's Withdraw and session
    // detail resolve against, are read again on every visit: the daemon's
    // view, not the one it had at launch (the legacy screen's rule).
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
                    held
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
                                        selectable(row, first: index == 0)
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
        .onAppear { model.refreshHistory() }
    }

    /// Held for review, never as rejected: the core's sentence, no promised
    /// wait, the server's distinct reasons without digests, and why there is
    /// no bulk action. Only when the rollup counts something held and the
    /// core's words are there (never an empty title). The reasons span every
    /// record the app holds, not the list's capped page.
    @ViewBuilder
    private var held: some View {
        if (store.rollup?.quarantined ?? 0) > 0, let words = MonitorWords.table {
            GlassNotice(tone: .ask, title: words.heldForReview) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(words.heldExplanation)
                    Text(HistoryLegacyWords.typicalWait)
                    ForEach(HeldExplanations.lines(in: model.history.filter { $0.status == "quarantined" }.map(\.explanations)),
                            id: \.self) { line in
                        Text(line)
                    }
                    Text(WithdrawalCopy.noBulkAction)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// A row a click selects, highlighted across its full width; the
    /// inspector shows the selected row's details.
    private func selectable(_ row: DaemonData.HistoryRow, first: Bool) -> some View {
        let selected = selection == row.submissionId
        return Button(action: { selection = row.submissionId }) {
            GlassTableRow(first: first) { HistoryRowView(row: row, statusLabel: statusLabel, compact: false) }
                .background(
                    RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                        .fill(selected ? GlassColor.ink(0.12) : Color.clear)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }
}

/// One contribution: folder, day and tool, its status tag, and (in full)
/// how it was approved and its credit.
struct HistoryRowView: View {
    let row: DaemonData.HistoryRow
    let statusLabel: (String?) -> String?
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
            if let tag = HomeFormat.statusWord(row.status, label: statusLabel) {
                GlassTag(tag, tone: HomeFormat.tone(row.status))
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

    /// A row's status tag, from the core's one status table. A missing or
    /// empty status reads the core's unavailable word too. With no core copy
    /// decoded there is no tag at all -- never the raw wire token, which is
    /// not a word a contributor was meant to read.
    static func statusWord(_ status: String?, label: (String?) -> String?) -> String? {
        label(status.flatMap { $0.isEmpty ? nil : $0 })
    }

    /// The word for a status, from the core's one table (`PublicRunCopy`).
    /// A missing or empty status reads the core's unavailable word; with no
    /// core copy decoded there is no word at all, never the raw token.
    static func historyStatusLabel(copy: PublicRunCopy?, _ status: String?) -> String? {
        guard let copy else { return nil }
        guard let status, !status.isEmpty else { return copy.contributionStatusUnavailable }
        return copy.historyStatusLabel(for: status)
    }

    /// Accepted reads as done; held for review and submitted as waiting;
    /// withdrawn as neutral. Held is never drawn as rejected.
    static func tone(_ status: String?) -> GlassTag.Tone {
        switch status {
        case "accepted": .on
        case "submitted", "processing", "quarantined": .ask
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
