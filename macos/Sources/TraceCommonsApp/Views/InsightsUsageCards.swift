import Charts
import SwiftUI
import TCDesign
import TCShellCore

/// "AI usage this week" (product owner, 2026-10-28): the high-level
/// takeaway that leads Overview. One card per resource found on this Mac,
/// every card the same component and the same rhythm of rows, side by side
/// and wrapping when the pane is narrow, each ending with the same 7-day
/// bars and "By project", which opens Details on its By project view:
///
/// - a Claude or Codex card per login found: what is left of each window
///   the provider reports and when it resets (a window it does not report
///   has no rows: Codex reports no session window today), pace, and banked
///   resets when known, with its Refresh; bars of the tool's tokens per
///   day. A login with no measured window keeps its card, its rows the
///   dash, over "Refresh to see limits", so the account never leaves
///   Overview;
/// - a NEAR AI card while NEAR AI calls from this Mac were made in the
///   last 31 days: spend and tokens this week, today and in the last 31
///   days; bars of spend per day, since spend is what the card is about.
///
/// Unknown figures are the core's dash, never a zero.
struct InsightsUsageCards: View {
    let subscriptions: SubscriptionUsageModel?
    let nearAiCredit: DaemonData.InsightsNearAiCredit?
    /// Claude and Codex tokens on each of the last 7 days.
    var usageByDay: [DaemonData.InsightsUsageDay]?
    let copy: [String: String]
    /// "By project": Details, on its By project view.
    var showProjects: () -> Void = {}
    /// The NEAR AI payout split (referral spec section 6): credit from own
    /// traces and from referrals, from the payout rail's streams
    /// (`NearAiEarningsSplit`). Empty draws nothing, as it is for an
    /// account that never referred anyone.
    var earnings: [NearAiEarningsSplit.Row] = []

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
                        ForEach(logins) { row in loginCard(row) }
                        if let credit = nearAiShown { nearAiCard(credit) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
            }
        }
    }

    private func loginCard(_ row: InsightsLimitsRows.Row) -> some View {
        let tool = row.provider == "claude" ? "claude_code" : row.provider
        return InsightsUsageCard(title: row.account, copy: copy,
                                 bars: InsightsUsageWords.tokenBars(usageByDay, tool: tool, copy: copy),
                                 showProjects: showProjects, header: {
            if row.provider == "claude" {
                InsightsInfo(row.account, [text("analytics_subscriptions_claude_detail")])
            }
            Spacer(minLength: GlassTokens.Space.s2)
            refreshButton(row)
        }, rows: {
            // One headline leads the card (product owner, 2026-10-29):
            // whether the limit lasts until it resets.
            if let headline = row.headline {
                Text(headline).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(row.paceWarns ? GlassStatus.outside.textColor : GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let session = row.session {
                InsightsUsageRow(label: text("analytics_limits_session_left"), value: session)
            }
            if let reset = row.sessionReset {
                InsightsUsageRow(label: text("analytics_usage_session_resets"), value: reset)
            }
            if let weekly = row.weekly {
                InsightsUsageRow(label: text("analytics_limits_weekly_left"), value: weekly)
            }
            if let reset = row.weeklyReset {
                InsightsUsageRow(label: text("analytics_usage_weekly_resets"), value: reset)
            }
            if row.headline == nil {
                InsightsUsageRow(label: text("analytics_limits_pace"), value: row.pace, warns: row.paceWarns)
            }
            if let banked = row.banked {
                InsightsUsageRow(label: text("analytics_subscriptions_banked"), value: banked)
            }
            if !row.measured { Text(text("analytics_limits_refresh_to_see")).insightsCaption() }
            if let freshness = row.freshness { Text(freshness).insightsCaption() }
        })
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
        return InsightsUsageCard(title: name, copy: copy, bars: InsightsUsageWords.days(credit, copy: copy),
                                 showProjects: showProjects, header: {
            InsightsInfo(name, [text("analytics_credit_scope_near_ai")])
        }, rows: {
            InsightsUsageRow(label: text("analytics_this_week"),
                             value: InsightsCreditWords.spendAndTokens(credit.week, copy: copy))
            InsightsUsageRow(label: text("analytics_today"),
                             value: InsightsCreditWords.spendAndTokens(credit.today, copy: copy))
            InsightsUsageRow(label: text("analytics_usage_last_31_days"),
                             value: InsightsCreditWords.spendAndTokens(credit.last31Days, copy: copy))
            ForEach(earnings) { row in
                InsightsUsageRow(label: row.label, value: row.value)
            }
        })
    }
}

/// One usage card: its title and header controls, its rows, the 7-day
/// bars, then "By project". Every card on the row is this component.
struct InsightsUsageCard<Header: View, Rows: View>: View {
    let title: String
    let copy: [String: String]
    let bars: [InsightsUsageWords.Day]
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
            InsightsUsageBars(days: bars, caption: text("analytics_usage_last_7_days"))
            Button(text("analytics_usage_by_project")) { showProjects() }
                .buttonStyle(GlassButtonStyle(.link))
                .accessibilityLabel(InsightsOverviewWords.fill(text("analytics_usage_by_project_a11y"), ["label": title]))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsCard()
    }
}

