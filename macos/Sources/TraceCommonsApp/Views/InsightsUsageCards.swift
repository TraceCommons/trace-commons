import Charts
import SwiftUI
import TCDesign
import TCShellCore

/// "AI usage this week" (product owner, 2026-10-28): the high-level
/// takeaway that leads Overview. One card per resource found on this Mac,
/// every card the same component and the same rhythm of rows, side by side
/// and wrapping when the pane is narrow:
///
/// - a Claude or Codex card per login found: session left, weekly left,
///   next reset and pace, each with its Refresh; a login with no measured
///   window keeps its card, its rows the dash, over "Refresh to see
///   limits" (owner, 2026-10-28), so the account never leaves Overview;
/// - a NEAR AI card while NEAR AI calls from this Mac were charged in the
///   last 31 days: this week, today, the last 31 days, and the last 7 days
///   as bars.
///
/// Each card ends with "By project", which opens Details on its By project
/// view. Unknown figures are the core's dash, never a zero.
struct InsightsUsageCards: View {
    let subscriptions: SubscriptionUsageModel?
    let nearAiCredit: DaemonData.InsightsNearAiCredit?
    let copy: [String: String]
    /// "By project": Details, on its By project view.
    var showProjects: () -> Void = {}

    private func text(_ key: String) -> String { copy[key] ?? "" }

    private var nearAiShown: DaemonData.InsightsNearAiCredit? {
        nearAiCredit.flatMap { InsightsUsageWords.showsNearAi($0) ? $0 : nil }
    }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let logins = InsightsLimitsRows.rows(subscriptions?.supportedProviders ?? [], now: context.date, copy: copy)
            if !logins.isEmpty || nearAiShown != nil {
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    Text(text("analytics_usage_title")).insightsHeading()
                    // Only beside a Refresh button, as the limits card had it
                    // (Kristi's #1687 review).
                    if let subscriptions, !logins.isEmpty, !subscriptions.isRefreshing {
                        Text(text("analytics_subscriptions_refresh_hint")).insightsCaption()
                    }
                    if let failure = subscriptions?.failureKey {
                        Text(text(failure)).insightsError()
                    }
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 172), spacing: GlassTokens.Space.cardGap,
                                                 alignment: .top)],
                              alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        ForEach(logins) { row in
                            InsightsUsageCard(title: row.account, copy: copy, showProjects: showProjects, header: {
                                if row.provider == "claude" {
                                    InsightsInfo(row.account, [text("analytics_subscriptions_claude_detail")])
                                }
                                Spacer(minLength: GlassTokens.Space.s2)
                                refreshButton(row)
                            }, rows: {
                                InsightsUsageRow(label: text("analytics_limits_session_left"), value: row.session)
                                InsightsUsageRow(label: text("analytics_limits_weekly_left"), value: row.weekly)
                                InsightsUsageRow(label: text("analytics_limits_next_reset"), value: row.reset)
                                InsightsUsageRow(label: text("analytics_limits_pace"), value: row.pace, warns: row.paceWarns)
                                if !row.measured { Text(text("analytics_limits_refresh_to_see")).insightsCaption() }
                                if let freshness = row.freshness { Text(freshness).insightsCaption() }
                            })
                        }
                        if let credit = nearAiShown { nearAiCard(credit) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
            }
        }
    }

    private func refreshButton(_ row: InsightsLimitsRows.Row) -> some View {
        let name = text("analytics_subscriptions_provider_" + row.provider)
        let accessibility = InsightsOverviewWords.fill(text("analytics_subscriptions_refresh_login_accessibility"),
                                                       ["harness": name, "login": row.loginLabel])
        return GlassRefreshButton(accessibility, small: true,
                                  isRefreshing: subscriptions?.isRefreshing(provider: row.provider, login: row.loginId) ?? false) {
            subscriptions?.refresh(provider: row.provider, login: row.loginId)
        }
        .disabled(!(subscriptions?.canRefresh(provider: row.provider, login: row.loginId) ?? false))
    }

    private func nearAiCard(_ credit: DaemonData.InsightsNearAiCredit) -> some View {
        let name = text("analytics_usage_near_ai")
        let days = InsightsUsageWords.days(credit, copy: copy)
        return InsightsUsageCard(title: name, copy: copy, showProjects: showProjects, header: {
            InsightsInfo(name, [text("analytics_credit_scope_near_ai")])
        }, rows: {
            InsightsUsageRow(label: text("analytics_this_week"),
                             value: InsightsCreditWords.amount(credit.week.usdMicros, copy: copy))
            InsightsUsageRow(label: text("analytics_today"),
                             value: InsightsCreditWords.amount(credit.today.usdMicros, copy: copy))
            InsightsUsageRow(label: text("analytics_usage_last_31_days"),
                             value: InsightsCreditWords.amount(credit.last31Days.usdMicros, copy: copy))
            if !days.isEmpty {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(text("analytics_usage_last_7_days")).insightsCaption()
                    Chart(days) { day in
                        // An unknown day draws no bar; its label says so.
                        BarMark(x: .value(text("analytics_usage_last_7_days"), day.label),
                                y: .value(text("analytics_credit_spent"), day.dollars ?? 0),
                                width: .ratio(0.56))
                        .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                        .accessibilityLabel(day.label)
                        .accessibilityValue(day.amount)
                    }
                    .chartYAxis(.hidden)
                    .frame(height: 56)
                }
            }
        })
    }
}

