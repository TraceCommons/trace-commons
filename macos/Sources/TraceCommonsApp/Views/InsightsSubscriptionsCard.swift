import SwiftUI
import TCDesign
import TCShellCore

/// Captured provider limits remain independent of the week picker.
struct InsightsSubscriptionsCard: View {
    let model: SubscriptionUsageModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        if model.isVisible, !text("analytics_subscriptions_title").isEmpty {
            VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                Text(text("analytics_subscriptions_title")).insightsTitle()
                Text(text(model.supportedProviders.contains(where: { $0.provider == "claude" })
                          ? "analytics_subscriptions_captured_limits" : "analytics_subscriptions_current")).insightsCaption()
                VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                    if let failure = model.failureKey {
                        Text(text(failure)).insightsError()
                    }
                    if model.shown?.truncated == true {
                        Text(text("analytics_subscriptions_truncated")).insightsCaption()
                    }
                    ForEach(model.supportedProviders) { provider in
                        providerSection(provider)
                    }
                }
                .insightsCardBody()
            }
            .insightsSection()
        }
    }

    private func providerSection(_ provider: DaemonData.SubscriptionUsageProvider) -> some View {
        let name = text("analytics_subscriptions_provider_" + provider.provider)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline) {
                Text(name).insightsHeading()
                if provider.provider == "codex", provider.logins.isEmpty {
                    Spacer(minLength: GlassTokens.Space.s3)
                    refreshButton(provider: provider.provider, login: nil, label: name)
                }
            }
            if provider.provider == "claude" {
                Text(text("analytics_subscriptions_claude_captured"))
                    .insightsCaption().fixedSize(horizontal: false, vertical: true)
                Text(text("analytics_subscriptions_claude_detail"))
                    .insightsCaption().fixedSize(horizontal: false, vertical: true)
            }
            if provider.refreshing, provider.logins.isEmpty {
                Text(text("analytics_subscriptions_reading")).insightsCaption()
            }
            ForEach(provider.logins) { login in
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    loginSection(login, provider: provider.provider, now: context.date)
                }
            }
            if !provider.refreshing, provider.logins.isEmpty {
                Text(InsightsOverviewWords.fill(text("analytics_subscriptions_no_logins"), ["harness": name])).insightsCaption()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func refreshButton(provider: String, login: String?, label: String) -> some View {
        let name = text("analytics_subscriptions_provider_" + provider)
        let accessibility = InsightsOverviewWords.fill(text("analytics_subscriptions_refresh_login_accessibility"),
                                                       ["harness": name, "login": label])
        return GlassRefreshButton(accessibility, small: true,
                                  isRefreshing: model.isRefreshing(provider: provider, login: login)) {
            model.refresh(provider: provider, login: login)
        }
        .disabled(!model.canRefresh(provider: provider, login: login))
    }

    private func loginSection(_ login: DaemonData.SubscriptionUsageLogin, provider: String, now: Date) -> some View {
        let words = InsightsSubscriptionWords.login(login, provider: provider, now: now, copy: copy)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .firstTextBaseline) {
                Text(words.label).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: GlassTokens.Space.s3)
                refreshButton(provider: provider, login: login.id, label: words.label)
            }
            if let freshness = words.freshness {
                Text(freshness).insightsCaption().fixedSize(horizontal: false, vertical: true)
            }
            if let state = words.state { Text(state).insightsCaption().fixedSize(horizontal: false, vertical: true) }
            if login.state == "observed", words.windows.isEmpty {
                Text(text("analytics_subscriptions_not_reported")).insightsCaption()
            }
            ForEach(words.windows) { row in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(row.label).insightsCaption()
                        Spacer(minLength: GlassTokens.Space.s3)
                        Text(row.remaining).glassType(GlassTokens.TypeScale.label.weight(.semibold)).monospacedDigit()
                    }
                    InsightsShareBar(permille: row.remainingPermille)
                    if let reset = row.reset { Text(reset).insightsCaption() }
                }
                .accessibilityElement(children: .combine)
            }
            if provider == "codex" || login.bankedResets != nil {
                HStack(alignment: .firstTextBaseline) {
                    Text(text("analytics_subscriptions_banked")).insightsCaption()
                    Spacer(minLength: GlassTokens.Space.s3)
                    Text(words.banked).glassType(GlassTokens.TypeScale.label.weight(.semibold)).monospacedDigit()
                }
                .accessibilityElement(children: .combine)
            }
            if let checked = words.checked { Text(checked).insightsCaption() }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// No allowance is inferred from absent data or local trace counts. The
/// timestamp check also prevents a held view from outliving a quota window.
enum InsightsSubscriptionWords {
    struct Window: Equatable, Identifiable {
        let id: String
        let label: String
        let remaining: String
        let remainingPermille: UInt64?
        let reset: String?
    }
    struct Login: Equatable {
        let label: String
        let state: String?
        let freshness: String?
        let windows: [Window]
        let banked: String
        let checked: String?
    }

