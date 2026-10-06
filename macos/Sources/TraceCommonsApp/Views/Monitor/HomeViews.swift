#if DEBUG
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
struct HomeTabView: View {
    enum Page: String {
        case overview
        case history
        case missions
    }

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String?) -> String?
    @Binding var page: Page
    /// The opened History row's submission id; empty for none. History
    /// draws its details below the list, in this pane.
    @Binding var selection: String
    /// Ron's "Open Traces": switch the window to the Traces tab.
    let openTraces: () -> Void

    var body: some View {
        switch page {
        case .overview:
            HomeOverview(
                store: store, traces: traces, statusLabel: statusLabel,
                openHistory: { page = .history }, openMissions: { page = .missions }, openTraces: openTraces)
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
    let openTraces: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                watching
                // Ron's three stat cards. Pending credit is a figure only
                // beside the commons' statement of what it waits on (D6).
                HStack(spacing: GlassTokens.Space.s3) {
                    // The Traces store's count, the one the status card's
                    // line and the Traces badge read: one waiting number.
                    GlassLegendCell(MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed), status: .ask)
                    GlassLegendCell(MonitorWords.contributed, value: HomeFormat.count(store.rollup?.allTime?.accepted), status: .shared)
                    GlassLegendCell(MonitorWords.pending, value: HomeFormat.pendingFigure(store.rollup?.creditPending, credit: store.credit), status: .ask)
                }
                if let condition = HomeFormat.pendingCondition(store.credit) {
                    Text(condition)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .fixedSize(horizontal: false, vertical: true)
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

    /// Ron's status card: the watching state, what is waiting in the
    /// Traces badge's own words, and the way into Traces.
    private var watching: some View {
        GlassCard {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    watchingLine
                    if let waiting = MonitorWindowView.tracesDescription(
                        traces.decisionsOwed, shield: traces.shield, secondLook: traces.words?.secondLookWaiting) {
                        Text(waiting)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .accessibilityElement(children: .combine)
                Spacer(minLength: 0)
                Button(action: openTraces) {
                    HStack(spacing: GlassTokens.Space.inlineGap) {
                        Text(MonitorWindowView.Tab.traces.title)
                        Image(systemName: "chevron.right")
                            .glassGlyph(10, weight: .semibold)
                    }
                }
                .buttonStyle(GlassButtonStyle(.link))
                .frame(minHeight: 44)
            }
        }
    }

    private var watchingLine: some View {
        let state = state
        return HStack(spacing: GlassTokens.Space.s4) {
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
        }
        .glassType(GlassTokens.TypeScale.bodyStrong)
        .foregroundStyle(GlassColor.textPrimary)
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

/// History, in Ron's #1146 order and all in the left pane: the stat cards,
/// community standing, every contribution from this machine grouped by
/// project with Open and Withdraw on its row, the credit record, what is
/// held for review (apart, never as rejected), and then the opened row's
/// details (`HistoryDetailInspector`: the session detail, the public-run
/// editor and Skills). The inspector beside it keeps the Traces selection.
private struct HistoryPage: View {
    let store: HomeStore
    let statusLabel: (String?) -> String?
    @Binding var selection: String
    let back: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var filter: HistoryList.Filter = .all

    // The app's own records, which each row's Withdraw and the opened
    // row's detail resolve against, are read again on every visit: the
    // daemon's view, not the one it had at launch (the legacy screen's rule).
    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassBreadcrumb(
                [GlassCrumb(MonitorWindowView.Tab.home.title, action: back), GlassCrumb(MonitorWords.history)],
                backLabel: MonitorWindowView.Tab.home.title, onBack: back)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        HistoryStats(store: store)
                        HistoryCommunityCard(store: store)
                        // `accessory:` by name: a first trailing closure
                        // would bind to `action` and make the card a button.
                        GlassEyebrowCard(MonitorWords.history, accessory: { refresh }) {
                            refreshOutcome
                            list
                        }
                        HistoryCreditCard(store: store)
                        held
                        if let opened = openedRow {
                            HistoryDetailInspector(row: opened)
                                .id(HistoryList.detailAnchor)
                        }
                    }
                }
                // Opening a row brings its details, below the list, into view.
                .onChange(of: selection) { _, opened in
                    if !opened.isEmpty { proxy.scrollTo(HistoryList.detailAnchor, anchor: .top) }
                }
            }
            .scrollIndicators(.never)
        }
        .onAppear {
            model.clearHistoryRefresh()
            model.refreshHistory()
            model.refreshAccountSession()
        }
    }

    /// The opened row, while it is still in the list.
    private var openedRow: DaemonData.HistoryRow? {
        guard !selection.isEmpty else { return nil }
        return store.history?.first { $0.submissionId == selection }
    }

    /// The refresh control (Ron's #1146 HistoryRefreshControl): asks the daemon to check the server
    /// sooner (`refresh_history`), then reads History again, the app's
    /// records and this screen's. Only with the core's words.
    @ViewBuilder
    private var refresh: some View {
        if let words = MonitorWords.table?.historyActions {
            Button(model.historyRefresh == .requesting ? words.requesting : words.requestRefresh) {
                Task { if await model.requestHistoryRefresh() { await store.load() } }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .frame(minHeight: 44)
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
        HistoryList.labels(disclosure: HistoryList.disclosure, publicRun: model.publicRunCopy)
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
                filterBar(rows)
                let groups = HistoryFolders.folders(
                    HistoryList.rows(rows, filter: shownFilter),
                    projectID: { $0.projectId ?? "" }, projectLabel: { $0.projectLabel ?? "—" })
                if groups.isEmpty {
                    Text("0")
                        .glassType(GlassTokens.TypeScale.number)
                        .foregroundStyle(GlassColor.textTertiary)
                }
                ForEach(groups) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                            Text(group.label)
                                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                                .foregroundStyle(GlassColor.textPrimary)
                                .lineLimit(1)
                            Spacer(minLength: GlassTokens.Space.s4)
                            Text("\(group.count)")
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                                .monospacedDigit()
                        }
                        .accessibilityElement(children: .combine)
                        ForEach(Array(group.entries.enumerated()), id: \.element.id) { index, row in
                            HistoryListRow(
                                row: row, statusLabel: statusLabel, first: index == 0,
                                selected: selection == row.submissionId,
                                open: { selection = row.submissionId })
                        }
                    }
                }
            }
        } else if store.failures["list_history"] == nil {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity)
        }
    }

    /// Ron's filter: All, then each status, with its count.
    @ViewBuilder
    private func filterBar(_ rows: [DaemonData.HistoryRow]) -> some View {
        if let labels {
            let counts = HistoryList.counts(rows)
            ScrollView(.horizontal) {
                GlassSegmentedTabs(
                    MonitorWords.history, selection: $filter,
                    segments: HistoryList.Filter.allCases.map { item in
                        GlassSegment(labels[item] ?? "", value: item, badge: counts[item] ?? 0)
                    })
            }
            .scrollIndicators(.never)
        }
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
}

