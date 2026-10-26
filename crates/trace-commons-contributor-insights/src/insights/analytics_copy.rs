//! The token analytics words: tabs, the Overview, Patterns and Sessions
//! cards, the glance card and tip, the weekly summary card and Settings.
//!
//! The subscription-limit entries and Models card (section 24) were added
//! after the original copy review. The other entries are approved: Ron
//! approved the words in his 2026-10-11 owner review of #1287 (owner decision
//! D17), and the owner adopted that review.
//! Trace-related words say "trace", not "session", except the Sessions tab
//! (with its card title, drill-down column header and Analyze's lines that
//! name it) and the context tip's "current session". A subscription provider's
//! Session quota window is a separate concept, not a name for a trace.
//! No shell authors any of these sentences; each reads this table through
//! `ui_copy()` (`tc_insights_copy_json`, or the `copy` operation of
//! `tc_insights_call`).
//!
//! Every number or name a sentence carries is a `{name}` hole from
//! [`ANALYTICS_PLACEHOLDERS`]; the shell fills it and adds nothing else.
//!
//! Held, and so absent, until owner decision D2: every advice sentence, the
//! what-if sentences and anything that recommends an action (held again in
//! Ron's 2026-10-11 review). No run of weeks is counted anywhere (owner
//! decision D1: six weekly marks, no streaks). No amount or currency sign
//! appears here: the Models card's money (owner decision D5, settled
//! 2026-10-20) is a `{x}` hole the shell fills, inside a sentence that
//! says which price it is.
//!
//! The glance, tip and news-mark words (`analytics_glance_*`,
//! `analytics_tip_*`, `analytics_mark_a11y_tip`) stay in this table: the
//! macOS menu-bar panel reads them through `tc_insights_copy_json`, like every
//! other Insights surface, rather than from the Monitor screens table.

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
    "time",
    "harness",
    "login",
    "span",
    "label",
    "tokens",
    // The route split's verified and outside proxy tokens. Its
    // failed-check tokens are `f`, shared with the file counts.
    "v",
    "o",
    // A watched trace's folder label (`list_projects`' label).
    "folder",
    // The weekday picked on "Tokens by day".
    "day",
];

