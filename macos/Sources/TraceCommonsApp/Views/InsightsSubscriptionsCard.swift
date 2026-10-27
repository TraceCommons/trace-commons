import SwiftUI
import TCDesign
import TCShellCore

/// Provider limits are current account facts, independent of the week picker.
struct InsightsSubscriptionsCard: View {
    let model: SubscriptionUsageModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        if model.isVisible, !text("analytics_subscriptions_title").isEmpty {
            VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                HStack(alignment: .firstTextBaseline) {
                    Text(text("analytics_subscriptions_title")).insightsTitle()
                    Spacer(minLength: GlassTokens.Space.s3)
                    GlassRefreshButton(text("analytics_subscriptions_refresh_accessibility"),
                                       isRefreshing: model.isRefreshing) { model.refresh() }
                        .disabled(!model.changingProviders.isEmpty)
                }
                Text(text("analytics_subscriptions_current")).insightsCaption()
                VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                    if let failure = model.failureKey {
                        Text(text(failure)).insightsError()
                    }
                    if model.shown?.truncated == true {
                        Text(text("analytics_subscriptions_truncated")).insightsCaption()
                    }
                    ForEach((model.shown?.providers ?? []).filter { Self.knownProvider($0.provider) }) { provider in
                        providerSection(provider)
                    }
                }
                .insightsCardBody()
            }
            .insightsSection()
        }
    }

    private static func knownProvider(_ provider: String) -> Bool { provider == "codex" || provider == "claude" }

    private func providerSection(_ provider: DaemonData.SubscriptionUsageProvider) -> some View {
        let name = text("analytics_subscriptions_provider_" + provider.provider)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                Text(name).insightsHeading()
                Spacer(minLength: GlassTokens.Space.s2)
                if model.canChange {
                    Button(text(provider.enabled ? "analytics_subscriptions_stop" : "analytics_subscriptions_read")) {
                        Task { await model.setEnabled(provider.provider, on: !provider.enabled) }
                    }
                    .buttonStyle(GlassButtonStyle(.secondary))
                    .disabled(!model.changingProviders.isEmpty)
                    .accessibilityLabel(InsightsOverviewWords.fill(
                        text(provider.enabled ? "analytics_subscriptions_stop_accessibility" : "analytics_subscriptions_read_accessibility"),
                        ["harness": name]))
                }
            }
            if provider.enabled {
                if provider.refreshing, provider.logins.isEmpty {
                    Text(text("analytics_subscriptions_reading")).insightsCaption()
                }
                ForEach(provider.logins) { login in
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        loginSection(login, provider: provider.provider, now: context.date)
                    }
                }
                if !provider.refreshing, provider.logins.isEmpty {
                    Text(text("analytics_subscriptions_unavailable")).insightsCaption()
                }
            } else {
                Text(InsightsOverviewWords.fill(text("analytics_subscriptions_read_detail"), ["harness": name]))
                    .insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func loginSection(_ login: DaemonData.SubscriptionUsageLogin, provider: String, now: Date) -> some View {
        let words = InsightsSubscriptionWords.login(login, provider: provider, now: now, copy: copy)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(words.label).glassType(GlassTokens.TypeScale.label.weight(.semibold))
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
        if login.stale {
            state = text("analytics_subscriptions_stale")
        } else if login.state != "observed" {
            switch login.reason {
            case "credentials_missing", "credentials_expired", "sign_in_required", "unsupported_login", "authentication_failed",
                 "auth_missing", "auth_expired", "auth_changed":
                state = fill("analytics_subscriptions_sign_in", ["harness": name])
            case "tools_missing", "native_cli_unavailable":
                state = fill("analytics_subscriptions_native_tool", ["harness": name])
            case "keychain_read_pending":
                state = text("analytics_subscriptions_keychain_pending")
            case "keychain_unavailable", "keychain_locked", "keychain_permission_required", "keychain_needs_permission":
                state = fill("analytics_subscriptions_keychain", ["harness": name])
            default:
                state = text("analytics_subscriptions_unavailable")
            }
        } else { state = nil }
        var seenWindows = Set<String>()
        let windows = login.windows.compactMap { window -> Window? in
            let label: String
            switch window.lane {
            case "session": label = text("analytics_subscriptions_session")
            case "weekly": label = text("analytics_subscriptions_weekly")
            case "model": label = window.label.flatMap { $0.isEmpty ? nil : $0 } ?? text("analytics_subscriptions_model")
            default: return nil
            }
            let expired = window.resetsAt.map { $0 <= now } ?? false
            let measured = login.state == "observed" && !expired
            let remaining = measured && window.usedPermille != nil
                ? (window.remainingText.flatMap { $0.isEmpty ? nil : fill("analytics_subscriptions_remaining", ["n": $0]) } ?? unknown)
                : unknown
            let reset = window.resetsAt.flatMap { date in
                date > now ? fill("analytics_subscriptions_resets", ["time": timestamp(date)]) : nil
            }
            let id = window.lane + ":" + (window.label ?? "")
            guard seenWindows.insert(id).inserted else { return nil }
            let fraction = measured ? window.usedPermille.map { UInt64(1000 - $0) } : nil
            return Window(id: id, label: label, remaining: remaining, remainingPermille: fraction, reset: reset)
        }
        let banked = login.bankedResets.map { fill("analytics_subscriptions_banked_count", ["n": String($0)]) } ?? unknown
        let checked = login.updatedAt.map { fill("analytics_subscriptions_checked", ["time": timestamp($0)]) }
        return Login(label: login.defaultLogin ? text("analytics_subscriptions_default_login") : login.label,
                     state: state, windows: windows, banked: banked, checked: checked)
    }

    private static func timestamp(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .shortened)
    }
}
