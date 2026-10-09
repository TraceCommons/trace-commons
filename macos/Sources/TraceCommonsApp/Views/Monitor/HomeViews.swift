import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Home tab (R9 of #1173, in Ron's #1146 shape): the status card with
/// its way into Traces, the counts, Missions, and the way into History
/// (spec, "Screens": History is Home, then History).
///
/// Counts come from the core: waiting is `decisions_owed` (a dash when the
/// core did not say), contributed is the rollup's accepted count. Pending
/// credit is shown only beside the commons' own statement of what it waits
/// on (D6), and never as earned.
///
/// Missions is #1146's: the local mission drafts, and its card opens them.
/// The two screens #1146 has no place for stay reachable from quiet cards
/// under History: Insights, and the commons mission catalogue once the
/// daemon answers it.
struct HomeTabView: View {
    /// Declared top-level (`MonitorNavigation.swift`) so a destination can
    /// name a page in a release build.
    typealias Page = HomePage

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String?) -> String?
    @Binding var page: Page
    /// The opened History row's submission id; empty for none. Opening a
    /// row shows its details in the inspector (Ron's inspector auto-open).
    @Binding var selection: String
    /// The status card's "Open Traces" link: switches the window's tab.
    var openTraces: () -> Void = {}
    /// The hosted Insights and Mission drafts screens' inputs, from
    /// `TraceCommonsAppMain`.
    let insightsStoreSelection: InsightsStoreSelection
    let missionDrafts: MissionDraftsModel

    /// The core's Insights copy, read once: it is fixed for the life of the
    /// process, and reading it is a C ABI call and a JSON decode, which
    /// `body` would otherwise repeat on every evaluation.
    private static let insightsCopy = TCInsights.copy()

    /// The breadcrumb word for a hosted page: the core's heading, or nil
    /// before its copy has arrived. Mission drafts is #1146's Missions, and
    /// its crumb says so; the drafts screen's own title is the core's
    /// (`missionDrafts.copy["title"]`), read by the screen.
    static func hostedHeading(_ page: Page, missionDrafts: MissionDraftsModel) -> String? {
        switch page {
        case .insights: HomeFormat.cardHeading(Self.insightsCopy?["title"])
        case .missionDrafts: HomeFormat.cardHeading(MonitorWords.missions)
        case .overview, .history, .missions: nil
        }
    }

    var body: some View {
        // History and Missions are reached from here and lead back here; the
        // breadcrumb that says so is the shell's, under the tabs
        // (`MonitorWindowView.breadcrumb`), as #1146 draws it.
        switch page {
        case .overview:
            HomeOverview(
                store: store, traces: traces, statusLabel: statusLabel, openTraces: openTraces,
                insightsHeading: Self.hostedHeading(.insights, missionDrafts: missionDrafts),
                missionDrafts: missionDrafts,
                openHistory: { page = .history }, openMissionDrafts: { page = .missionDrafts },
                openInsights: { page = .insights }, openMissions: { page = .missions })
        case .history:
            HistoryPage(store: store, statusLabel: statusLabel, selection: $selection, back: { page = .overview })
        case .missions:
            MissionsPage(store: store, back: { page = .overview })
        // The screens built outside the Monitor (R15: the legacy window
        // that held them is gone), hosted as Home pages under the shell's
        // breadcrumb.
        case .insights:
            InsightsView(storeSelection: insightsStoreSelection)
        case .missionDrafts:
            MissionDraftsView(model: missionDrafts)
        }
    }
}

private struct HomeOverview: View {
    let store: HomeStore
    let traces: TracesStore
    let statusLabel: (String?) -> String?
    let openTraces: () -> Void
    /// The core's Insights heading; nil until the core's copy has arrived,
    /// and then its card is not drawn.
    let insightsHeading: String?
    let missionDrafts: MissionDraftsModel
    let openHistory: () -> Void
    let openMissionDrafts: () -> Void
    let openInsights: () -> Void
    let openMissions: () -> Void