/// One contribution in History's list (Ron's `HistoryRow`): what it is, the
/// server's reasons, then Open and Withdraw on the row, the one Withdraw on
/// History's page. Withdraw asks first, with the existing confirmation, and
/// stands on the app's own record of the contribution, so a row that record
/// does not hold yet offers none (fail closed). It decides from the status
/// the opened detail decides from, offers Retry after a failed withdrawal,
/// and gives way to the core's sign-in while no account session is active.
/// Without the core's public-run copy the row offers nothing.
private struct HistoryListRow: View {
    let row: DaemonData.HistoryRow
    let statusLabel: (String?) -> String?
    let first: Bool
    let selected: Bool
    let open: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var confirming = false

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    var body: some View {
        GlassTableRow(first: first) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HistoryRowView(row: row, statusLabel: statusLabel, compact: false)
                explanations
                if let copy = model.publicRunCopy {
                    actions(copy)
                }
            }
        }
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(selected ? GlassColor.ink(0.12) : Color.clear)
        )
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    /// The server's own reasons for this row, without opaque digests; a held
    /// row the server said nothing about reads the held sentence.
    @ViewBuilder
    private var explanations: some View {
        let lines = HeldExplanations.lines(in: [row.explanations ?? []])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            if lines.isEmpty, row.status == "quarantined" {
                Text(HistoryCopy.heldExplanation).fixedSize(horizontal: false, vertical: true)
            }
        }
        .glassType(GlassTokens.TypeScale.caption)
        .foregroundStyle(GlassColor.textSecondary)
    }

    @ViewBuilder
    private func actions(_ copy: PublicRunCopy) -> some View {
        let detail = record.flatMap { model.sessionDetails[$0.submissionID] }
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        let control = HistoryList.rowWithdraw(record: record, detail: detail, result: result, account: model.accountSession)
        HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
            Button(copy.viewSession, action: open)
                .buttonStyle(GlassButtonStyle(.glass, small: true))
                .frame(minHeight: 44)
            if result == nil, !confirming || control != .withdraw {
                withdrawControl(control, copy: copy)
            }
        }
        if let record {
            if let result {
                if HistoryList.showsOutcome(result) {
                    WithdrawalOutcomeView(result: result)
                }
                withdrawControl(control, copy: copy)
            } else if confirming, control == .withdraw {
                WithdrawalConfirmationView(
                    status: SessionDetailView.withdrawalStatus(record, detail: detail),
                    keepLabel: copy.keepContribution,
                    inFlight: model.withdrawing.contains(record.submissionID),
                    onKeep: { confirming = false },
                    onConfirm: { model.withdraw(record) }
                )
            }
        }
    }

    /// What stands in Withdraw's place (`HistoryList.rowWithdraw`). Sign-in
    /// and the account check need the core's words, and draw nothing
    /// without them.
    @ViewBuilder
    private func withdrawControl(_ control: HistoryList.RowWithdraw, copy: PublicRunCopy) -> some View {
        let words = MonitorWords.table?.historyActions
        switch control {
        case .withdraw: Button(copy.withdraw) { confirming = true }
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .frame(minHeight: 44)
            .disabled(record.map { model.withdrawing.contains($0.submissionID) } ?? true)
        // Retry withdraws without asking again, as the legacy
        // `SessionWithdrawalAction` does: the first attempt already asked.
        // (Ron's "Try again" asks again; native's behaviour is kept.)
        case .retry:
            if let record {
                Button(copy.withdraw) { model.withdraw(record) }
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
/// (the daemon opens the browser), say so while it waits, and say why when
/// it did not leave an active session. All in the core's words.
private struct HistorySignInControl: View {
    let words: MonitorHistoryActionsCopy
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Button(model.accountSigningIn ? words.waitingForSignIn : words.signInToWithdraw) { Task { await model.signInToWithdraw() } }
                .buttonStyle(GlassButtonStyle(.glass, small: true))
                .frame(minHeight: 44)
                .disabled(model.accountSigningIn)
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
/// contributed, and held for review.
private struct HistoryStats: View {
    let store: HomeStore

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MonitorWords.pending, value: HomeFormat.pendingFigure(store.rollup?.creditPending, credit: store.credit), status: .ask)
                GlassLegendCell(MonitorWords.contributed, value: HomeFormat.count(store.rollup?.allTime?.accepted), status: .shared)
                GlassLegendCell(MonitorWords.held, value: HomeFormat.count(store.rollup?.quarantined), status: .ask)
            }
            if let condition = HomeFormat.pendingCondition(store.credit) {
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

    /// The core's disclosure bundle, decoded once; nil when it will not.
    static let disclosure = ContributorDisclosureCopy.decode(fromJSON: TCCoreCopy.contributorDisclosureCopyJSON())

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

    /// A filter's label from the core, or nil when the core has not said.
    static func label(_ filter: Filter, disclosure: ContributorDisclosureCopy?, publicRun: PublicRunCopy?) -> String? {
        let label = filter == .all ? publicRun?.allContributions : disclosure?.historyUi.statusLabels[filter.rawValue]
        return label.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Every filter's label, or nil if any is missing.
    static func labels(disclosure: ContributorDisclosureCopy?, publicRun: PublicRunCopy?) -> [Filter: String]? {
        var labels: [Filter: String] = [:]
        for filter in Filter.allCases {
            guard let label = label(filter, disclosure: disclosure, publicRun: publicRun) else { return nil }
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

/// Community standing, when the rollup carries one; nothing otherwise.
struct HistoryCommunityCard: View {
    let store: HomeStore

    var body: some View {
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

/// The credit record: final credit, and pending credit only with the
/// commons' statement of its condition beside it (D6). Without that
/// statement the pending figure is a dash: a bare number would read as owed.
struct HistoryCreditCard: View {
    let store: HomeStore

    var body: some View {
        let condition = HomeFormat.pendingCondition(store.credit)
        GlassEyebrowCard(MonitorWords.credit) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                GlassKeyValueList([
                    .init(MonitorWords.final, HomeFormat.points(store.rollup?.creditFinal)),
                    .init(MonitorWords.pending, HomeFormat.pendingFigure(store.rollup?.creditPending, credit: store.credit)),
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

    /// Pending credit as a figure only when the commons has said what it
    /// waits on (D6); a dash otherwise, never a bare number that reads as
    /// owed.
    static func pendingFigure(_ pending: Double?, credit: DaemonData.CommonsCreditSummary?) -> String {
        pendingCondition(credit) == nil ? "—" : points(pending)
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
