//! The token analytics words: tabs, the Overview, Patterns and Sessions
//! cards, the glance card and tip, the weekly summary card and Settings.
//!
//! Every entry is DRAFT, NEEDS APPROVAL (owner decision D17, open). No shell
//! authors any of these sentences; each reads this table through `ui_copy()`
//! (`tc_insights_copy_json`, or the `copy` operation of `tc_insights_call`).
//!
//! Every number or name a sentence carries is a `{name}` hole from
//! [`ANALYTICS_PLACEHOLDERS`]; the shell fills it and adds nothing else.
//!
//! Held, and so absent, until owner decision D2: every advice sentence, the
//! what-if sentences and anything that recommends an action. No run of weeks
//! is counted anywhere (owner decision D1, open). No money appears here
//! (owner decision D5, open).
//!
//! The glance, tip and news-mark words (`analytics_glance_*`,
//! `analytics_tip_*`, `analytics_mark_a11y_tip`) are listed here so they are
//! enumerated with the rest; the menu-bar slice moves them into the table the
//! menu-bar panel reads.

/// Every `{name}` hole the table uses.
pub const ANALYTICS_PLACEHOLDERS: &[&str] = &[
    "range",
    "k",
    "n",
    "p",
    "u",
    "d",
    "r",
    "q",
    "t",
    "f",
    "c",
    "l",
    "x",
    "m",
    "threshold",
    "letter",
    "ext",
    "goal",
    "pattern",
    "ctx",
    "source",
    "date",
    "harness",
    "span",
    "label",
    "tokens",
];