    /// The rollup, or nil after a failed `history_rollup` read
    /// (`SummaryFacts.fresh`): a stale one is not shown as current.
    private var freshRollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
    }

    /// The credit summary, or nil after a failed `commons_credit_summary`
    /// read: the earlier settlement sentence is not shown as current.
    private var freshCredit: DaemonData.CommonsCreditSummary? {
        SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
    }

    private var words: MonitorHomeHistoryCopy? { MonitorWords.table?.homeHistory }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                watching
                stats
                // Ron's Missions card: the local drafts, under his Drafts tag.
                GlassEyebrowCard(MonitorWords.missions, action: openMissionDrafts) {
                    HStack(spacing: GlassTokens.Space.s3) {
                        if let words { GlassTag(words.draftsTag, tone: .ask).fixedSize() }
                        HomeChevron()
                    }
                } content: {
                    drafts
                }
                GlassEyebrowCard(MonitorWords.history, action: openHistory) {
                    HStack(spacing: GlassTokens.Space.s3) {
                        // Ron's "X credit pending", under D6: only beside
                        // the commons' statement of what it waits on.
                        if let pending = HomeFormat.creditPendingAccessory(
                            freshRollup?.creditPending, condition: HomeFormat.pendingCondition(freshCredit)) {
                            Text(pending)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                                .fixedSize()
                        }
                        HomeChevron()
                    }
                } content: {
                    recent
                }
                // What #1146 has no place for, after its cards: Insights,
                // and the commons catalogue once the daemon answers it.
                if let insightsHeading {
                    hostedCard(insightsHeading, action: openInsights)
                }
                if store.missions != nil, let catalogue = words?.missionCatalogue {
                    hostedCard(catalogue, action: openMissions)
                }
            }
        }
        .scrollIndicators(.never)
        .task { missionDrafts.preview() }
    }

    /// A way into a screen #1146 has no place for: its heading and a
    /// chevron, and nothing else (the screen holds its own words).
    private func hostedCard(_ heading: String, action: @escaping () -> Void) -> some View {
        GlassEyebrowCard(heading, action: action) {
            HomeChevron()
        } content: {
            EmptyView()
        }
    }

    /// Paused, or watching N tools, from the core's status and the tree.
    /// The stack-wide state (ScreenState) for the watching row: the core's
    /// line when it does not answer, a dash before the first read or when
    /// the core did not say whether watching is paused, Paused, or
    /// watching N tools. A missing signal is never drawn as watching.
    private var state: ScreenState {
        HomeFormat.watchingState(store)
    }

    /// Waiting, contributed and pending credit (#1146 `home-view.tsx`), each
    /// from the core: waiting is decisions owed, contributed the rollup's
    /// accepted count. Pending credit is a figure only beside the commons'
    /// statement of what it waits on (D6), drawn under the tiles; without
    /// that statement it is a dash, never a bare number that reads as owed.
    ///
    /// Waiting is the Traces store's count, the one the Traces badge reads:
    /// one waiting number. A failed rollup or credit read is a dash, never
    /// the store's earlier value (`freshRollup`, `freshCredit`).
    private var stats: some View {
        let condition = HomeFormat.pendingCondition(freshCredit)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                HomeStatTile(label: MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed))
                HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(freshRollup?.allTime?.accepted))
                HomeStatTile(label: HomeFormat.creditPendingWord,
                             value: HomeFormat.pendingFigure(freshRollup?.creditPending, condition: condition))
            }
            .fixedSize(horizontal: false, vertical: true)
            if let condition {
                Text(condition)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// Ron's status card: the dot, the watching line in 600, and under it
    /// the sessions waiting and how many are worth a second look, 12 apart.
    private var watching: some View {
        let state = state
        return GlassCard {
            HStack(spacing: GlassTokens.Space.s6) {
                // #1146 `home-view.tsx`: the dot is the two lines' sibling,
                // so the second line starts under the first line's words,
                // after the dot, never under the dot.
                VStack(alignment: .homeStatusText, spacing: GlassTokens.Space.s1) {
                    status(state)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                    // Ron's #1146 second line: sessions waiting and how many
                    // are worth a second look. Only from the core's count:
                    // an unknown count draws no line, never "nothing".
                    if let line = HomeFormat.waitingLine(traces.decisionsOwed, sessions: traces.tree.allSessions) {
                        Text(line)
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textSecondary)
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    }
                }
                .accessibilityElement(children: .combine)
                Spacer(minLength: 0)
                // The way into the tree, as #1146's status card has it.
                Button(action: openTraces) {
                    HStack(spacing: GlassTokens.Space.s1) {
                        Text(HomeFormat.openTracesWord)
                        Image(systemName: "chevron.right").glassGlyph(9, weight: .semibold).accessibilityHidden(true)
                    }
                }
                .buttonStyle(GlassButtonStyle(.link))
                .fixedSize()
            }
            .foregroundStyle(GlassColor.textPrimary)
        }
    }

    @ViewBuilder
    private func status(_ state: ScreenState) -> some View {
        HStack(spacing: GlassTokens.Space.s6) {
                switch state {
                case .ready:
                    switch HomeFormat.watching(store.status, destinations: store.destinations) {
                    case .signedOut:
                        GlassStatusDot(.ask, ring: true)
                        Text(MonitorWords.signedOut)
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    case .unhealthy(let label):
                        GlassStatusDot(.outside, ring: true)
                        Text(HealthCopy.core(label: label, maxQueueEntries: nil).title)
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    case .watching(let tools):
                        GlassStatusDot(.on, ring: true)
                        Text(MonitorWords.table?.shell.watching(tools: tools) ?? FlowMapScene.pair(MonitorWords.watching, tools))
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    }
                case .paused:
                    GlassStatusDot(.ask, ring: true)
                    Text(MonitorWords.paused)
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                case .coreDown:
                    GlassStatusDot(.outside, ring: true)
                    Text(store.failures["status"].flatMap { MonitorWords.table?.line(for: $0) } ?? "—")
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                case .loading, .unknown:
                    // No dot: unknown is never drawn as on, or as off.
                    Text("—").accessibilityLabel(MonitorWords.unknown)
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                }
        }
    }

    /// Ron's Missions card body: up to two drafts, each its id over its
    /// source count with its status tag, a hairline between them; or his
    /// empty line. A dash before the inbox has answered: an unread inbox is
    /// never drawn as an empty one.
    @ViewBuilder
    private var drafts: some View {
        if let summary = missionDrafts.summary, let words {
            if summary.isEmpty {
                Text(words.noMissionDrafts)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
            } else {
                HomeCardRows(Array(summary.prefix(2))) { draft in
                    VStack(alignment: .leading, spacing: 0) {
                        Text(draft.id)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Text(words.sources(draft.source_count))
                            .glassType(GlassTokens.TypeScale.label.weight(.regular))
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                } tag: { draft in
                    // The drafts screen's own word for the status, from the
                    // core; never the raw token.
                    let word = missionDrafts.text(draft.status)
                    if !word.isEmpty { GlassTag(word).fixedSize() }
                }
            }
        } else {
            Text("—")
                .glassType(GlassTokens.TypeScale.label)
                .foregroundStyle(GlassColor.textTertiary)
                .accessibilityLabel(MonitorWords.unknown)
        }
    }

    /// Ron's History card body: the two newest contributions, each its
    /// folder over its day and tool with its toned status tag.
    @ViewBuilder
    private var recent: some View {
        let rows = Array((store.history ?? []).prefix(2))
        if store.history == nil {
            Text("—").glassType(GlassTokens.TypeScale.label).foregroundStyle(GlassColor.textTertiary)
        } else if rows.isEmpty {
            Text(MonitorWords.table?.shell.nothingContributed ?? "0")
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
        } else {
            HomeCardRows(rows) { row in
                VStack(alignment: .leading, spacing: 0) {
                    Text(row.projectLabel ?? "—")
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .lineLimit(1)
                    Text(HomeFormat.meta(row, compact: true))
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                        .lineLimit(1)
                }
            } tag: { row in
                if let tag = HomeFormat.statusWord(row.status, label: statusLabel) {
                    GlassTag(tag, tone: HomeFormat.tone(row.status)).fixedSize()
                }
            }
        }
    }
}