    static func login(_ login: DaemonData.SubscriptionUsageLogin, provider: String,
                      now: Date, copy: [String: String]) -> Login {
        func text(_ key: String) -> String { copy[key] ?? "" }
        func fill(_ key: String, _ values: [String: String]) -> String {
            InsightsOverviewWords.fill(text(key), values)
        }
        let unknown = text("analytics_unavailable")
        let name = text("analytics_subscriptions_provider_" + provider)
        let state: String?
        switch login.reason {
        case "captured_on_refresh" where provider == "claude":
            state = nil
        case "credentials_expired" where provider == "claude", "auth_expired" where provider == "claude":
            state = text("analytics_subscriptions_claude_expired")
        case "auth_missing" where provider == "claude":
            state = text("analytics_subscriptions_claude_auth_missing")
        case "scope_missing":
            state = text("analytics_subscriptions_claude_scope")
        case "keychain_needs_permission" where provider == "claude":
            state = text("analytics_subscriptions_claude_keychain_denied")
        case "credential_read_timeout":
            state = text("analytics_subscriptions_claude_keychain_pending")
        case "auth_changed" where provider == "claude":
            state = text("analytics_subscriptions_claude_auth_changed")
        case "request_timeout", "rate_limited":
            state = text("analytics_subscriptions_" + (login.reason ?? "unavailable"))
        case "credentials_missing", "credentials_expired", "sign_in_required", "unsupported_login", "authentication_failed",
             "auth_missing", "auth_expired", "auth_changed":
            state = fill("analytics_subscriptions_sign_in", ["harness": name])
        case "tools_missing", "native_cli_unavailable":
            state = fill("analytics_subscriptions_native_tool", ["harness": name])
        case "native_tool_running":
            state = text("analytics_subscriptions_native_tool_running")
        case "base_url_refused", "invalid_response", "no_logins", "native_storage_unsupported":
            state = fill("analytics_subscriptions_" + (login.reason ?? "unavailable"), ["harness": name])
        case "not_checked", "not_observed":
            state = text("analytics_subscriptions_not_checked")
        case "keychain_unavailable", "keychain_locked", "keychain_permission_required", "keychain_needs_permission":
            state = fill("analytics_subscriptions_keychain", ["harness": name])
        default:
            state = login.state != "observed" || login.reason != nil ? text("analytics_subscriptions_unavailable") : nil
        }
        // A refused refresh may retain an earlier measurement. Its age and
        // the current failure both matter; neither replaces the other.
        let freshness = login.stale && login.reason != "captured_on_refresh"
            ? text(state == nil ? "analytics_subscriptions_stale" : "analytics_subscriptions_cached") : nil
        let windows = login.windows.enumerated().map { index, window -> Window in
            let label: String
            if window.lane == "model" {
                label = window.label.flatMap { $0.isEmpty ? nil : $0 } ?? text("analytics_subscriptions_model")
            } else if let duration = window.durationMins {
                if duration == 10_080 { label = text("analytics_subscriptions_weekly") }
                else {
                    let unit = duration.isMultiple(of: 1_440) ? "days" : (duration.isMultiple(of: 60) ? "hours" : "minutes")
                    let count = duration / (unit == "days" ? 1_440 : (unit == "hours" ? 60 : 1))
                    let key = "analytics_subscriptions_window_" + (count == 1 ? String(unit.dropLast()) : unit)
                    label = fill(key, ["n": String(count)])
                }
            } else {
                switch window.lane {
                case "session": label = text("analytics_subscriptions_session")
                case "weekly": label = text("analytics_subscriptions_weekly")
                default: label = text("analytics_subscriptions_window_unknown")
                }
            }
            let expired = window.resetsAt.map { $0 <= now } ?? false
            let measured = login.state == "observed" && !expired
            let remaining = measured && window.usedPermille != nil
                ? (window.remainingText.flatMap { $0.isEmpty ? nil : fill("analytics_subscriptions_remaining", ["n": $0]) } ?? unknown)
                : unknown
            let reset = window.resetsAt.flatMap { date in
                date > now ? fill("analytics_subscriptions_resets", ["time": timestamp(date)]) : nil
            }
            // Providers may report several windows in the same lane. Keep
            // each slot, including two windows with an unknown duration.
            let id = "\(index):\(window.lane):\(window.durationMins ?? 0):\(window.label ?? "")"
            let fraction = measured ? window.usedPermille.map { UInt64(1000 - $0) } : nil
            return Window(id: id, label: label, remaining: remaining, remainingPermille: fraction, reset: reset)
        }
        let banked = login.bankedResets.map { fill("analytics_subscriptions_banked_count", ["n": String($0)]) } ?? unknown
        let checked = login.updatedAt.map { fill("analytics_subscriptions_checked", ["time": timestamp($0)]) }
        return Login(label: login.defaultLogin ? text("analytics_subscriptions_default_login") : login.label,
                     state: state, freshness: freshness, windows: windows, banked: banked, checked: checked)
    }

    private static func timestamp(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .shortened)
    }
}