/// Key, words. Approved in Ron's 2026-10-11 owner review (D17).
pub const ANALYTICS_COPY: &[(&str, &str)] = &[
    // The figure that is not shown: unknown, or not computed under this feed.
    ("analytics_unavailable", "—"),
    // 1. Tabs and headings.
    ("analytics_tab_overview", "Overview"),
    ("analytics_tab_patterns", "Patterns"),
    ("analytics_tab_sessions", "Sessions"),
    ("analytics_tab_analyze", "Analyze"),
    ("analytics_this_week", "This week"),
    ("analytics_today", "Today"),
    ("analytics_where_tokens_went", "Where tokens went"),
    ("analytics_lever_title", "Higher than usual"),
    ("analytics_goals_title", "Goals"),
    ("analytics_reread_title", "Most re-read files"),
    ("analytics_input_each_turn", "Input sent each turn"),
    ("analytics_your_week", "Your week · {range}"),
    ("analytics_sample_data", "Sample data"),
    // Current native subscription limits; separate from historical analytics.
    ("analytics_subscriptions_title", "Subscription limits"),
    ("analytics_subscriptions_current", "Current account limits"),
    ("analytics_subscriptions_provider_codex", "Codex"),
    ("analytics_subscriptions_provider_claude", "Claude"),
    (
        "analytics_subscriptions_captured_limits",
        "Last captured account limits",
    ),
    (
        "analytics_subscriptions_claude_detail",
        "Reads your Claude Code sign-in only when you press Refresh. macOS may ask to allow access.",
    ),
    (
        "analytics_subscriptions_refresh_hint",
        "Refresh to see your latest limits.",
    ),
    (
        "analytics_subscriptions_refresh_login_accessibility",
        "Refresh {harness} usage for {login}",
    ),
    (
        "analytics_subscriptions_claude_expired",
        "The saved Claude usage token expired. Open Claude Code, then choose Refresh.",
    ),
    (
        "analytics_subscriptions_claude_auth_missing",
        "Claude usage credentials aren't available. Open Claude Code, then choose Refresh.",
    ),
    (
        "analytics_subscriptions_claude_scope",
        "This Claude sign-in cannot read usage. Check usage in Claude Code.",
    ),
    (
        "analytics_subscriptions_claude_keychain_denied",
        "Keychain access wasn't granted. Choose Refresh to try again.",
    ),
    (
        "analytics_subscriptions_claude_keychain_pending",
        "Keychain access hasn't finished. Check for a macOS prompt before choosing Refresh again.",
    ),
    (
        "analytics_subscriptions_claude_auth_changed",
        "Claude credentials changed while usage was read. Choose Refresh again.",
    ),
    (
        "analytics_subscriptions_request_timeout",
        "The usage request timed out. Choose Refresh to try again.",
    ),
    (
        "analytics_subscriptions_rate_limited",
        "Usage checks are temporarily limited. Try Refresh later.",
    ),
    ("analytics_subscriptions_read", "Read usage"),
    ("analytics_subscriptions_stop", "Stop reading"),
    (
        "analytics_subscriptions_refresh_accessibility",
        "Refresh subscription limits",
    ),
    (
        "analytics_subscriptions_read_accessibility",
        "Read {harness} subscription usage",
    ),
    (
        "analytics_subscriptions_stop_accessibility",
        "Stop reading {harness} subscription usage",
    ),
    (
        "analytics_subscriptions_read_detail",
        "Enable usage reading in Settings; only Refresh asks the native app for usage.",
    ),
    // Switch labels follow CodexBar's "Show ... usage" (2026-10-27).
    ("analytics_subscriptions_setting_codex", "Show Codex usage"),
    (
        "analytics_subscriptions_setting_claude",
        "Show Claude Code usage",
    ),
    (
        "analytics_subscriptions_codex_detail",
        "Reads your Codex sign-in only when you press Refresh.",
    ),
    (
        "analytics_subscriptions_not_checked",
        "Refresh to read current usage.",
    ),
    (
        "analytics_subscriptions_native_tool_running",
        "Codex needs to renew its sign-in. Close Codex, then Refresh.",
    ),
    (
        "analytics_subscriptions_base_url_refused",
        "This Codex configuration uses an unsupported server address.",
    ),
    (
        "analytics_subscriptions_invalid_response",
        "{harness} returned usage this app cannot read.",
    ),
    (
        "analytics_subscriptions_no_logins",
        "No saved {harness} sign-in was found.",
    ),
    ("analytics_subscriptions_window_hours", "{n} hours"),
    ("analytics_subscriptions_window_hour", "1 hour"),
    ("analytics_subscriptions_window_minutes", "{n} minutes"),
    ("analytics_subscriptions_window_minute", "1 minute"),
    ("analytics_subscriptions_window_days", "{n} days"),
    ("analytics_subscriptions_window_day", "1 day"),
    (
        "analytics_subscriptions_window_unknown",
        "Duration not reported",
    ),
    (
        "analytics_subscriptions_native_storage_unsupported",
        "Usage is unavailable for this Codex sign-in storage. Check limits in Codex.",
    ),
    ("analytics_subscriptions_default_login", "Default login"),
    ("analytics_subscriptions_session", "Session"),
    ("analytics_subscriptions_weekly", "Weekly"),
    ("analytics_subscriptions_model", "Model limit"),
    ("analytics_subscriptions_banked", "Banked resets"),
    ("analytics_subscriptions_banked_count", "{n} available"),
    ("analytics_subscriptions_remaining", "{n} remaining"),
    ("analytics_subscriptions_resets", "Resets {time}"),
    ("analytics_subscriptions_checked", "Checked {time}"),
    (
        "analytics_subscriptions_stale",
        "Last reading; refresh to check current usage.",
    ),
    (
        "analytics_subscriptions_sign_in",
        "Sign in with {harness} to read usage.",
    ),
    (
        "analytics_subscriptions_native_tool",
        "Install or sign in with {harness} to read usage.",
    ),
    (
        "analytics_subscriptions_keychain",
        "Refresh to allow Keychain access for {harness} usage.",
    ),
    ("analytics_subscriptions_cached", "Last reading."),
    ("analytics_subscriptions_reading", "Reading usage…"),
    (
        "analytics_subscriptions_not_reported",
        "Limits aren't reported for this account.",
    ),
    (
        "analytics_subscriptions_unavailable",
        "Usage couldn't be read.",
    ),
    (
        "analytics_subscriptions_change_failed",
        "The usage setting couldn't be saved.",
    ),
    (
        "analytics_subscriptions_truncated",
        "Some saved logins aren't shown.",
    ),
    // 2. Coverage.
    (
        "analytics_coverage_line",
        "Counts known for {k} of {n} traces · {p} partial · {u} unknown, not counted as zero",
    ),
    (
        "analytics_coverage_undated",
        "{d} traces have no dates and aren't in any week.",
    ),
    (
        "analytics_coverage_overlap",
        "{r} traces overlap an earlier import and are counted once.",
    ),
    (
        "analytics_coverage_too_few",
        "Not enough traces this week to compare.",
    ),
    // Coverage states and reasons, for the drill-down's Coverage and Reason
    // columns. Not enumerated in the spec's copy list; added so no shell
    // shows a wire label.
    ("analytics_state_known", "Known"),
    ("analytics_state_partial", "Partial"),
    ("analytics_state_unknown", "Unknown"),
    (
        "analytics_reason_codex_baseline_excluded",
        "Usage before the first reading isn't counted",
    ),
    (
        "analytics_reason_truncated",
        "Too many turns; some weren't kept",
    ),
    (
        "analytics_reason_some_turns_unknown",
        "Some turns have no token counts",
    ),
    ("analytics_reason_spans_weeks", "Spans more than one week"),
    ("analytics_reason_no_usage_counters", "No token counts"),
    (
        "analytics_reason_source_unsupported",
        "This format has no token counts",
    ),
    ("analytics_reason_undated", "No dates"),
    (
        "analytics_reason_reimport_overlap",
        "Overlaps an earlier import",
    ),
    (
        "analytics_reason_not_routed",
        "Not routed through Private AI",
    ),
    ("analytics_reason_stale", "Not read recently"),
    // A route qualifier, not a coverage reason: some of the session's calls
    // carry a proof label and some do not, so its category is the labelled
    // calls'. It never marks the transcript's token figure unknown.
    (
        "analytics_reason_some_calls_unrecorded",
        "Some calls have no record of where they went",
    ),
    // 3. Feed lines.
    (
        "analytics_feed_saved",
        "Only traces you analyzed are counted.",
    ),
    (
        "analytics_feed_ledger",
        "Only calls routed through Private AI are counted.",
    ),
    (
        "analytics_feed_counter_pass",
        "Traces in your watched folders.",
    ),
    (
        "analytics_feed_counter_pass_unavailable",
        "Can't count watched folders right now.",
    ),
    (
        "analytics_feed_comparisons_need_counter_pass",
        "Comparing weeks needs watched-folder counting.",
    ),
    // 4. Card names.
    ("analytics_card_tokens", "Tokens"),
    ("analytics_card_cache_share", "Cache share"),
    ("analytics_card_sessions", "Sessions"),
    ("analytics_card_tokens_by_day", "Tokens by day"),
    ("analytics_card_by_model", "By model"),
    ("analytics_card_by_project", "By project"),
    ("analytics_card_by_tool", "By tool"),
    ("analytics_unknown_label", "Unknown"),
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
        "Codex · total from first to last reading",
    ),
    (
        "analytics_codex_interval",
        "Codex · whole-trace totals, not by day",
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
    ("analytics_drill_title", "Breakdown"),
    ("analytics_drill_session", "Session"),
    ("analytics_drill_tokens", "Tokens"),
    ("analytics_drill_coverage", "Coverage"),
    ("analytics_drill_reason", "Reason"),
    ("analytics_drill_show_all", "Show all {n}"),
    ("analytics_drill_show_fewer", "Show fewer"),
    // 7b. The breakdown for one day picked on "Tokens by day": Claude Code
    // sessions by their tokens that day. Codex counts intervals, not days.
    ("analytics_drill_title_day", "Breakdown · {day}"),
    ("analytics_drill_day_week", "Show week"),
    ("analytics_drill_day_none", "No tokens counted on this day."),
    (
        "analytics_drill_day_codex",
        "Codex traces can't be split by day yet.",
    ),
    // 7a. Where a feed T session's calls went (design part B), from the
    // proxy's own record. Its tokens are the proxy's per-call count, never
    // summed with or reconciled to the transcript's. Only verified proof
    // reads "verified"; no proxy record is never "not private".
    ("analytics_drill_private", "Private AI"),
    ("analytics_route_unobserved", "No record"),
    ("analytics_route_unrecorded", "Destination not recorded"),
    ("analytics_route_outside", "Not through Private AI"),
    ("analytics_route_mixed", "Partly through Private AI"),
    ("analytics_route_verified", "Through Private AI, verified"),
    (
        "analytics_route_unverified",
        "Through Private AI, not verified",
    ),
    (
        "analytics_route_check_failed",
        "Through Private AI, check failed",
    ),
    (
        "analytics_route_split",
        "Tokens: {v} verified · {u} not verified · {f} check failed · {o} outside · {n} not recorded",
    ),
    (
        "analytics_route_measure_note",
        "Private AI counts tokens per call, so its total can differ from the trace's.",
    ),
    // Shown once in place of the column while the `insights_ledger_feed`
    // switch ("Count Private AI tokens", Settings > Private AI) is off.
    (
        "analytics_route_feed_off",
        "Turn on \"Count Private AI tokens\" in Settings to see where calls went.",
    ),
    // Shown once in place of the column while there is no proxy ledger to
    // read (`routing_unavailable` "no_ledger").
    (
        "analytics_route_no_ledger",
        "No Private AI calls recorded yet.",
    ),
    // 8. By model.
    (
        "analytics_model_labels_note",
        "Model names as the agent reported them, not necessarily the model that answered.",
    ),
    // 9. Patterns intro.
    (
        "analytics_patterns_intro",
        "Found from tool calls and token counts. No trace text is stored.",
    ),
    (
        "analytics_patterns_overlap",
        "Patterns overlap, so they don't add up to your total.",
    ),
    // 10. Repeated reads.
    ("analytics_pattern_repeated_reads", "Repeated reads"),
    (
        "analytics_pattern_repeated_reads_count",
        "{r} reads of {f} files that weren't edited in between",
    ),
    // A card whose estimate rounds to 0 leads with its count (owner
    // decision 2026-10-20), and the line beneath it drops the count.
    ("analytics_pattern_repeated_reads_headline_one", "{r} read"),
    (
        "analytics_pattern_repeated_reads_headline_other",
        "{r} reads",
    ),
    (
        "analytics_pattern_repeated_reads_count_under_count_one",
        "In {f} file not edited in between",
    ),
    (
        "analytics_pattern_repeated_reads_count_under_count_other",
        "In {f} files not edited in between",
    ),
    (
        "analytics_estimate_from_result_size",
        "estimated from result size",
    ),
    // 11. Retried tool calls.
    ("analytics_pattern_retried_calls", "Retried tool calls"),
    (
        "analytics_pattern_retried_calls_count",
        "{c} calls repeated with the same tool and input, with only reads in between",
    ),
    ("analytics_pattern_retried_calls_headline_one", "{c} call"),
    (
        "analytics_pattern_retried_calls_headline_other",
        "{c} calls",
    ),
    (
        "analytics_pattern_retried_calls_count_under_count",
        "Same tool and input, with only reads in between",
    ),
    // 12. Edit, failed command, edit (owner decision D9: kept, labelled inferred).
    (
        "analytics_pattern_edit_fail_edit",
        "Edit, failed command, edit",
    ),
    (
        "analytics_pattern_edit_fail_edit_count",
        "{l} times a file was edited, a command failed, and the same file was edited again.",
    ),
    ("analytics_pattern_edit_fail_edit_headline_one", "{l} time"),
    (
        "analytics_pattern_edit_fail_edit_headline_other",
        "{l} times",
    ),
    (
        "analytics_pattern_edit_fail_edit_count_under_count",
        "File edited, command failed, same file edited again",
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
    ("analytics_pattern_long_context_headline_one", "{n} turn"),
    ("analytics_pattern_long_context_headline_other", "{n} turns"),
    ("analytics_from_counters", "from token counts"),
    // 14.
    (
        "analytics_claude_sessions_only",
        "Claude Code traces only: {k} of {n}.",
    ),
    ("analytics_see_sessions", "Show {n} traces"),
    // 15. Re-read table (owner decision D8: letters plus extension, never a name).
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
    ("analytics_goal_cache_share", "Cache share at least {p}%"),
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
    ("analytics_goal_delete", "Delete"),
    // 17. Lever (an observation only; its advice sentence is held, D2).
    (
        "analytics_lever_line",
        "{f} files were read again {r} times, about {t} tokens.",
    ),
    ("analytics_lever_show_reads", "Show reads"),
    ("analytics_lever_not_useful", "Not useful"),
    (
        "analytics_lever_off",
        "Hidden after you marked it not useful.",
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
    ("analytics_marker_inferred", "Inferred from token counts."),
    ("analytics_marker_reread", "File {letter} read again"),
    (
        "analytics_marker_reread_detail",
        "Not edited since the last read.",
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
        "{date} · {harness} · {n} turns · {t} tokens · {span} long",
    ),
    // A saved session's label in the session picker and the drill-down rows,
    // so neither shows the snapshot ID: the first recorded event's local date
    // and time, the harness and the session's tokens. No project name (owner
    // decision D7, open).
    (
        "analytics_session_label",
        "{date} {time} · {harness} · {t} tokens",
    ),
    // Counter breakdown time span, ending at the latest recorded activity.
    (
        "analytics_session_label_range",
        "{span} · {harness} · {t} tokens",
    ),
    // The same label for a session with no recorded event time.
    (
        "analytics_session_label_undated",
        "Undated · {harness} · {t} tokens",
    ),
    // A watched trace's label in the session picker under feed T (owner
    // decision 2026-10-20): its folder's label, its first recorded event's
    // local date and time, the harness and its turns. A Codex trace records
    // no turns, so its label has none.
    (
        "analytics_watched_label",
        "{folder} · {date} {time} · {harness} · {n} turns",
    ),
    (
        "analytics_watched_label_no_turns",
        "{folder} · {date} {time} · {harness}",
    ),
    // The same label for a trace with no recorded event time.
    (
        "analytics_watched_label_undated",
        "{folder} · Undated · {harness}",
    ),
    // `{folder}` when no folder the daemon knows matches the trace's.
    ("analytics_folder_unknown", "Unknown folder"),
    // A Sessions feed row under feed T (owner request 2026-10-20): its
    // tokens beside its sparkline. A row with no counts reads
    // `analytics_reason_no_usage_counters` once instead, with no sparkline.
    ("analytics_session_row_tokens", "{t} tokens"),
    // The sparkline in words, for VoiceOver: its largest turn.
    ("analytics_sparkline_peak", "Peak {t} tokens at turn {n}"),
    // The row at the foot of the feed while the next page is read.
    (
        "analytics_sessions_explainer_watched",
        "Your Claude Code and Codex traces, newest first. Open one to see where its tokens went, label it, or analyze it.",
    ),
    (
        "analytics_sessions_explainer_saved",
        "Traces you analyzed and saved. Pick one to see where its tokens went, turn by turn.",
    ),
    ("analytics_sessions_loading_more", "Loading more\u{2026}"),
    // The feed foot's button after a page could not be read; same wording as
    // the startup retry in `health_copy`.
    ("analytics_sessions_retry", "Try again"),
    (
        "analytics_sessions_page_failed",
        "More traces couldn't be loaded.",
    ),
    // A Sessions feed row's own labels (Insights PM request, 2026-10-21):
    // the task, pre-filled from the inferred one, and what became of the
    // work. Kept on this Mac only. `analytics_task_<wire>` and
    // `analytics_outcome_<wire>` name each closed value
    // (`session_labels::LabelledTask`, `SessionOutcome`).
    ("analytics_label_task", "Task"),
    ("analytics_label_outcome", "Outcome"),
    ("analytics_task_bug_fix", "Bug fix"),
    ("analytics_task_refactor", "Refactor"),
    ("analytics_task_tests", "Tests"),
    ("analytics_task_docs", "Docs"),
    ("analytics_task_other", "Other"),
    // The task the rules inferred, until the person sets one.
    ("analytics_task_inferred", "{label} · inferred"),
    // Takes the inferred task as the person's own, in one click.
    ("analytics_task_confirm", "Confirm"),
    // Clears the person's task, so the inferred one shows again.
    ("analytics_task_use_inferred", "Use inferred"),
    ("analytics_outcome_kept", "Kept"),
    ("analytics_outcome_partly_kept", "Partly kept"),
    ("analytics_outcome_thrown_away", "Thrown away"),
    ("analytics_outcome_clear", "Clear"),
    // Each row's controls for VoiceOver, so the rows' repeated controls
    // are told apart: `{label}` is the row's own label.
    ("analytics_label_task_a11y", "Task, {label}"),
    ("analytics_label_outcome_a11y", "Outcome, {label}"),
    ("analytics_task_confirm_a11y", "Confirm task, {label}"),
    // Under a row whose label could not be saved; the row shows its label
    // as it was.
    (
        "analytics_label_save_failed",
        "The label couldn't be saved.",
    ),
    // Analyze reads a watched trace by its Sessions row, never a file the
    // person picks (Insights PM, 2026-10-21: terminal users never see their
    // trace files). The open row's one action analyzes it and saves the
    // snapshot, which then opens in Analyze.
    ("analytics_analyze_save", "Analyze & save"),
    ("analytics_analyze_save_a11y", "Analyze and save, {label}"),
    // Under the open row when the daemon could not analyze it.
    (
        "analytics_analyze_failed",
        "This trace couldn't be analyzed.",
    ),
    // Its file was moved or deleted since it was counted.
    (
        "analytics_analyze_file_gone",
        "This trace's file is no longer where it was found.",
    ),
    // The row was listed again under a new reference (its file grew) or
    // its folder became Never.
    (
        "analytics_analyze_unknown",
        "This trace has changed since it was listed. Try again once the list refreshes.",
    ),
    // Analyze's head while nothing is saved, and once something is.
    (
        "analytics_analyze_empty",
        "No saved insights yet. Open a trace in Sessions and choose Analyze & save.",
    ),
    (
        "analytics_analyze_how",
        "To add a trace, open it in Sessions and choose Analyze & save.",
    ),
    // Behind Analyze's "i".
    (
        "analytics_analyze_intro",
        "Analyze & save reads a watched trace on this device and keeps a dated snapshot of what it found. Nothing is uploaded. Analyze it again to refresh it.",
    ),
    // In place of Analyze & save on a row the daemon says it can't analyze
    // (`analyzable: false`), and under the row when the daemon refuses it
    // for that reason: the core's whole-file analysis takes files up to
    // its size limit, and a Codex file with exactly one trace in it.
    (
        "analytics_analyze_not_analyzable",
        "This trace can't be analyzed: its file is too large or doesn't hold exactly one trace.",
    ),
    (
        "analytics_analyze_too_large",
        "This trace's file is too large to analyze.",
    ),
    (
        "analytics_analyze_codex_not_one",
        "This Codex file doesn't hold exactly one trace, so it can't be analyzed.",
    ),
    ("analytics_turn", "Turn {n}"),
    (
        "analytics_codex_not_recorded",
        "Per-turn counts aren't shown for Codex traces.",
    ),
    // 20. Glance and tip (the tip is held until D2 and a user threshold).
    (
        "analytics_glance_line",
        "{tokens} tokens · {p}% of input from cache",
    ),
    // A day whose known calls read no input, so no cache share.
    ("analytics_glance_tokens_only", "{tokens} tokens"),
    ("analytics_glance_routed_only", "Private AI calls only"),
    (
        "analytics_tip_context",
        "Context is at {ctx} in your current session.",
    ),
    ("analytics_tip_threshold", "Your threshold is {threshold}."),
    ("analytics_tip_mute", "Mute"),
    ("analytics_mark_a11y_tip", "New context tip"),
    // 21. Weekly summary card (feed T only). Its title is
    // `analytics_your_week`.
    (
        "analytics_recap_fewer",
        "{n} traces · {p}% fewer tokens than last week",
    ),
    (
        "analytics_recap_more",
        "{n} traces · {p}% more tokens than last week",
    ),
    (
        "analytics_recap_best_cache",
        "Best cache week yet: {p}%. Previous best {q}%.",
    ),
    ("analytics_recap_goal_met", "Goal met: {goal}."),
    ("analytics_recap_goal_not_met", "Goal not met: {goal}."),
    (
        "analytics_recap_pattern_up",
        "{pattern} up {p}% on your usual week.",
    ),
    (
        "analytics_recap_threshold",
        "{n} traces passed your {threshold} threshold.",
    ),
    ("analytics_recap_open", "View week"),
    ("analytics_recap_turn_off", "Hide"),
    // 22. Weekly summary notification sentence (no notification is sent
    // until owner decision D1).
    (
        "analytics_recap_notification",
        "Last week: {t} tokens across {n} traces, counts known for {k} of {n}.",
    ),
    // 23. Settings.
    ("analytics_setting_context_threshold", "Context threshold"),
    ("analytics_setting_not_set", "Not set"),
    ("analytics_setting_recap_card", "Weekly summary"),
    ("analytics_setting_lever", "Higher-than-usual card"),
    // The `insights_counter_pass` switch (owner decision D4: off by
    // default; Ron's #1287 review).
    (
        "analytics_setting_counter_pass",
        "Count tokens in watched folders",
    ),
    // The `insights_ledger_feed` switch (owner decision D3, settled
    // 2026-10-09: on by default, the switch turns it off).
    ("analytics_setting_ledger_feed", "Count Private AI tokens"),
    // 24. The Models card (feed T, `insights_model_answers`; model answers
    // spec section 8). Drafted 2026-10-21 for the first macOS version; not
    // in Ron's review. Each answer is a scoped fact about the contributor's
    // own traces, never a ranking: no "best", "better", "fastest",
    // "winner" or "benchmark". Money is a `{x}` the shell fills with the
    // amount, always beside "at API prices" or "Charged through Private
    // AI" (owner decision D5, settled 2026-10-20).
    ("analytics_card_models", "Models"),
    (
        "analytics_models_scope",
        "In your traces · labels as recorded · task type yours, else inferred",
    ),
    (
        "analytics_models_window",
        "From your traces in the last 13 weeks, whatever week is on screen.",
    ),
    // Each row's name, by the wire's `question`.
    ("analytics_models_row_bug_fix_rejected", "Bug fixing"),
    ("analytics_models_row_refactor_rejected", "Refactoring"),
    ("analytics_models_row_tests_rejected", "Tests"),
    ("analytics_models_row_docs_cost", "Docs cost"),
    (
        "analytics_models_row_rejected_across_tasks",
        "All task types",
    ),
    ("analytics_models_row_finish_time", "Time to finish"),
    // Each row's fact, by `question`, with `_undone` when the row's
    // `basis` is `undone`. A rejection figure is a share of the model's
    // edits, so its fact names a share: the lowest share can still be
    // the most edits.
    (
        "analytics_models_lead_bug_fix_rejected",
        "Lowest share of edits rejected in your bug-fix traces",
    ),
    (
        "analytics_models_lead_bug_fix_rejected_undone",
        "Lowest share of edits undone in your bug-fix traces",
    ),
    (
        "analytics_models_lead_refactor_rejected",
        "Lowest share of edits rejected in your refactoring traces",
    ),
    (
        "analytics_models_lead_refactor_rejected_undone",
        "Lowest share of edits undone in your refactoring traces",
    ),
    (
        "analytics_models_lead_tests_rejected",
        "Lowest share of edits rejected in your test traces",
    ),
    (
        "analytics_models_lead_tests_rejected_undone",
        "Lowest share of edits undone in your test traces",
    ),
    (
        "analytics_models_lead_docs_cost",
        "Lowest cost per docs trace, at API prices",
    ),
    (
        "analytics_models_lead_rejected_across_tasks",
        "Lowest share of edits rejected across your task types, weighted by task mix",
    ),
    (
        "analytics_models_lead_rejected_across_tasks_undone",
        "Lowest share of edits undone across your task types, weighted by task mix",
    ),
    (
        "analytics_models_lead_finish_time",
        "Least active time to finish, in your finished traces",
    ),
    // A model and its figure; the leading one, then the next.
    ("analytics_models_model_line", "{label} ({x}, {n} traces)"),
    (
        "analytics_models_next_line",
        "Next: {label} ({x}, {n} traces)",
    ),
    // The traces each model has, on a row short of them.
    ("analytics_models_count_line_one", "{label}: {n} trace"),
    ("analytics_models_count_line_other", "{label}: {n} traces"),
    // The figures, by `figure.kind`.
    ("analytics_models_rate", "{p}%"),
    ("analytics_models_cost", "{x} at API prices"),
    ("analytics_models_finish_one", "{span} active · {u} turn"),
    ("analytics_models_finish_other", "{span} active · {u} turns"),
    ("analytics_models_interrupts", "Interrupts per trace: {x}"),
    // On a rejection row, each model whose traces the person gave an
    // Outcome label (`kept_by_model`): the share they marked Kept, beside
    // the figure and never folded into it. Partly kept is not kept.
    (
        "analytics_models_kept_one",
        "{label}: {p}% kept, of {n} trace you marked",
    ),
    (
        "analytics_models_kept_other",
        "{label}: {p}% kept, of {n} traces you marked",
    ),
    // The row states that name no model first.
    ("analytics_models_not_enough", "Not enough traces yet"),
    ("analytics_models_not_priced", "Not priced"),
    (
        "analytics_models_no_clear_difference",
        "No clear difference",
    ),
    // What a rejection row counts, by `basis`.
    (
        "analytics_models_basis_rejected",
        "Rejected edits: ones you declined or undid",
    ),
    ("analytics_models_basis_undone", "Undone edits we could see"),
    // Which traces the time row compares, by `edit_mode`.
    (
        "analytics_models_mode_prompting",
        "Traces that asked you before editing",
    ),
    (
        "analytics_models_mode_not_prompting",
        "Traces that edited without asking",
    ),
    // What Private AI charged, on the Docs cost row only, apart from the
    // API-price figure and never added to it.
    (
        "analytics_models_charge",
        "Charged through Private AI: {x} per trace (median), {n} traces",
    ),
    (
        "analytics_models_charge_unknown",
        "Through Private AI: {n} traces, charge not recorded",
    ),
    (
        "analytics_models_charge_some_unknown",
        "Charge not recorded for {u} of them",
    ),
    // Behind a row's "i": how its figure is measured.
    (
        "analytics_models_info_rejected",
        "Rejected edits are the edits you declined plus the edits undone after you stepped in, out of every edit the model made.",
    ),
    (
        "analytics_models_info_undone",
        "Not every trace records the edits you declined, so this compares undone edits: an edit put back by a later one after you stepped in. Edits made by shell commands or in an editor aren't seen.",
    ),
    (
        "analytics_models_info_cost",
        "At API prices: what the API would charge at its published list prices, not what a plan costs. Traces sent through Private AI are counted apart.",
    ),
    (
        "analytics_models_info_finish",
        "Median active time of finished traces, without the time spent waiting for you, and the median turns you took.",
    ),
    (
        "analytics_models_info_lead",
        "A model is named first only when its lead holds when the traces are resampled.",
    ),
    (
        "analytics_models_info_no_clear_difference",
        "The gap didn't hold when the traces were resampled, so neither model is named first.",
    ),
    // Behind a row's "i": the traces left out of it, per `left_out` reason.
    ("analytics_models_left_out", "Left out of this answer:"),
    (
        "analytics_models_left_out_unknown_facts",
        "Facts couldn't be read: {n}",
    ),
    (
        "analytics_models_left_out_automated",
        "Started by a tool, not by you: {n}",
    ),
    (
        "analytics_models_left_out_delegated",
        "Handed work to subagents we can't read: {n}",
    ),
    (
        "analytics_models_left_out_capped",
        "Too long to read whole: {n}",
    ),
    (
        "analytics_models_left_out_routed",
        "Route doesn't confirm the model: {n}",
    ),
    ("analytics_models_left_out_reviewer", "Review traces: {n}"),
    (
        "analytics_models_left_out_no_majority",
        "No model on most turns: {n}",
    ),
    (
        "analytics_models_left_out_unpriced",
        "No API price for the model: {n}",
    ),
    (
        "analytics_models_left_out_too_few_sessions",
        "Model had too few traces of a task type: {n}",
    ),
    (
        "analytics_models_left_out_too_few_edits",
        "Model had too few edits: {n}",
    ),
    (
        "analytics_models_left_out_mix_too_different",
        "Model's task mix too different to compare: {n}",
    ),
];