/// A Home card's chevron accessory.
private struct HomeChevron: View {
    var body: some View {
        Image(systemName: "chevron.right")
            .glassGlyph(10, weight: .semibold)
            .foregroundStyle(GlassColor.textTertiary)
            .accessibilityHidden(true)
    }
}

/// Rows inside a Home card (Ron's `home-view.tsx`): text on the left, a tag
/// on the right, and a hairline with 6pt above every row after the first.
private struct HomeCardRows<Item: Identifiable, Words: View, Tag: View>: View {
    let items: [Item]
    let words: (Item) -> Words
    let tag: (Item) -> Tag

    init(_ items: [Item], @ViewBuilder text: @escaping (Item) -> Words, @ViewBuilder tag: @escaping (Item) -> Tag) {
        self.items = items
        self.words = text
        self.tag = tag
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                HStack(spacing: GlassTokens.Space.s5) {
                    words(item)
                    Spacer(minLength: 0)
                    tag(item)
                }
                .padding(.top, index == 0 ? 0 : GlassTokens.Space.s3)
                .overlay(alignment: .top) {
                    if index > 0 { GlassHairline(GlassColor.hairline) }
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// History, in Ron's #1146 order, in the left pane: his page description,
/// the three stat cards, community standing, every contribution from this
/// machine under SUBMISSIONS, grouped by PROJECT, with Open and Withdraw on
/// its row, the credit record, and what is held for privacy review (apart,
/// never as rejected). The record by period, which #1146 has no place for,
/// closes the page. Opening a row shows its details in the inspector
/// (`HistoryDetailInspector`, Ron's inspector auto-open).
private struct HistoryPage: View {
    let store: HomeStore
    let statusLabel: (String?) -> String?
    @Binding var selection: String
    let back: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var filter: HistoryList.Filter = .all

    private var words: MonitorHomeHistoryCopy? { MonitorWords.table?.homeHistory }

    // The app's own records, which each row's Withdraw and the opened
    // row's detail resolve against, are read again on every visit: the
    // daemon's view, not the one it had at launch (the legacy screen's rule).
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let words {
                    Text(words.historyDescription)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                // A failed rollup read says so; the cells it feeds
                // read a dash, never the store's earlier counts.
                if let failure = store.failures["history_rollup"] {
                    GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                }
                // Verdict news, in the daemon's words. See history
                // acknowledges it; this page is already the history.
                if let card = store.verdictsCard {
                    NudgeGlassCard(
                        card: card, busy: store.nudgeBusy,
                        refusal: store.nudgeError.flatMap { MonitorWords.table?.line(for: $0) }
                    ) { intent in Task { await store.perform(intent) } }
                }
                // The one-time offer to turn verdict notifications on.
                NudgeOfferCards(place: .history)
                HistoryStats(store: store)
                HistoryCommunityCard(store: store)
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        submissionsHeader
                        refreshOutcome
                        list
                    }
                }
                HistoryCreditCard(store: store)
                held
                HistoryPeriodCard(store: store)
            }
        }
        .scrollIndicators(.never)
        .onAppear {
            model.clearHistoryRefresh()
            model.refreshHistory()
            model.refreshAccountSession()
        }
        // Escape goes back to Home, as the shell's breadcrumb does.
        .onExitCommand(perform: back)
    }

    /// The rollup, or nil after a failed `history_rollup` read: a stale one
    /// is not shown as current (`SummaryFacts.fresh`).
    private var rollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
    }

    /// Ron's SUBMISSIONS eyebrow over "Contribution history", with the
    /// refresh control beside them. Without the core's words, History's own
    /// word alone.
    private var submissionsHeader: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(words?.submissions ?? MonitorWords.history)
                    .glassType(GlassTokens.TypeScale.eyebrow)
                    .foregroundStyle(GlassColor.textTertiary)
                if let words {
                    Text(words.contributionHistory)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                }
            }
            Spacer(minLength: 0)
            refresh
        }
    }

    /// The refresh control (Ron's #1146 HistoryRefreshControl): asks the daemon to check the
    /// server sooner (`refresh_history`), then reads History again, the app's
    /// records and this screen's. Only with the core's words.
    @ViewBuilder
    private var refresh: some View {
        if let words = MonitorWords.table?.historyActions {
            Button(model.historyRefresh == .requesting ? words.requesting : words.requestRefresh) {
                Task { if await model.requestHistoryRefresh() { await store.load() } }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .fixedSize()
            .disabled(model.historyRefresh == .requesting)
        }
    }

    /// What the last refresh request did, in the core's words: asked, or
    /// refused. Nothing before one is made.
    @ViewBuilder
    private var refreshOutcome: some View {
        if let words = MonitorWords.table?.historyActions {
            switch model.historyRefresh {
            case .requested: Text(words.refreshRequested)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            case .failed: GlassStatusLabel(words.refreshFailed, status: .outside)
                .fixedSize(horizontal: false, vertical: true)
            case .idle, .requesting: EmptyView()
            }
        }
    }

    /// The core's filter labels, or none at all: a filter with an empty
    /// segment is never drawn, and then every row is shown.
    private var labels: [HistoryList.Filter: String]? {
        HistoryList.labels(shell: MonitorWords.table?.shell)
    }

    private var shownFilter: HistoryList.Filter {
        labels == nil ? .all : filter
    }

    @ViewBuilder
    private var list: some View {
        if let failure = store.failures["list_history"] {
            GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
        }
        if let rows = store.history {
            if let cap = HomeFormat.cap(rows.count, rollup: rollup) {
                Text(cap)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            if rows.isEmpty {
                Text(MonitorWords.table?.shell.historyEmpty ?? "0")
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                filterBar(rows)
                let groups = HistoryFolders.folders(
                    HistoryList.rows(rows, filter: shownFilter),
                    projectID: { $0.projectId ?? "" }, projectLabel: { $0.projectLabel ?? "—" })
                if groups.isEmpty {
                    Text(MonitorWords.table?.shell.historyFilterEmpty ?? "0")
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textTertiary)
                } else {
                    // Ron's groups: 24 apart, each after the first under a
                    // hairline, a PROJECT eyebrow over the folder, its
                    // record count, then its rows.
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s10) {
                        ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                            projectGroup(group, first: index == 0)
                        }
                    }
                    .padding(.top, GlassTokens.Space.s8)
                }
            }
        } else if store.failures["list_history"] == nil {
            // Ron's reading line while History is first read.
            if let words {
                Text(words.readingHistory)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                GlassSpinner(MonitorWords.history).frame(maxWidth: .infinity)
            }
        }
    }

    private func projectGroup(_ group: QueueGroup<DaemonData.HistoryRow>, first: Bool) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            HStack(alignment: .bottom, spacing: GlassTokens.Space.s9) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    if let words {
                        Text(words.project)
                            .glassType(GlassTokens.TypeScale.eyebrow)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                    Text(group.label)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Text(words?.records(group.count) ?? "\(group.count)")
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
                    .monospacedDigit()
                    .fixedSize()
            }
            .accessibilityElement(children: .combine)
            VStack(alignment: .leading, spacing: 1) {
                ForEach(group.entries) { row in
                    HistoryListRow(
                        row: row, statusLabel: statusLabel,
                        selected: selection == row.submissionId,
                        open: { selection = row.submissionId })
                }
            }
        }
        .padding(.top, first ? 0 : GlassTokens.Space.s9 - 2)
        .overlay(alignment: .top) {
            if !first { GlassHairline(GlassColor.hairline) }
        }
    }

    /// Ron's filter: All, then each status, with its count, as pills that
    /// wrap onto a second line rather than scroll out of sight.
    @ViewBuilder
    private func filterBar(_ rows: [DaemonData.HistoryRow]) -> some View {
        if let labels {
            let counts = HistoryList.counts(rows)
            HistoryFilterFlow(spacing: GlassTokens.Space.s2) {
                ForEach(HistoryList.Filter.allCases, id: \.self) { item in
                    HistoryFilterPill(
                        label: labels[item] ?? "", count: counts[item] ?? 0,
                        selected: filter == item, select: { filter = item })
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(words?.filterLabel ?? MonitorWords.history)
        }
    }

    /// Held for review, never as rejected (Ron's PRIVACY REVIEW card): the
    /// count as its heading, the core's sentence, no promised wait, the
    /// server's distinct reasons without digests, and why there is no bulk
    /// action. Only when the rollup counts something held and the core's
    /// words are there (never an empty title). The reasons span every
    /// record the app holds, not the list's capped page.
    @ViewBuilder
    private var held: some View {
        if let quarantined = rollup?.quarantined, quarantined > 0, let words = MonitorWords.table {
            GlassEyebrowCard(words.shell.filterQuarantined) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(words.homeHistory.held(quarantined))
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    Text(words.heldExplanation)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                    Group {
                        Text(HistoryLegacyWords.typicalWait)
                        ForEach(HeldExplanations.lines(in: model.history.filter { $0.status == "quarantined" }.map(\.explanations)),
                                id: \.self) { line in
                            Text(line)
                        }
                        Text(WithdrawalCopy.noBulkAction)
                    }
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// One History filter (Ron's `history-filter.tsx`): an 11pt pill, its label
/// and count; the chosen one on a lit fill in primary ink, the others
/// tertiary, lifting to primary under the pointer.
private struct HistoryFilterPill: View {
    let label: String
    let count: Int
    let selected: Bool
    let select: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: select) {
            HStack(spacing: GlassTokens.Space.s2) {
                Text(label)
                Text("\(count)").monospacedDigit().opacity(0.7)
            }
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(selected || hovering ? GlassColor.textPrimary : GlassColor.textTertiary)
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, GlassTokens.Space.s5)
            .padding(.vertical, 5)
            .background(Capsule().fill(selected ? GlassTokens.Color.controlSelected.color : Color.clear))
            .contentShape(Capsule())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Ron's `flex-wrap` for the filter: items left to right, onto the next
/// line when the next one would not fit.
struct HistoryFilterFlow: Layout {
    let spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let lines = Self.lines(width: proposal.width ?? .infinity, sizes: subviews.map { $0.sizeThatFits(.unspecified) },
                               spacing: spacing)
        let width = lines.map(\.width).max() ?? 0
        let height = lines.map(\.height).reduce(0, +) + spacing * CGFloat(max(lines.count - 1, 0))
        return CGSize(width: proposal.width.map { min($0, width) } ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        var y = bounds.minY
        for line in Self.lines(width: bounds.width, sizes: sizes, spacing: spacing) {
            var x = bounds.minX
            for index in line.items {
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sizes[index]))
                x += sizes[index].width + spacing
            }
            y += line.height + spacing
        }
    }

    /// The items on each line, and each line's width and height. Pure.
    static func lines(width: CGFloat, sizes: [CGSize], spacing: CGFloat) -> [(items: [Int], width: CGFloat, height: CGFloat)] {
        var lines: [(items: [Int], width: CGFloat, height: CGFloat)] = []
        var current: (items: [Int], width: CGFloat, height: CGFloat) = ([], 0, 0)
        for (index, size) in sizes.enumerated() {
            let needed = current.items.isEmpty ? size.width : current.width + spacing + size.width
            if !current.items.isEmpty, needed > width {
                lines.append(current)
                current = ([index], size.width, size.height)
            } else {
                current = (current.items + [index], needed, max(current.height, size.height))
            }
        }
        if !current.items.isEmpty { lines.append(current) }
        return lines
    }
}

/// One contribution in History's list (Ron's `HistoryRow`): a folder tile,
/// then what it is -- the folder, its tool and day, its status line, the
/// server's reasons and its settled credit -- with Open and Withdraw on the
/// right of the row, the one Withdraw on History's page. Withdraw asks
/// first, with the existing confirmation, and stands on the app's own record
/// of the contribution, so a row that record does not hold yet offers none
/// (fail closed). It decides from the status the opened detail decides
/// from, offers Retry after a failed withdrawal, and gives way to the core's
/// sign-in while no account session is active. Without the core's
/// public-run copy the row offers nothing.
private struct HistoryListRow: View {
    let row: DaemonData.HistoryRow
    let statusLabel: (String?) -> String?
    let selected: Bool
    let open: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var confirming = false

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // The actions beside the text while the text keeps a readable
            // column; stacked at the row's right, then under the text, as
            // the pane narrows. Never a label broken mid-word.
            ViewThatFits(in: .horizontal) {
                line(stacked: false)
                line(stacked: true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s7) {
                        tile
                        description
                    }
                    if let copy = model.publicRunCopy {
                        HStack(spacing: GlassTokens.Space.s4) { actions(copy) }
                    }
                }
            }
            if let copy = model.publicRunCopy {
                below(copy)
            }
        }
        // #1146's confirmation: a glass modal over the window, not a well
        // under the row (owner: glass modal for confirmations).
        .glassModal(isPresented: Binding(
            get: {
                confirming && control == .withdraw && record.flatMap { model.withdrawals[$0.submissionID] } == nil
            },
            set: { if !$0 { confirming = false } }
        )) {
            if let record, let copy = model.publicRunCopy {
                WithdrawalConfirmationModal(
                    status: SessionDetailView.withdrawalStatus(record, detail: model.sessionDetails[record.submissionID]),
                    keepLabel: copy.keepContribution,
                    inFlight: model.withdrawing.contains(record.submissionID),
                    onKeep: { confirming = false },
                    onConfirm: { model.withdraw(record) }
                )
            }
        }
        .padding(.vertical, GlassTokens.Space.s7)
        .padding(.horizontal, GlassTokens.Space.s2)
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(selected ? GlassColor.ink(0.12) : Color.clear)
        )
        .overlay(alignment: .bottom) {
            GlassHairline(GlassColor.hairline)
        }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    /// #1146's tile: the folder's first letter.
    private var tile: some View {
        GlassToolTile(.folderInitial(row.projectLabel ?? ""), large: true)
            .frame(width: 38, alignment: .leading)
    }

    private func line(stacked: Bool) -> some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s7) {
            tile
            description
                .frame(minWidth: 120, idealWidth: 120, maxWidth: .infinity, alignment: .leading)
            if let copy = model.publicRunCopy {
                let layout = stacked
                    ? AnyLayout(VStackLayout(alignment: .trailing, spacing: GlassTokens.Space.s3))
                    : AnyLayout(HStackLayout(spacing: GlassTokens.Space.s4))
                layout { actions(copy) }
                    .fixedSize()
            }
        }
    }

    /// The folder in 600, its tool and day, Ron's "Status:" line, the
    /// server's reasons, and the settled credit.
    private var description: some View {
        let words = MonitorWords.table?.homeHistory
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(row.projectLabel ?? "—")
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
            Text(HomeFormat.rowMeta(row))
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let word = HomeFormat.statusWord(row.status, label: statusLabel) {
                if let words {
                    Text(words.status(word))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    GlassTag(word, tone: HomeFormat.tone(row.status)).fixedSize()
                }
            }
            explanations
            if let figure = HomeFormat.credit(row), let words {
                Text(words.credit(figure))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// The server's own reasons for this row, without opaque digests; a held
    /// row the server said nothing about reads the core's held sentence
    /// (`MonitorScreensCopy.heldExplanation`); without the core's words it
    /// reads nothing, never a blank line.
    @ViewBuilder
    private var explanations: some View {
        let lines = HeldExplanations.lines(in: [row.explanations ?? []])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            if lines.isEmpty, row.status == "quarantined", !MonitorWords.heldExplanation.isEmpty {
                Text(MonitorWords.heldExplanation).fixedSize(horizontal: false, vertical: true)
            }
        }
        .glassType(GlassTokens.TypeScale.caption)
        .foregroundStyle(GlassColor.textSecondary)
    }

    private var control: HistoryList.RowWithdraw {
        let detail = record.flatMap { model.sessionDetails[$0.submissionID] }
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        return HistoryList.rowWithdraw(record: record, detail: detail, result: result, account: model.accountSession)
    }

    /// Open, and Withdraw (or what stands in its place) while no outcome is
    /// drawn under the row.
    @ViewBuilder
    private func actions(_ copy: PublicRunCopy) -> some View {
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        let control = control
        Button(MonitorWords.table?.shell.open ?? copy.viewSession, action: open)
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .frame(minHeight: 44)
        if result == nil {
            withdrawControl(control, copy: copy)
        }
    }

    /// What runs the row's full width under it: the withdrawal outcome and
    /// Retry, and sign-in's progress and failure lines. The confirmation is
    /// a glass modal over the window (`WithdrawalConfirmationModal`).
    @ViewBuilder
    private func below(_ copy: PublicRunCopy) -> some View {
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        let control = control
        if record != nil {
            if let result {
                if HistoryList.showsOutcome(result) {
                    WithdrawalOutcomeView(result: result)
                }
                withdrawControl(control, copy: copy)
            }
        }
        if control == .signIn, let words = MonitorWords.table?.historyActions {
            HistorySignInStatus(words: words)
        }
    }

    /// What stands in Withdraw's place (`HistoryList.rowWithdraw`). Sign-in
    /// and the account check need the core's words, and draw nothing
    /// without them.
    @ViewBuilder
    private func withdrawControl(_ control: HistoryList.RowWithdraw, copy: PublicRunCopy) -> some View {
        let words = MonitorWords.table?.historyActions
        switch control {
        // #1146: Withdraw reads in the outside ink.
        case .withdraw: Button(copy.withdraw) { confirming = true }
            .buttonStyle(GlassButtonStyle(.destructive, small: true))
            .frame(minHeight: 44)
            .disabled(record.map { model.withdrawing.contains($0.submissionID) } ?? true)
        // Retry withdraws without asking again, as the legacy
        // `SessionWithdrawalAction` does: the first attempt already asked.
        // (Ron's "Try again" asks again; native's behaviour is kept, under
        // his word.)
        case .retry:
            if let record {
                Button(WithdrawalCopy.tryAgain) { model.withdraw(record) }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .frame(minHeight: 44)
                    .disabled(model.withdrawing.contains(record.submissionID))
            }
        case .signIn:
            if let words {
                HistorySignInControl(words: words)
            }
        case .checkingAccount:
            if let words {
                Text(words.checkingAccount)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(minHeight: 44)
            }
        case .none:
            EmptyView()
        }
    }
}

