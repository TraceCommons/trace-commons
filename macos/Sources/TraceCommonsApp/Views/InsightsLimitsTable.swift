import SwiftUI
import TCDesign
import TCShellCore

/// Limits at a glance (product owner, 2026-10-28, CodexBar-style): one row
/// per measured login, with what is left of the session and weekly
/// windows, the next reset and the pace, so accounts compare in one look.
/// The subscription card under it keeps the detail. Only measured windows
/// fill a cell; anything else is the core's dash.
struct InsightsLimitsTable: View {
    let model: SubscriptionUsageModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let rows = InsightsLimitsRows.rows(model.supportedProviders, now: context.date, copy: copy)
            if !rows.isEmpty {
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    Text(text("analytics_limits_title")).insightsHeading()
                    Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s5,
                         verticalSpacing: GlassTokens.Space.s3) {
                        GridRow {
                            ForEach(InsightsLimitsRows.headerKeys, id: \.self) { key in
                                Text(text(key)).insightsCaption()
                            }
                        }
                        Divider()
                        ForEach(rows) { row in
                            GridRow {
                                Text(row.account).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                                Text(row.session).monospacedDigit()
                                Text(row.weekly).monospacedDigit()
                                Text(row.reset).monospacedDigit()
                                Text(row.pace).foregroundStyle(row.paceWarns ? GlassStatus.outside.textColor : GlassColor.textPrimary)
                            }
                            .glassType(GlassTokens.TypeScale.label)
                            .accessibilityElement(children: .combine)
                        }
                    }
                    .insightsCardBody()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
            }
        }
    }
}

/// The table's rows, kept pure so the pace and the cells are tested
/// without a view.
enum InsightsLimitsRows {
    static let headerKeys = [
        "analytics_limits_account", "analytics_limits_session_left", "analytics_limits_weekly_left",
        "analytics_limits_next_reset", "analytics_limits_pace",
    ]

    struct Row: Equatable, Identifiable {
        let id: String
        let account: String
        let session: String
        let weekly: String
        let reset: String
        let pace: String
        let paceWarns: Bool
    }

    enum Pace: Equatable {
        case onPace
        case runsOut(Date)
    }

    static func rows(_ providers: [DaemonData.SubscriptionUsageProvider], now: Date,
                     copy: [String: String]) -> [Row] {
        func text(_ key: String) -> String { copy[key] ?? "" }
        let unknown = text("analytics_unavailable")
        return providers.flatMap { provider -> [Row] in
            let name = text("analytics_subscriptions_provider_" + provider.provider)
            let measured = provider.logins.filter { $0.state == "observed" && !$0.windows.isEmpty }
            return measured.map { login in
                let account = measured.count > 1 && !login.defaultLogin ? "\(name) · \(login.label)" : name
                let live = login.windows.filter { window in
                    window.usedPermille != nil && (window.resetsAt.map { $0 > now } ?? true)
                }
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
                return Row(id: provider.provider + ":" + login.id, account: account,
                           session: session.map(left) ?? unknown, weekly: weekly.map(left) ?? unknown,
                           reset: reset, pace: paceWords, paceWarns: warns)
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
