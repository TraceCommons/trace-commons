//! Shared words for the local Skills surface and public publisher catalog.

pub const PLACEHOLDERS: &[&str] = &["publisher", "name", "count", "date"];

pub const COPY: &[(&str, &str)] = &[
    ("skills_population_direct", "Direct observations"),
    ("skills_population_delegated", "Delegated observations"),
    ("skills_population_unknown", "Observation origin unknown"),
    ("skills_evidence_links", "Trace evidence"),
    ("skills_evidence_child", "Open child trace {count}"),
    ("skills_evidence_parent", "Open parent"),
    (
        "skills_evidence_accessibility",
        "Open trace {count} for {name}",
    ),
    (
        "skills_evidence_child_accessibility",
        "Open child trace {count} for {name}",
    ),
    (
        "skills_evidence_parent_accessibility",
        "Open parent trace for {name} evidence {count}",
    ),
    ("skills_agent_child_sessions", "Child traces"),
    ("skills_agent_claude_child_sessions", "Claude child traces"),
    ("skills_agent_codex_child_sessions", "Codex child traces"),
    ("skills_coverage_partial", "Partial skill evidence"),
    ("skills_coverage_pending", "Skill evidence pending"),
    ("skills_coverage_unknown", "Skill evidence unknown"),
    (
        "skills_daily_retained",
        "Daily observations · retained history within 90 days, independent of the selected window",
    ),
    (
        "skills_definition_unavailable",
        "Definition unavailable. Refresh to check this installation.",
    ),
    ("skills_evidence_session", "Open trace {count}"),
    ("skills_evidence_title", "Evidence trace"),
    (
        "skills_evidence_unavailable",
        "This trace is no longer available.",
    ),
    ("skills_evidence_turns", "{count} turns"),
    ("skills_evidence_tokens", "{count} tokens"),
    ("skills_close", "Close"),
    (
        "skills_analysis_off",
        "Local trace analysis is off. Your installed skills remain available here.",
    ),
    ("skills_review_settings", "Review settings"),
    ("skills_filter_agent", "Agent"),
    ("skills_all_agents", "All agents"),
    ("skills_filter_availability", "Availability"),
    ("skills_all_availability", "All availability"),
    ("skills_window", "Usage window"),
    ("skills_days", "{count} days"),
    ("skills_sort", "Sort"),
    ("skills_sort_name", "Name"),
    ("skills_sort_sessions", "Traces used"),
    ("skills_sort_recent", "Last used"),
    ("skills_inspect", "Inspect {name}"),
    ("skills_coverage", "Coverage"),
    ("skills_usage_unavailable", "Usage unavailable"),
    (
        "skills_usage_pending",
        "Usage analysis pending or incomplete",
    ),
    (
        "skills_usage_unattributed",
        "Usage could not be attributed to this installation",
    ),
    ("skills_no_usage", "No observed usage"),
    (
        "skills_undated",
        "{count} undated observations excluded from this window",
    ),
    ("skills_earliest", "Earliest retained evidence {date}"),
    (
        "skills_usage_semantics",
        "Traces used counts each trace with an observed invocation or successful definition load once, separately for direct and delegated observations. Parent links provide context and do not add to usage totals. Loads do not prove successful application. Windows include dated evidence only; retained history may be shorter than the selected window.",
    ),
    (
        "skills_definition_status_invalid",
        "Invalid skill definition",
    ),
    (
        "skills_definition_status_unreadable",
        "Could not read skill definition",
    ),
    ("skills_availability_available", "Available"),
    ("skills_availability_disabled", "Disabled"),
    ("skills_availability_cached", "Cached"),
    ("skills_availability_missing", "Missing"),
    ("skills_availability_unknown", "Availability unknown"),
    ("skills_title", "Skills"),
    ("skills_tab_inventory", "Index"),
    ("skills_tab_usage", "Usage"),
    ("skills_tab_catalog", "Catalog"),
    ("skills_tab_following", "Following"),
    ("skills_inventory", "Detected skills"),
    ("skills_usage", "Observed usage"),
    ("skills_catalog", "Catalog"),
    ("skills_search", "Search skills"),
    ("skills_search_empty", "No matching skills."),
    ("skills_publishers", "Publishers"),
    ("skills_partial_catalog", "Showing part of the catalog."),
    ("skills_partial_feed", "Showing part of the publisher feed."),
    ("skills_feed", "Following"),
    ("skills_refresh", "Refresh"),
    (
        "skills_refresh_accessibility",
        "Refresh skills and publisher feeds",
    ),
    ("skills_loading", "Checking skills"),
    (
        "skills_unavailable",
        "Skills could not be read. Try refreshing.",
    ),
    (
        "skills_refresh_failed",
        "Could not refresh. Last-known information is shown.",
    ),
    (
        "skills_follow_failed",
        "Could not save your publisher selection. Try again.",
    ),
    ("skills_disabled", "Skills is off."),
    (
        "skills_inventory_empty",
        "No skills found in the checked locations.",
    ),
    (
        "skills_usage_empty",
        "No invocations observed in the covered traces.",
    ),
    (
        "skills_catalog_empty",
        "No catalog entries are available yet.",
    ),
    (
        "skills_feed_empty",
        "Follow a publisher to see additions and changes observed in its catalog.",
    ),
    (
        "skills_partial_inventory",
        "Some skill locations could not be checked completely.",
    ),
    (
        "skills_partial_usage",
        "Usage coverage is incomplete. Missing observations do not mean a skill was unused.",
    ),
    ("skills_follow", "Follow"),
    ("skills_unfollow", "Unfollow"),
    ("skills_follow_accessibility", "Follow {publisher}"),
    ("skills_unfollow_accessibility", "Unfollow {publisher}"),
    ("skills_instructions", "Read instructions"),
    (
        "skills_instructions_accessibility",
        "Read instructions for {name}",
    ),
    ("skills_invocations", "{count} invocations"),
    ("skills_loads", "{count} loads"),
    ("skills_sessions", "{count} traces used"),
    ("skills_sessions_observed", "{count} traces checked"),
    (
        "skills_sessions_unknown",
        "{count} traces without complete skill evidence",
    ),
    ("skills_last_seen", "Last used {date}"),
    ("skills_checked_at", "Checked {date}"),
    ("skills_status_current", "Up to date"),
    ("skills_status_up_to_date", "Up to date"),
    ("skills_status_update_available", "Update available"),
    ("skills_status_locally_modified", "Locally modified"),
    ("skills_status_unknown_source", "Unknown source"),
    ("skills_status_unknown", "Could not establish freshness"),
    (
        "skills_status_loads_only",
        "Loads observed; invocation coverage unavailable",
    ),
    ("skills_no_invocations", "No invocations observed"),
    ("skills_status_not_checked", "Freshness not established"),
    ("skills_status_stale", "Not checked recently"),
    ("skills_status_failed", "Could not check for updates"),
    ("skills_status_unavailable", "Update checks unavailable"),
    (
        "skills_status_documentation_only",
        "Publisher documentation",
    ),
    ("skills_status_docs_only", "Publisher documentation"),
    ("skills_status_checked", "Checked"),
    ("skills_status_complete", "Checked"),
    ("skills_status_missing", "Location not present"),
    ("skills_status_unreadable", "Could not read this location"),
    ("skills_status_partial", "Partially checked"),
    (
        "skills_status_policy_excluded",
        "Excluded by folder settings",
    ),
    ("skills_status_limited", "Scan limit reached"),
    ("skills_status_truncated", "Scan limit reached"),
    (
        "skills_status_cached_plugin_activation_unknown",
        "Cached plugin; activation unknown",
    ),
    (
        "skills_coverage_explicit_invocations",
        "Explicit skill invocations",
    ),
    (
        "skills_coverage_loads_only",
        "Observed loads; invocation coverage unavailable",
    ),
    (
        "skills_coverage_unsupported",
        "Skill usage coverage unavailable",
    ),
    (
        "skills_coverage_name_only",
        "Skill names observed; installation identity unknown",
    ),
    ("skills_feed_added", "Added to the catalog"),
    ("skills_feed_discovered", "Discovered in the catalog"),
    ("skills_feed_updated", "Upstream change observed"),
    ("skills_agent_codex", "Codex"),
    ("skills_agent_claude_code", "Claude Code"),
    ("skills_agent_opencode", "OpenCode"),
    ("skills_agent_shared", "Shared"),
    ("skills_agent_matching", "Attribution"),
    ("skills_scope_global", "Global"),
    ("skills_scope_project", "Project"),
    (
        "skills_scope_plugin_cache",
        "Cached plugin; activation unknown",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn copy_is_unique_shared_and_has_declared_placeholders() {
        let copy = super::super::service::ui_copy();
        let mut keys = BTreeSet::new();
        for &(key, words) in COPY {
            assert!(keys.insert(key));
            assert_eq!(copy.get(key).map(String::as_str), Some(words));
            let mut rest = words;
            while let Some(open) = rest.find('{') {
                let end = open + rest[open..].find('}').unwrap();
                assert!(PLACEHOLDERS.contains(&&rest[open + 1..end]));
                rest = &rest[end + 1..];
            }
        }
    }
}