/// Ron's `AccountSignInControl`: sign in through the native identity path
/// (the daemon opens the browser), and say so while it waits. The button
/// sits on the row; what it says while waiting, and why a sign-in did not
/// leave an active session, run under the row (`HistorySignInStatus`). All
/// in the core's words.
private struct HistorySignInControl: View {
    let words: MonitorHistoryActionsCopy
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Button(model.accountSigningIn ? words.waitingForSignIn : words.signInToWithdraw) { Task { await model.signInToWithdraw() } }
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .frame(minHeight: 44)
            .disabled(model.accountSigningIn)
    }
}

/// Sign-in's progress line and its failure, under the row.
private struct HistorySignInStatus: View {
    let words: MonitorHistoryActionsCopy
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if model.accountSigningIn { Text(words.completeSignIn)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            }
            if let failure = model.accountSignInFailure {
                GlassStatusLabel(line(failure), status: .outside)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func line(_ failure: AppModel.AccountSignInFailure) -> String {
        switch failure {
        case .inactive: words.signInInactive
        case .unverified: words.signInUnverified
        case .failed: words.signInFailed
        }
    }
}

/// History's stat cards (Ron's `HistoryPage`): pending credit under D6,
/// contributed, and held for review, as Home's three are drawn.
private struct HistoryStats: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash in every cell, never the earlier counts.
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        let credit = SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                HomeStatTile(label: HomeFormat.creditPendingWord,
                             value: HomeFormat.pendingFigure(rollup?.creditPending, credit: credit))
                HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(rollup?.allTime?.accepted))
                HomeStatTile(label: MonitorWords.held, value: HomeFormat.count(rollup?.quarantined))
            }
            .fixedSize(horizontal: false, vertical: true)
            if let condition = HomeFormat.pendingCondition(credit) {
                Text(condition)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// The list's rules, in Ron's terms (`history-view-model.ts`): the filter is
/// an exact status match counted over every row read, and its labels are
/// the core's (`historyUi`; "all" from the public-run copy).
enum HistoryList {
    enum Filter: String, CaseIterable, Hashable {
        case all
        case accepted
        case submitted
        case quarantined
        case withdrawn
    }

    /// Where the opened row's details are drawn, for scrolling to them.
    static let detailAnchor = "history.detail"

    static func counts(_ rows: [DaemonData.HistoryRow]) -> [Filter: Int] {
        var counts: [Filter: Int] = [.all: rows.count]
        for row in rows {
            if let status = row.status, let filter = Filter(rawValue: status), filter != .all {
                counts[filter, default: 0] += 1
            }
        }
        return counts
    }

    static func rows(_ rows: [DaemonData.HistoryRow], filter: Filter) -> [DaemonData.HistoryRow] {
        filter == .all ? rows : rows.filter { $0.status == filter.rawValue }
    }

    /// A filter's label from the core (Ron's #1146 `history-filter.tsx`
    /// words, `MonitorShellCopy`), or nil when the core has not said.
    static func label(_ filter: Filter, shell: MonitorShellCopy?) -> String? {
        let label: String? = switch filter {
        case .all: shell?.filterAll
        case .accepted: shell?.filterAccepted
        case .submitted: shell?.filterSubmitted
        case .quarantined: shell?.filterQuarantined
        case .withdrawn: shell?.filterWithdrawn
        }
        return label.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Every filter's label, or nil if any is missing.
    static func labels(shell: MonitorShellCopy?) -> [Filter: String]? {
        var labels: [Filter: String] = [:]
        for filter in Filter.allCases {
            guard let label = label(filter, shell: shell) else { return nil }
            labels[filter] = label
        }
        return labels
    }

    /// What a row offers in Withdraw's place.
    enum RowWithdraw: Equatable {
        /// Nothing: no record, a status the owner's rule does not withdraw,
        /// or a withdrawal already done.
        case none
        /// The account session is being read for the first time.
        case checkingAccount
        /// No active account session: sign in first (Ron's `HistoryRow`).
        case signIn
        /// Withdraw, which asks first.
        case withdraw
        /// Withdraw again after a withdrawal that did not finish.
        case retry
    }

    /// Whether a row draws its last withdrawal's outcome. A refusal for
    /// want of an account session is never drawn: the row offers sign-in,
    /// or Withdraw once signed in, in its place, and the legacy sentence
    /// says this build has no sign-in. Every other outcome is drawn.
    static func showsOutcome(_ result: AppModel.WithdrawalResult) -> Bool {
        result != .noAccountSession
    }

    /// Withdraw on a row stands on the app's own record and the owner's
    /// rule, asked of the status the session detail decides from (its
    /// read status, else the record's: `SessionDetailView.withdrawalStatus`).
    /// No record, no Withdraw. It offers Retry after a failed attempt, as
    /// the detail's action did, and needs an active account session: until
    /// one is known, the row offers sign-in instead.
    static func rowWithdraw(
        record: HistoryRecord?, detail: SessionDetail?, result: AppModel.WithdrawalResult?,
        account: AppModel.AccountSessionRead
    ) -> RowWithdraw {
        guard let record,
              ContributionStatusPresentation.offersWithdraw(SessionDetailView.withdrawalStatus(record, detail: detail))
        else { return .none }
        if case .withdrawn = result { return .none }
        switch account {
        case .signedIn: return result == nil ? .withdraw : .retry
        case .checking: return .checkingAccount
        case .unread, .signedOut: return .signIn
        }
    }
}

/// Community standing (Ron's `CommunityPanel`), when the rollup carries
/// one; nothing otherwise: its eyebrow and heading, then rank, novelty
/// credit, accepted in the commons' window and the accept rate, and the
/// commons' own word when aggregate analytics are withheld.
struct HistoryCommunityCard: View {
    let store: HomeStore

    var body: some View {
        // Nothing after a failed read: not the earlier standing.
        if let community = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])?.community {
            if let words = MonitorWords.table?.homeHistory {
                GlassCard(quiet: true) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                            Text(MonitorWords.community)
                                .glassType(GlassTokens.TypeScale.eyebrow)
                                .foregroundStyle(GlassColor.textTertiary)
                            Text(words.publicStanding)
                                .glassType(GlassTokens.TypeScale.title)
                                .foregroundStyle(GlassColor.textPrimary)
                                .accessibilityAddTraits(.isHeader)
                        }
                        HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                            cell(MonitorWords.rank, community.rank.map { "#\($0)" } ?? "—")
                            cell(words.noveltyCredit, HomeFormat.points(community.noveltyCredit))
                            cell(words.accepted(window: community.windowLabel ?? "—"),
                                 HomeFormat.count(community.acceptedInWindow))
                            cell(words.acceptRate, HomeFormat.rate(community.acceptRate))
                        }
                        .padding(.vertical, GlassTokens.Space.s4)
                        .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                        .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
                        if community.analyticsWithheld == true {
                            Text(words.analyticsWithheld)
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                                .foregroundStyle(GlassColor.textPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            } else {
                GlassEyebrowCard(MonitorWords.community) {
                    GlassKeyValueList([
                        .init(MonitorWords.rank, community.rank.map { "#\($0)" } ?? "—"),
                        .init(MonitorWords.window, community.windowLabel ?? "—"),
                    ])
                }
            }
        }
    }

    private func cell(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(label)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            Text(value)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .monospacedDigit()
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// The credit record (Ron's `CreditRecordPanel`): what credit is, then the
/// final figure and the one still being scored, or his chip before the
/// first sync. Pending credit is a figure only with the commons' statement
/// of its condition beside it (D6); without that statement it is a dash: a
/// bare number would read as owed.
struct HistoryCreditCard: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash, never the earlier figures or sentence.
        let credit = SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
        let condition = HomeFormat.pendingCondition(credit)
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        let words = MonitorWords.table?.homeHistory
        GlassEyebrowCard(words?.creditRecord ?? MonitorWords.credit) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                if let words {
                    Text(words.aboutCredit)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    Text(words.aboutCreditBody)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let words, let rollup, rollup.lastRefreshedAt == nil {
                    GlassChip(words.notSynced)
                } else {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s10) {
                        figure(MonitorWords.final, HomeFormat.points(rollup?.creditFinal))
                        figure(words?.stillBeingScored ?? MonitorWords.pending,
                               HomeFormat.pendingFigure(rollup?.creditPending, credit: credit))
                    }
                    .padding(.top, GlassTokens.Space.s7)
                    .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                }
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

    private func figure(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(label)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            Text(value)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .monospacedDigit()
        }
        .accessibilityElement(children: .combine)
    }
}