/// Key, words. DRAFT, NEEDS APPROVAL (owner decision D17, open).
pub const ANALYTICS_COPY: &[(&str, &str)] = &[
    // The figure that is not shown: unknown, or not computed under this feed.
    ("analytics_unavailable", "—"),
    // 1. Tabs and headings.
    ("analytics_tab_overview", "Overview"),
    ("analytics_tab_patterns", "Patterns"),
    ("analytics_tab_sessions", "Sessions"),
    ("analytics_tab_spend", "Spend"),
    ("analytics_later", "Later"),
    ("analytics_tab_analyze", "Analyze"),
    ("analytics_this_week", "This week"),
    ("analytics_today", "Today"),
    ("analytics_where_tokens_went", "Where tokens went"),
    ("analytics_lever_title", "Lever of the week"),
    ("analytics_goals_title", "Your goals"),
    ("analytics_reread_title", "Most re-read files"),
    ("analytics_input_each_turn", "Input sent each turn"),
    ("analytics_your_week", "Your week · {range}"),
    ("analytics_sample_data", "Sample data"),
    // 2. Coverage.
    (
        "analytics_coverage_line",
        "Usage known for {k} of {n} sessions · {p} partial · {u} unknown, not counted as zero",
    ),
    (
        "analytics_coverage_undated",
        "{d} sessions have no recorded dates and are not in any week.",
    ),
    (
        "analytics_coverage_overlap",
        "{r} sessions overlap an earlier import and are counted once.",
    ),
    (
        "analytics_coverage_too_few",
        "Not enough sessions this week to compare.",
    ),
    // Coverage states and reasons, for the drill-down's Coverage and Reason
    // columns. Not enumerated in the spec's copy list; added so no shell
    // shows a wire label.
    ("analytics_state_known", "Known"),
    ("analytics_state_partial", "Partial"),
    ("analytics_state_unknown", "Unknown"),
    (
        "analytics_reason_codex_baseline_excluded",
        "Counters from before the first reading are left out",
    ),
    (
        "analytics_reason_truncated",
        "Too many turns to keep them all",
    ),
    (
        "analytics_reason_some_turns_unknown",
        "Some turns have no usage counters",
    ),
    (
        "analytics_reason_spans_weeks",
        "Runs across more than one week",
    ),
    ("analytics_reason_no_usage_counters", "No usage counters"),
    (
        "analytics_reason_source_unsupported",
        "This file format has no usage counters",
    ),
    ("analytics_reason_undated", "No recorded dates"),
    (
        "analytics_reason_reimport_overlap",
        "Overlaps an earlier import",
    ),
    (
        "analytics_reason_not_routed",
        "Not routed through Private AI",
    ),
    ("analytics_reason_stale", "Not read recently"),
    // 3. Feed lines.
    (
        "analytics_feed_saved",
        "Only sessions you analyzed are counted.",
    ),
    (
        "analytics_feed_ledger",
        "Routed calls only. Sessions outside Private AI are not counted.",
    ),
    (
        "analytics_feed_counter_pass",
        "Sessions in your watched folders.",
    ),
    (
        "analytics_feed_counter_pass_unavailable",
        "Watched-folder counting is unavailable right now.",
    ),
    (
        "analytics_feed_comparisons_need_counter_pass",
        "Week-to-week comparisons need watched-folder counting.",
    ),
    // 4. Card names.
    ("analytics_card_tokens", "Tokens"),
    ("analytics_card_cache_share", "Cache share"),
    ("analytics_card_sessions", "Sessions"),
    ("analytics_card_tokens_by_day", "Tokens by day"),
    ("analytics_card_by_model", "By model"),
    ("analytics_card_by_project", "By project"),
    ("analytics_card_by_tool", "By tool"),
    ("analytics_unknown_label", "Unknown label"),
    (
        "analytics_by_project_unavailable",
        "Not available for analyzed files",
    ),
    // 5. Series and source lines.
    ("analytics_series_uncached", "Uncached"),
    ("analytics_series_cache_read", "Cache read"),
    ("analytics_series_cache_write", "Cache write"),
    ("analytics_series_output", "Output"),
    (
        "analytics_source_claude_code",
        "Claude Code · counted per turn",
    ),
    (
        "analytics_source_codex",
        "Codex · change between first and last counter",
    ),
    (
        "analytics_codex_interval",
        "Codex · interval totals, not by day",
    ),
    // 6. Change and best.
    ("analytics_change_down", "▼ {p}% vs last week"),
    ("analytics_change_up", "▲ {p}% vs last week"),
    (
        "analytics_cache_share_line",
        "{source} · {p}% of input read from cache",
    ),
    ("analytics_best_week", "Your best week · previous best {q}%"),
    ("analytics_largest", "Largest: {t} tokens"),
    // 7. Drill-down.
    ("analytics_drill_title", "What makes up this number"),
    ("analytics_drill_session", "Session"),
    ("analytics_drill_tokens", "Tokens"),
    ("analytics_drill_coverage", "Coverage"),
    ("analytics_drill_reason", "Reason"),
    // 8. By model.
    (
        "analytics_model_labels_note",
        "Declared model labels. They show what the agent reported, not which model served the request.",
    ),
    // 9. Patterns intro.
    (
        "analytics_patterns_intro",
        "Patterns found in tool calls and usage counters. No session text is stored.",
    ),
    (
        "analytics_patterns_overlap",
        "These patterns overlap, so they do not add up to your total.",
    ),
    // 10. Repeated reads.
    ("analytics_pattern_repeated_reads", "Repeated reads"),
    (
        "analytics_pattern_repeated_reads_count",
        "{r} reads of {f} files with no edit tool call to them in between",
    ),
    (
        "analytics_estimate_from_result_size",
        "about, from result size",
    ),
    // 11. Retried tool calls.
    ("analytics_pattern_retried_calls", "Retried tool calls"),
    (
        "analytics_pattern_retried_calls_count",
        "{c} calls repeated with the same tool and arguments, with only reads in between",
    ),
    // 12. Edit, failed command, edit (owner decision D9, open).
    (
        "analytics_pattern_edit_fail_edit",
        "Edit, failed command, edit",
    ),
    (
        "analytics_pattern_edit_fail_edit_count",
        "{l} times a file was edited, a command failed, and the same file was edited again.",
    ),
    (
        "analytics_inferred_from_order",
        "inferred from the order of tool calls",
    ),
    // 13. Long context.
    ("analytics_pattern_long_context", "Long context"),
    (
        "analytics_pattern_long_context_line",
        "Input sent after context passed {threshold}",
    ),
    ("analytics_from_counters", "from counters"),
    // 14.
    (
        "analytics_claude_sessions_only",
        "Claude Code sessions only: {k} of {n}.",
    ),
    ("analytics_see_sessions", "See {n} sessions →"),
    // 15. Re-read table (owner decision D8, open: letters, never a name).
    ("analytics_reread_file", "File"),
    ("analytics_reread_reads", "Reads"),
    (
        "analytics_reread_after_shrink",
        "After context shrank (inferred)",
    ),
    ("analytics_reread_tokens", "Tokens"),
    ("analytics_file_label", "File {letter} · {ext}"),
    ("analytics_file_label_no_ext", "File {letter}"),
    // 16. Goals.
    ("analytics_goal_add", "Add goal"),
    ("analytics_goal_cache_share", "Input read from cache ≥ {p}%"),
    ("analytics_goal_repeated_reads", "Repeated reads under {t}"),
    ("analytics_goal_long_context", "Long context under {t}"),
    ("analytics_goal_weekly_tokens", "Weekly tokens under {t}"),
    ("analytics_goal_met", "Met"),
    ("analytics_goal_not_met", "Not met"),
    ("analytics_goal_down_from", "Down from {x} last week."),
    ("analytics_goal_up_from", "Up from {x} last week."),
    (
        "analytics_goals_note",
        "Goals are compared only with your own past weeks.",
    ),
    ("analytics_goal_delete", "Delete goal"),
    // 17. Lever (an observation only; its advice sentence is held, D2).
    (
        "analytics_lever_line",
        "{f} files were read again {r} times, about {t} tokens.",
    ),
    ("analytics_lever_show_reads", "Show the reads"),
    ("analytics_lever_not_useful", "Not useful"),
    (
        "analytics_lever_off",
        "Lever suggestions are off. Turn them on in Settings.",
    ),
    // 18. Markers.
    (
        "analytics_marker_cache_rewrite",
        "Cache written again after a {m} min pause (inferred)",
    ),
    (
        "analytics_marker_cache_rewrite_detail",
        "Turn {t} read far less from cache and wrote {x} tokens to it.",
    ),
    ("analytics_marker_crossed", "Context passed {threshold}"),
    (
        "analytics_marker_crossed_detail",
        "Every turn after this one is in the long-context range.",
    ),
    ("analytics_marker_shrank", "Context shrank at turn {t}"),
    ("analytics_marker_inferred", "Inferred from usage counters."),
    ("analytics_marker_reread", "File {letter} read again"),
    (
        "analytics_marker_reread_detail",
        "No edit tool call to it since the last read.",
    ),
    // A re-read is read from the order of tool calls, not from usage
    // counters, so its card carries this label instead. Not in the spec's
    // list; added so no shell authors it.
    (
        "analytics_marker_from_tool_calls",
        "From the order of tool calls.",
    ),
    // 19. Session header.
    (
        "analytics_session_header",
        "{date} · {harness} · {n} turns · {t} tokens · {span} between first and last event",
    ),
    ("analytics_turn", "Turn {n}"),
    (
        "analytics_codex_not_recorded",
        "Turn-by-turn usage isn't recorded for Codex sessions.",
    ),
    ("analytics_proxy_session", "Proxy session {label}"),
    ("analytics_proxy_retries", "proxy retries, not tool retries"),
    // 20. Glance and tip (the tip is held until D2 and a user threshold).
    (
        "analytics_glance_line",
        "{tokens} tokens · {p}% of input from cache",
    ),
    ("analytics_glance_routed_only", "Routed calls only"),
    (
        "analytics_tip_context",
        "Context is at {ctx} in your current session.",
    ),
    ("analytics_tip_threshold", "Your threshold is {threshold}."),
    ("analytics_tip_mute", "Mute tips"),
    (
        "analytics_mark_a11y_tip",
        "New: a context tip for your current session.",
    ),
    // 21. Weekly summary card (feed T only). Its title is
    // `analytics_your_week`.
    (
        "analytics_recap_fewer",
        "{n} sessions · {p}% fewer tokens than last week",
    ),
    (
        "analytics_recap_more",
        "{n} sessions · {p}% more tokens than last week",
    ),
    (
        "analytics_recap_best_cache",
        "Best cache week yet: {p}%. Previous best {q}%.",
    ),
    ("analytics_recap_goal_met", "Goal met: {goal}."),
    ("analytics_recap_goal_not_met", "Goal not met: {goal}."),
    (
        "analytics_recap_pattern_up",
        "{pattern} went up {p}% against your usual week.",
    ),
    (
        "analytics_recap_threshold",
        "{n} sessions passed your {threshold} threshold.",
    ),
    ("analytics_recap_open", "Open recap"),
    ("analytics_recap_turn_off", "Turn off"),
    // 22. Weekly summary notification sentence (no notification is sent
    // until owner decision D1).
    (
        "analytics_recap_notification",
        "Last week: {t} tokens across {n} sessions, usage known for {k} of {n}.",
    ),
    // 23. Settings.
    ("analytics_setting_context_threshold", "Context threshold"),
    ("analytics_setting_not_set", "Not set"),
    (
        "analytics_setting_recap_card",
        "Show the weekly summary card",
    ),
    ("analytics_setting_lever", "Lever suggestions"),
    // 24. The Spend chip is `analytics_later`.
];