/// One usage card: its title and header controls, its rows, then "By
/// project". Every card on the row is this component.
struct InsightsUsageCard<Header: View, Rows: View>: View {
    let title: String
    let copy: [String: String]
    let showProjects: () -> Void
    @ViewBuilder var header: () -> Header
    @ViewBuilder var rows: () -> Rows

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                Text(title).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                header()
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) { rows() }
            Spacer(minLength: 0)
            Button(text("analytics_usage_by_project")) { showProjects() }
                .buttonStyle(GlassButtonStyle(.link))
                .accessibilityLabel(InsightsOverviewWords.fill(text("analytics_usage_by_project_a11y"), ["label": title]))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsCard()
    }
}

/// A label and its figure, the rhythm every usage card shares.
struct InsightsUsageRow: View {
    let label: String
    let value: String
    var warns = false

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label).insightsCaption()
            Spacer(minLength: GlassTokens.Space.s3)
            Text(value).glassType(GlassTokens.TypeScale.label.weight(.semibold)).monospacedDigit()
                .foregroundStyle(warns ? GlassStatus.outside.textColor : GlassColor.textPrimary)
        }
        .accessibilityElement(children: .combine)
    }
}

/// The NEAR AI card's figures, kept pure for tests.
enum InsightsUsageWords {
    struct Day: Equatable, Identifiable {
        let id: String
        /// The weekday's short name.
        let label: String
        /// The amount, or the dash when unknown.
        let amount: String
        /// The bar's height; `nil` (no bar) when unknown.
        let dollars: Double?
    }

    /// Only while NEAR AI calls from this Mac were made in the last 31 days.
    static func showsNearAi(_ credit: DaemonData.InsightsNearAiCredit) -> Bool {
        credit.last31Days.calls > 0
    }

    /// The last 7 days, oldest first, from the daemon's `by_day`; empty
    /// from a daemon that predates it.
    static func days(_ credit: DaemonData.InsightsNearAiCredit, copy: [String: String]) -> [Day] {
        let parse = Date.ISO8601FormatStyle().year().month().day().dateSeparator(.dash)
        return (credit.byDay ?? []).enumerated().map { index, window in
            let date = window.date.flatMap { try? parse.parse($0) }
            let label = date.map { $0.formatted(.dateTime.weekday(.narrow)) } ?? String(index + 1)
            return Day(id: window.date ?? String(index), label: label,
                       amount: InsightsCreditWords.amount(window.usdMicros, copy: copy),
                       dollars: window.usdMicros.map { Double($0) / 1_000_000 })
        }
    }
}

/// One card per measured Codex or Claude login, kept pure so the pace and
/// the figures are tested without a view.
enum InsightsLimitsRows {
    struct Row: Equatable, Identifiable {
        let id: String
        /// `codex` or `claude`.
        let provider: String
        let loginId: String
        /// The login as the Refresh button names it.
        let loginLabel: String
        let account: String
        let session: String
        let weekly: String
        let reset: String
        let pace: String
        let paceWarns: Bool
        /// Said when the reading is an earlier one kept after a refusal.
        var freshness: String?
        /// Whether a window was measured; an unmeasured login's figures are
        /// all the dash.
        var measured = true
    }