/// The record by period: the week's and month's accepted counts and the
/// withdrawn count, from the rollup. #1146's History has no place for it,
/// so it closes the page, quietly, rather than sitting among Ron's stat
/// cards.
private struct HistoryPeriodCard: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash in every row, never the earlier counts.
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        GlassEyebrowCard(MonitorWords.summary) {
            GlassKeyValueList([
                .init(MonitorWords.week, HomeFormat.count(rollup?.week?.accepted)),
                .init(MonitorWords.month, HomeFormat.count(rollup?.month?.accepted)),
                .init(MonitorWords.withdrawn, HomeFormat.count(rollup?.takenBack)),
            ])
        }
    }
}

/// One of Home's and History's three counts (Ron's `StatCard`): a full
/// card, 10 by 12 inside, its eyebrow word over an 18pt-class bold tabular
/// figure. The word wraps at a space, never inside a word.
private struct HomeStatTile: View {
    let label: String
    let value: String

    /// #1146's `StatCard`: a full card, the value bold and tabular.
    var body: some View {
        GlassStatCard(label, value: value)
    }
}

/// Formatting for Home and History. Pure, so the rules are tested.
enum HomeFormat {
    /// A card heading from the core's copy: the word, or nil when the copy
    /// has not arrived (or the word is empty), so no card is drawn on a
    /// word of this shell's own.
    static func cardHeading(_ word: String?) -> String? {
        guard let word, !word.isEmpty else { return nil }
        return word
    }

