import SwiftUI
import TCDesign
import TCShellCore

// The limits card that stood here left Overview (product owner,
// 2026-10-28): each measured login is now a card in "AI usage this week"
// (`InsightsUsageCards`), which reads its login words from here.

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