    enum Pace: Equatable {
        case onPace
        case runsOut(Date)
    }

    /// One per login found. A login with no measured window (not read yet,
    /// signed out, refused) keeps its row with every figure the dash
    /// (owner, 2026-10-28).
    static func rows(_ providers: [DaemonData.SubscriptionUsageProvider], now: Date,
                     copy: [String: String]) -> [Row] {
        func text(_ key: String) -> String { copy[key] ?? "" }
        let unknown = text("analytics_unavailable")
        return providers.flatMap { provider -> [Row] in
            let name = text("analytics_subscriptions_provider_" + provider.provider)
            return provider.logins.map { login in
                let account = provider.logins.count > 1 && !login.defaultLogin ? "\(name) · \(login.label)" : name
                let measured = login.state == "observed" && !login.windows.isEmpty
                let words = InsightsSubscriptionWords.login(login, provider: provider.provider, now: now, copy: copy)
                let live = measured ? login.windows.filter { window in
                    window.usedPermille != nil && (window.resetsAt.map { $0 > now } ?? true)
                } : []
                let session = live.first(where: isSession)
                let weekly = live.first(where: isWeekly)
                let reset = live.compactMap(\.resetsAt).min().map { duration(from: now, to: $0) } ?? unknown
                let paced = (weekly ?? session).flatMap { pace($0, now: now) }
                let paceWords: String
                switch paced {
                case .onPace: paceWords = text("analytics_limits_on_pace")
                case .runsOut(let date):
                    paceWords = InsightsOverviewWords.fill(text("analytics_limits_runs_out"),
                                                           ["time": duration(from: now, to: date)])
                case nil: paceWords = unknown
                }
                let warns: Bool
                if case .runsOut = paced { warns = true } else { warns = false }
                return Row(id: provider.provider + ":" + login.id, provider: provider.provider, loginId: login.id,
                           loginLabel: words.label, account: account,
                           session: session.map(left) ?? unknown, weekly: weekly.map(left) ?? unknown,
                           reset: reset, pace: paceWords, paceWarns: warns, freshness: words.freshness,
                           measured: measured)
            }
        }
    }

    static func isWeekly(_ window: DaemonData.SubscriptionUsageWindow) -> Bool {
        window.lane != "model" && (window.durationMins == 10_080 || (window.durationMins == nil && window.lane == "weekly"))
    }

    static func isSession(_ window: DaemonData.SubscriptionUsageWindow) -> Bool {
        window.lane != "model" && !isWeekly(window)
            && (window.lane == "session" || (window.durationMins.map { $0 < 10_080 } ?? false))
    }

    /// What is left, as a percentage of the window.
    static func left(_ window: DaemonData.SubscriptionUsageWindow) -> String {
        guard let used = window.usedPermille else { return "" }
        let remaining = max(0, min(1_000, 1_000 - used))
        return "\(Int((Double(remaining) / 10).rounded()))%"
    }

    /// At the rate used so far in this window, whether it lasts until it
    /// resets, or when it runs out. `nil` when the window's length or reset
    /// is not reported, or under 5% of it has passed (too early to say).
    static func pace(_ window: DaemonData.SubscriptionUsageWindow, now: Date) -> Pace? {
        guard let used = window.usedPermille, let resets = window.resetsAt,
              let minutes = window.durationMins, minutes > 0, resets > now else { return nil }
        let length = Double(minutes) * 60
        let elapsed = length - resets.timeIntervalSince(now)
        guard elapsed >= length * 0.05 else { return nil }
        let fraction = Double(used) / 1_000
        if fraction >= 1 { return .runsOut(now) }
        guard fraction > 0 else { return .onPace }
        let runsOutIn = (1 - fraction) * elapsed / fraction
        let runsOut = now.addingTimeInterval(runsOutIn)
        return runsOut < resets ? .runsOut(runsOut) : .onPace
    }

    /// "4d 21h", "3h 5m", "12m": the two largest units.
    static func duration(from now: Date, to date: Date) -> String {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 2
        formatter.allowedUnits = [.day, .hour, .minute]
        formatter.zeroFormattingBehavior = .dropAll
        let seconds = max(60, date.timeIntervalSince(now))
        return formatter.string(from: seconds) ?? ""
    }
}