    /// A count, or a dash when the core did not say. Zero is a number.
    static func count(_ value: Int?) -> String {
        value.map(String.init) ?? "—"
    }

    static func points(_ value: Double?) -> String {
        value.map { $0.formatted(.number.precision(.fractionLength(1))) } ?? "—"
    }

    /// Home's pending-credit tile: the figure only with the commons'
    /// statement of what it waits on (D6), a dash otherwise.
    static func pendingFigure(_ value: Double?, condition: String?) -> String {
        condition == nil ? "—" : points(value)
    }

    /// The same rule from the commons' credit summary: a figure only when
    /// the commons has said what it waits on (D6); a dash otherwise, never
    /// a bare number that reads as owed.
    static func pendingFigure(_ pending: Double?, credit: DaemonData.CommonsCreditSummary?) -> String {
        pendingFigure(pending, condition: pendingCondition(credit))
    }

    /// Ron's #1146 second status line, from the core's decisions owed and
    /// the waiting sessions' second-look reasons. Nil when the core did not
    /// say how many are owed: an unknown count is never "nothing waiting".
    static func waitingLine(_ owed: Int?, sessions: [DaemonData.QueueEntry]) -> String? {
        guard let owed, let shell = MonitorWords.table?.shell else { return nil }
        let secondLook = sessions.filter { !($0.secondLook ?? []).isEmpty }.count
        return shell.waiting(owed, secondLook: secondLook)
    }

