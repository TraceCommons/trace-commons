//! The subscription usage words (PR #1608 design, section 5.3, first
//! slice): the Insights "Subscriptions" card and its Settings switch, for
//! Codex. No shell authors any of these sentences; each reads this table
//! through `ui_copy()` (`tc_insights_copy_json`), like every other Insights
//! word. DRAFT, NEEDS APPROVAL.
//!
//! The daemon's `subscription_usage` answer carries labels, never words.
//! A shell turns them into words with this table:
//!
//! - `plan` -> `subscription_usage_plan_{plan}`; `unknown` titles the card
//!   `subscription_usage_provider_codex` with `subscription_usage_plan_unknown`.
//! - a `session` or `weekly` window -> by `duration_mins`: 300 is
//!   `subscription_usage_window_5h`, 10080 `subscription_usage_window_weekly`,
//!   another whole number of days `subscription_usage_window_days`, of hours
//!   `subscription_usage_window_hours`, none `subscription_usage_window_unnamed`.
//!   A `model` window is named by its `label`, or `..._window_unnamed`.
//!   A window whose `resets_at` has passed is `subscription_usage_unknown`,
//!   never its figure.
//! - the card's title is the plan's name, or `subscription_usage_provider_codex`
//!   when `plan` is `null` or `unknown`.
//! - `reset_credits` of 1 is `subscription_usage_reset_credit_one`, any
//!   other count above 0 `subscription_usage_reset_credits`.
//! - `reason` -> `codex_not_found` `subscription_usage_codex_not_found`,
//!   `codex_running` `subscription_usage_codex_running`, `signed_out`
//!   `subscription_usage_signed_out_codex`, `not_subscription`
//!   `subscription_usage_not_subscription`, `blocked`
//!   `subscription_usage_needs_attention`; `never_refreshed` shows no line
//!   beyond the Refresh button; `timeout`, `unreadable` and `failed` are
//!   `subscription_usage_unknown`.
//!
//! Every number or name a sentence carries is a `{name}` hole from
//! [`SUBSCRIPTION_USAGE_PLACEHOLDERS`]; the shell fills it and adds nothing
//! else.

/// Every `{name}` hole the table uses.
pub const SUBSCRIPTION_USAGE_PLACEHOLDERS: &[&str] = &["n", "relative", "age", "balance"];

/// Key, words.
pub const SUBSCRIPTION_USAGE_COPY: &[(&str, &str)] = &[
    // Settings.
    ("subscription_usage_section_title", "Subscription usage"),
    ("subscription_usage_codex_switch", "Codex usage"),
    (
        "subscription_usage_codex_detail",
        "Shows your ChatGPT plan and how much of each Codex limit you have used. When you choose Refresh, Trace Commons asks the Codex app on this Mac. Codex may renew its own sign-in while it answers. Nothing else leaves this Mac.",
    ),
    // The Insights card.
    ("subscription_usage_card_title", "Subscriptions"),
    ("subscription_usage_provider_codex", "Codex"),
    ("subscription_usage_plan_free", "ChatGPT Free"),
    ("subscription_usage_plan_plus", "ChatGPT Plus"),
    ("subscription_usage_plan_pro", "ChatGPT Pro"),
    ("subscription_usage_plan_team", "ChatGPT Team"),
    ("subscription_usage_plan_business", "ChatGPT Business"),
    ("subscription_usage_plan_enterprise", "ChatGPT Enterprise"),
    ("subscription_usage_plan_edu", "ChatGPT Edu"),
    ("subscription_usage_plan_unknown", "Plan unknown"),
    ("subscription_usage_window_5h", "5-hour"),
    ("subscription_usage_window_weekly", "Weekly"),
    ("subscription_usage_window_hours", "{n}-hour"),
    ("subscription_usage_window_days", "{n}-day"),
    ("subscription_usage_window_unnamed", "Usage window"),
    ("subscription_usage_used", "{n}% used"),
    ("subscription_usage_resets", "Resets {relative}"),
    ("subscription_usage_updated", "Updated {age} ago"),
    ("subscription_usage_last_updated", "Last updated {age} ago"),
    ("subscription_usage_unknown", "Usage unknown"),
    ("subscription_usage_limit_reached", "Limit reached"),
    ("subscription_usage_credits_available", "Credits available"),
    ("subscription_usage_credits_unlimited", "Unlimited credits"),
    ("subscription_usage_credits_balance", "{balance} credits"),
    ("subscription_usage_reset_credit_one", "1 reset credit"),
    ("subscription_usage_reset_credits", "{n} reset credits"),
    // Why there is no reading.
    (
        "subscription_usage_codex_not_found",
        "Codex not found on this Mac",
    ),
    (
        "subscription_usage_codex_running",
        "Codex is open. Close it, then choose Refresh.",
    ),
    ("subscription_usage_signed_out_codex", "Signed out of Codex"),
    (
        "subscription_usage_not_subscription",
        "Not a ChatGPT sign-in",
    ),
    (
        "subscription_usage_needs_attention",
        "Codex needs attention. Open Codex once.",
    ),
    // The card's one control.
    ("subscription_usage_refresh", "Refresh"),
    (
        "subscription_usage_refresh_a11y",
        "Refresh subscription usage",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_unique_namespaced_and_every_hole_declared() {
        let mut keys = std::collections::BTreeSet::new();
        for (key, words) in SUBSCRIPTION_USAGE_COPY {
            assert!(keys.insert(*key), "duplicate {key}");
            assert!(key.starts_with("subscription_usage_"), "{key}");
            assert!(!words.is_empty(), "{key}");
            let mut rest = *words;
            while let Some(open) = rest.find('{') {
                let close = rest[open..].find('}').expect("closed hole") + open;
                let name = &rest[open + 1..close];
                assert!(
                    SUBSCRIPTION_USAGE_PLACEHOLDERS.contains(&name),
                    "{key}: {{{name}}}"
                );
                rest = &rest[close + 1..];
            }
        }
    }

    /// The shells read these through `ui_copy()`, word for word.
    #[test]
    fn ui_copy_carries_every_entry() {
        let copy = super::super::service::ui_copy();
        for (key, words) in SUBSCRIPTION_USAGE_COPY {
            assert_eq!(copy.get(*key).map(String::as_str), Some(*words), "{key}");
        }
    }

    /// Every plan label the daemon can send has a name, so a shell never
    /// shows a wire label.
    #[test]
    fn every_plan_label_has_words() {
        for plan in [
            "free",
            "plus",
            "pro",
            "team",
            "business",
            "enterprise",
            "edu",
            "unknown",
        ] {
            let key = format!("subscription_usage_plan_{plan}");
            assert!(
                SUBSCRIPTION_USAGE_COPY.iter().any(|(k, _)| *k == key),
                "{key}"
            );
        }
    }

    #[test]
    fn the_switch_detail_says_what_leaves_the_mac() {
        let detail = SUBSCRIPTION_USAGE_COPY
            .iter()
            .find(|(key, _)| *key == "subscription_usage_codex_detail")
            .unwrap()
            .1;
        assert!(detail.contains("When you choose Refresh"));
        assert!(detail.ends_with("Nothing else leaves this Mac."));
    }
}
