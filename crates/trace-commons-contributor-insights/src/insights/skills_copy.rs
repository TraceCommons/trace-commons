//! Shared words for the local Skills surface and public publisher catalog.

pub const PLACEHOLDERS: &[&str] = &["publisher", "name", "count", "previous", "date"];

pub const COPY: &[(&str, &str)] = &[
    ("skills_tab_home", "Home"),
    ("skills_home_today", "Today"),
    ("skills_home_this_week", "This week"),
    ("skills_home_earlier", "Earlier"),
    ("skills_home_story_update", "{name} has an update"),
    ("skills_home_story_more", "You used {name} more"),
    ("skills_home_story_less", "You used {name} less"),
    ("skills_home_story_added", "{name} was added"),
    ("skills_home_story_updated", "{name} was updated"),
    ("skills_home_trend_period", "Past 7 days"),
    ("skills_home_empty", "No updates yet."),
    ("skills_home_creators", "Creators"),
    ("skills_home_view", "View"),
    ("skills_home_browse", "Browse"),
    ("skills_home_index", "Installed skills"),
    ("skills_home_index_accessibility", "View installed skills"),
    ("skills_home_browse_accessibility", "Browse skill catalog"),
    ("skills_home_open_skill", "View {name}"),
    ("skills_home_open_usage", "View usage for {name}"),
    ("skills_home_open_creator", "View {name} from {publisher}"),
    ("skills_home_compare_up", "{count} uses, up from {previous}"),
    (
        "skills_home_compare_down",
        "{count} uses, down from {previous}",
    ),
    ("skills_home_week", "Your week"),
    (
        "skills_home_rollup",
        "{count} skills from {publisher} changed",
    ),
    ("skills_most_used", "Most-used skills"),
    ("skills_related_traces", "Related traces"),
    ("skills_removal_candidates", "Removal candidates"),
    ("skills_no_use_period", "No use found · {count} days"),
    ("skills_uses", "Uses"),
    ("skills_usage_period", "Last {count} days"),
    ("skills_removal_empty", "No unused skills found."),
    ("skills_usage_period_label", "Time period"),
    ("skills_select_usage_accessibility", "Show usage for {name}"),
    ("skills_usage_chart_accessibility", "Daily uses of {name}"),
    ("skills_population_direct", "Your traces"),
    ("skills_population_delegated", "Delegated traces"),
    ("skills_population_unknown", "Trace type unknown"),
    ("skills_evidence_links", "Traces"),
    ("skills_evidence_child", "Open trace {count}"),
    ("skills_evidence_parent", "Open parent"),
    (
        "skills_evidence_accessibility",
        "Open trace {count} for {name}",
    ),
    (
        "skills_evidence_child_accessibility",
        "Open delegated trace {count} for {name}",
    ),
    (
        "skills_evidence_parent_accessibility",
        "Open parent trace for {name}, trace {count}",
    ),
    ("skills_agent_child_sessions", "Delegated traces"),
    (
        "skills_agent_claude_child_sessions",
        "Claude delegated traces",
    ),
    (
        "skills_agent_codex_child_sessions",
        "Codex delegated traces",
    ),
    ("skills_coverage_partial", "Some traces couldn’t be checked"),
    ("skills_coverage_pending", "Not checked yet"),
    ("skills_coverage_unknown", "Couldn’t check usage"),
    ("skills_daily_retained", "Last 90 days"),
    (
        "skills_definition_unavailable",
        "Couldn’t read this skill. Try refreshing.",
    ),
    ("skills_evidence_session", "Open trace {count}"),
    ("skills_evidence_title", "Trace"),
    ("skills_evidence_unavailable", "Trace unavailable."),
    ("skills_evidence_turns", "{count} turns"),
    ("skills_evidence_tokens", "{count} tokens"),
    ("skills_close", "Close"),
    ("skills_analysis_off", "Local trace analysis is off."),
    ("skills_review_settings", "Review settings"),
    ("skills_filter_agent", "Agent"),
    ("skills_all_agents", "All agents"),
    ("skills_filter_availability", "Availability"),
    ("skills_all_availability", "All"),
    ("skills_window", "Time period"),
    ("skills_days", "{count} days"),
    ("skills_sort", "Sort"),
    ("skills_sort_name", "Name"),
    ("skills_sort_sessions", "Traces"),
    ("skills_sort_recent", "Last used"),
    ("skills_inspect", "Show {name}"),
    ("skills_coverage", "Coverage"),
    ("skills_usage_unavailable", "Usage unavailable"),
    ("skills_usage_pending", "Usage not ready"),
    (
        "skills_usage_unattributed",
        "Couldn’t match usage to this copy",
    ),
    ("skills_no_usage", "No use found"),
    ("skills_undated", "{count} records have no date"),
    ("skills_earliest", "History starts {date}"),
    (
        "skills_usage_semantics",
        "Traces include uses and loads. A load does not mean a skill was used.",
    ),
    ("skills_definition_status_invalid", "Invalid skill"),
    ("skills_definition_status_unreadable", "Couldn’t read skill"),
    ("skills_availability_available", "Available"),
    ("skills_availability_disabled", "Disabled"),
    ("skills_availability_cached", "Cached"),
    ("skills_availability_missing", "Missing"),
    ("skills_availability_unknown", "Couldn’t check availability"),
    ("skills_title", "Skills"),
    ("skills_tab_inventory", "Index"),
    ("skills_tab_usage", "Usage"),
    ("skills_tab_catalog", "Catalog"),
    ("skills_tab_following", "Following"),
    ("skills_inventory", "Installed skills"),
    ("skills_usage", "Usage"),
    ("skills_catalog", "Catalog"),
    ("skills_search", "Search skills"),
    ("skills_search_empty", "No matching skills."),
    ("skills_publishers", "Publishers"),
    ("skills_partial_catalog", "Showing part of the catalog."),
    ("skills_partial_feed", "Showing part of the feed."),
    ("skills_feed", "Following"),
    ("skills_refresh", "Refresh"),
    (
        "skills_refresh_accessibility",
        "Refresh skills and publisher feeds",
    ),
    ("skills_loading", "Checking skills"),
    (
        "skills_unavailable",
        "Couldn’t read skills. Try refreshing.",
    ),
    (
        "skills_refresh_failed",
        "Couldn’t refresh. Showing saved data.",
    ),
    ("skills_follow_failed", "Couldn’t save. Try again."),
    ("skills_disabled", "Skills is off."),
    ("skills_inventory_empty", "No skills found."),
    ("skills_usage_empty", "No use found."),
    ("skills_catalog_empty", "No skills yet."),
    ("skills_feed_empty", "Follow a publisher to see updates."),
    (
        "skills_partial_inventory",
        "Some locations couldn’t be checked.",
    ),
    ("skills_partial_usage", "Some traces couldn’t be checked."),
    ("skills_follow", "Follow"),
    ("skills_following", "Following"),
    ("skills_following_accessibility", "Following {publisher}"),
    (
        "skills_follow_saving_accessibility",
        "Saving changes for {publisher}",
    ),
    (
        "skills_follow_failed_accessibility",
        "Couldn’t save changes for {publisher}. Try again.",
    ),
    ("skills_unfollow", "Unfollow"),
    ("skills_follow_accessibility", "Follow {publisher}"),
    ("skills_unfollow_accessibility", "Unfollow {publisher}"),
    ("skills_instructions", "Read instructions"),
    (
        "skills_instructions_accessibility",
        "Read instructions for {name}",
    ),
    ("skills_invocations", "{count} uses"),
    ("skills_loads", "Loaded {count} times"),
    ("skills_sessions", "{count} traces"),
    ("skills_sessions_observed", "{count} traces checked"),
    (
        "skills_sessions_unknown",
        "{count} traces couldn’t be checked",
    ),
    ("skills_last_seen", "Last used {date}"),
    ("skills_checked_at", "Checked {date}"),
    ("skills_status_current", "Up to date"),
    ("skills_status_up_to_date", "Up to date"),
    ("skills_status_update_available", "Update available"),
    ("skills_status_locally_modified", "Changed locally"),
    ("skills_status_unknown_source", "Unknown source"),
    ("skills_status_unknown", "Not checked"),
    ("skills_status_loads_only", "Loads only; uses unavailable"),
    ("skills_no_invocations", "No use found"),
    ("skills_status_not_checked", "Not checked"),
    ("skills_status_stale", "Not checked recently"),
    ("skills_status_failed", "Could not check for updates"),
    ("skills_status_unavailable", "Update checks unavailable"),
    ("skills_status_documentation_only", "Instructions"),
    ("skills_status_docs_only", "Instructions"),
    ("skills_status_checked", "Checked"),
    ("skills_status_complete", "Checked"),
    ("skills_status_missing", "Not found"),
    ("skills_status_unreadable", "Couldn’t read location"),
    ("skills_status_partial", "Partly checked"),
    (
        "skills_status_policy_excluded",
        "Excluded in folder settings",
    ),
    ("skills_status_limited", "Scan limit reached"),
    ("skills_status_truncated", "Scan limit reached"),
    (
        "skills_status_cached_plugin_activation_unknown",
        "Cached; may be inactive",
    ),
    ("skills_coverage_explicit_invocations", "Uses"),
    ("skills_coverage_loads_only", "Loads only; uses unavailable"),
    ("skills_coverage_unsupported", "Usage unavailable"),
    ("skills_coverage_name_only", "Matched by name"),
    ("skills_feed_added", "Added"),
    ("skills_feed_discovered", "Added"),
    ("skills_feed_updated", "Updated"),
    ("skills_agent_codex", "Codex"),
    ("skills_agent_claude_code", "Claude Code"),
    ("skills_agent_opencode", "OpenCode"),
    ("skills_agent_shared", "Shared"),
    ("skills_agent_matching", "Name matching"),
    ("skills_scope_global", "All projects"),
    ("skills_scope_project", "Project"),
    ("skills_scope_plugin_cache", "Cached; may be inactive"),
    // The Home card for publishers with recent public activity. The owner
    // accepted these three wordings for now on 2026-11-16. The card is drawn
    // only from a verified server catalog, which no build holds until a
    // catalog key is pinned.
    ("skills_home_active", "Active on GitHub"),
    ("skills_home_stars", "{count} stars"),
    ("skills_home_recent_commits", "Recent commits: {count}"),
    // NEEDS OWNER APPROVAL (2026-11-15, spec skills-catalog-server): the
    // opt-in for reporting public skills the catalog lacks. Not wired to any
    // view; the reporting flag ships off.
    (
        "skills_suggestions_toggle",
        "Suggest public skills for the catalog",
    ),
    (
        "skills_suggestions_detail",
        "Sends the GitHub address of public skills you use that the catalog doesn’t list yet. Never sends private or local skills, their contents, or who you are.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn home_copy_is_short_and_names_its_actions() {
        let copy = super::super::service::ui_copy();
        assert_eq!(copy["skills_tab_home"], "Home");
        assert_eq!(copy["skills_home_view"], "View");
        assert_eq!(copy["skills_home_trend_period"], "Past 7 days");
        for (key, words) in COPY
            .iter()
            .filter(|(key, _)| key.starts_with("skills_home_"))
        {
            assert!(!words.contains("invocation"), "{key}");
            assert!(!words.contains("session"), "{key}");
        }
    }

    #[test]
    fn the_active_publishers_card_names_what_is_measured() {
        let copy = super::super::service::ui_copy();
        // GitHub has its own list called Trending; this is not that list.
        assert!(!copy["skills_home_active"].contains("Trending"));
        // The count is of a week that ended when the server measured it,
        // which may be days ago, and the feed's "Past 7 days" means seven
        // completed local days.
        for key in ["skills_home_active", "skills_home_recent_commits"] {
            assert!(!copy[key].contains("week"), "{key}");
            assert!(!copy[key].contains("7 days"), "{key}");
            assert!(!copy[key].contains("today"), "{key}");
        }
        // A count of one reads correctly.
        assert_eq!(
            copy["skills_home_recent_commits"].replace("{count}", "1"),
            "Recent commits: 1"
        );
        assert_eq!(copy["skills_home_stars"].matches("{count}").count(), 1);
    }

    #[test]
    fn home_usage_lines_give_both_weeks_and_claim_no_first_use() {
        let copy = super::super::service::ui_copy();
        for key in ["skills_home_compare_up", "skills_home_compare_down"] {
            let words = &copy[key];
            assert_eq!(words.matches("{count}").count(), 1, "{key}");
            assert_eq!(words.matches("{previous}").count(), 1, "{key}");
            assert!(!words.contains("first"), "{key}");
            assert!(!words.contains("started"), "{key}");
        }
        assert_eq!(copy["skills_home_week"], "Your week");
        // A roll-up counts catalog changes; it does not say who wrote them.
        assert!(copy["skills_home_rollup"].contains("{publisher}"));
        assert!(!copy["skills_home_rollup"].contains("posted"));
    }

    #[test]
    fn home_stories_name_their_skill_and_claim_no_first_use_or_author() {
        let stories: Vec<_> = COPY
            .iter()
            .filter(|(key, _)| key.starts_with("skills_home_story_"))
            .collect();
        assert_eq!(stories.len(), 5);
        for (key, words) in stories {
            // The headline is split at the name to set it in strong type.
            assert_eq!(words.matches("{name}").count(), 1, "{key}");
            // A catalog change is not a creator's post.
            assert!(!words.contains("{publisher}"), "{key}");
            // Two weeks of history cannot show a first-ever use.
            assert!(!words.contains("started"), "{key}");
            assert!(!words.contains("first"), "{key}");
        }
    }

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