    /// History card's "X credit pending", under the same rule as the tile:
    /// only with the commons' statement of what it waits on (D6), and never
    /// for nothing pending.
    static func creditPendingAccessory(_ pending: Double?, condition: String?) -> String? {
        guard let pending, pending > 0, condition != nil,
              let template = MonitorWords.table?.shell.creditPendingAmount else { return nil }
        return template.replacingOccurrences(of: "{amount}", with: points(pending))
    }

    /// Home's pending-credit tile and status-card link, in the core's words.
    static var creditPendingWord: String { MonitorWords.table?.creditPending ?? "" }

    static var openTracesWord: String { MonitorWords.table?.openTraces ?? "" }

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
                parts.append(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .memory))
            }
            if let withdrawn = row.revokedAt ?? row.withdrawnAt {
                parts.append("\(MonitorWords.withdrawn) \(day(withdrawn))")
            }
        }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
    }

    /// A History row's tool and day (Ron's `history-row.tsx`, source
    /// first). How it was approved, its size and its withdrawal are the
    /// opened row's (`HistoryDetailInspector`, `meta(_:compact: false)`).
    static func rowMeta(_ row: DaemonData.HistoryRow) -> String {
        var parts: [String] = []
        if let source = row.source { parts.append(InferenceTabView.toolName(source)) }
        if let date = row.submittedAt { parts.append(day(date)) }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
    }

    /// The community's accept rate as a whole percentage, or a dash when
    /// the commons did not say.
    static func rate(_ value: Double?) -> String {
        value.map { $0.formatted(.percent.precision(.fractionLength(0))) } ?? "—"
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

extension HorizontalAlignment {
    private enum HomeStatusText: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[.leading] }
    }

    /// The left edge of the Home status card's words, after its dot.
    static let homeStatusText = HorizontalAlignment(HomeStatusText.self)
}