/// The same small bars on every usage card: the last 7 days, oldest first.
/// An unknown day draws no bar and reads as the dash.
struct InsightsUsageBars: View {
    let days: [InsightsUsageWords.Day]
    let caption: String

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(caption).insightsCaption()
            Chart(days) { day in
                BarMark(x: .value(caption, day.id), y: .value(caption, day.value ?? 0), width: .ratio(0.56))
                    .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                    .accessibilityLabel(day.label)
                    .accessibilityValue(day.amount)
            }
            .chartXScale(domain: days.map(\.id))
            .chartXAxis {
                AxisMarks { value in
                    AxisValueLabel {
                        if let id = value.as(String.self), let day = days.first(where: { $0.id == id }) {
                            Text(day.label).insightsTick()
                        }
                    }
                }
            }
            .chartYAxis(.hidden)
            .frame(height: 56)
            .insightsPlot()
        }
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

/// The usage cards' figures, kept pure for tests.
enum InsightsUsageWords {
    struct Day: Equatable, Identifiable {
        /// The date, `YYYY-MM-DD`.
        let id: String
        /// The weekday's short name.
        let label: String
        /// The figure read out, or the dash when unknown.
        let amount: String
        /// The bar's height; `nil` (no bar) when unknown.
        let value: Double?
    }

    /// Only while NEAR AI calls from this Mac were made in the last 31 days.
    static func showsNearAi(_ credit: DaemonData.InsightsNearAiCredit) -> Bool {
        credit.last31Days.calls > 0
    }

    private static func weekday(_ date: String?, fallback: Int) -> String {
        let parse = Date.ISO8601FormatStyle().year().month().day().dateSeparator(.dash)
        let utc = TimeZone(secondsFromGMT: 0) ?? .current
        return date.flatMap { try? parse.parse($0) }
            .map { $0.formatted(Date.FormatStyle(timeZone: utc).weekday(.narrow)) } ?? String(fallback + 1)
    }

    /// NEAR AI spend on each of the last 7 days, from `by_day`; empty from a
    /// daemon that predates it.
    static func days(_ credit: DaemonData.InsightsNearAiCredit, copy: [String: String]) -> [Day] {
        (credit.byDay ?? []).enumerated().map { index, window in
            // A partly known day draws its known part, read out as "at
            // least" (owner, 2026-10-28).
            Day(id: window.date ?? String(index), label: weekday(window.date, fallback: index),
                amount: InsightsCreditWords.spend(window.usdMicros, known: window.usdMicrosKnown,
                                                  partial: window.usdPartial, copy: copy),
                value: (window.usdMicros ?? (window.usdPartial == true ? window.usdMicrosKnown : nil))
                    .map { Double($0) / 1_000_000 })
        }
    }

    /// A tool's tokens on each of the last 7 days (`claude_code` or
    /// `codex`), from `usage_by_day`; empty from a daemon that predates it.
    static func tokenBars(_ days: [DaemonData.InsightsUsageDay]?, tool: String, copy: [String: String]) -> [Day] {
        (days ?? []).enumerated().map { index, day in
            let tokens = tool == "codex" ? day.codex : day.claudeCode
            return Day(id: day.date, label: weekday(day.date, fallback: index),
                       amount: InsightsOverviewWords.figure(tokens.map { UInt64(max(0, $0)) }, copy: copy),
                       value: tokens.map(Double.init))
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
        /// What is left of the session window; `nil` (no row) when the
        /// provider reports no session window.
        let session: String?
        /// When the session window resets; `nil` with no session row, and
        /// for an unmeasured login.
        let sessionReset: String?
        /// What is left of the weekly window; `nil` (no row) when not
        /// reported.
        let weekly: String?
        let weeklyReset: String?
        let pace: String
        let paceWarns: Bool
        /// Banked resets (Codex's rate-limit reset credits), only when known.
        var banked: String?
        /// Said when the reading is an earlier one kept after a refusal.
        var freshness: String?
        /// Whether a window was measured; an unmeasured login's figures are
        /// all the dash.
        var measured = true
        /// "Weekly limit runs out in 4d 21h, before it resets" or "On
        /// pace: lasts until reset"; `nil` when the pace is unknown.
        var headline: String? = nil
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
                // A window the provider reports, measured or not, has its
                // rows; one it does not report has none. An unmeasured
                // login shows both, as the dash.
                let reports = { (lane: (DaemonData.SubscriptionUsageWindow) -> Bool) in
                    !measured || login.windows.contains(where: lane)
                }
                let resets = { (window: DaemonData.SubscriptionUsageWindow?) -> String? in
                    guard measured else { return nil }
                    return window?.resetsAt.map { duration(from: now, to: $0) } ?? unknown
                }
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
                let paceWindow = text(weekly != nil ? "analytics_subscriptions_weekly" : "analytics_subscriptions_session")
                let headline: String?
                switch paced {
                case .onPace: headline = text("analytics_limits_headline_on_pace")
                case .runsOut(let date):
                    headline = InsightsOverviewWords.fill(text("analytics_limits_headline_runs_out"),
                                                          ["window": paceWindow, "time": duration(from: now, to: date)])
                case nil: headline = nil
                }
                let hasSession = reports(isSession), hasWeekly = reports(isWeekly)
                return Row(id: provider.provider + ":" + login.id, provider: provider.provider, loginId: login.id,
                           loginLabel: words.label, account: account,
                           session: hasSession ? (session.map(left) ?? unknown) : nil,
                           sessionReset: hasSession ? resets(session) : nil,
                           weekly: hasWeekly ? (weekly.map(left) ?? unknown) : nil,
                           weeklyReset: hasWeekly ? resets(weekly) : nil,
                           pace: paceWords, paceWarns: warns,
                           banked: login.bankedResets.map(String.init),
                           freshness: words.freshness, measured: measured, headline: headline)
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
