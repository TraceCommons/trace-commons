using System;
using System.Threading;
using System.Threading.Tasks;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The sample Insights draws when it has no trace of the person's own, and
/// what the debug sample sets open on (<c>InsightsSample</c>,
/// macos/Sources/TraceCommonsApp/InsightsSample.swift). Sample figures, never
/// a pilot's, and never mixed with the person's own. Every number here is
/// the Mac's, copied from that file.
/// </summary>
public static class InsightsSample
{
    private const string Opus = "claude-opus-5-5", Sonnet = "claude-sonnet-5-5", Haiku = "claude-haiku-5-5";
    private const string Codex = "gpt-5-codex", Claude = "claude_code", CodexTool = "codex";

    private static BenchmarkPick Pick((string Label, string Tool, long Figure) leader, (string Label, string Tool, long Figure) next) =>
        new(new BenchmarkEntry(leader.Label, leader.Tool, leader.Figure), new BenchmarkEntry(next.Label, next.Tool, next.Figure));

    private static BenchmarkCategory Category(string key, int sessions, BenchmarkPick quality, BenchmarkPick value,
        BenchmarkPick speed, BenchmarkTip? tip = null) =>
        new(key, sessions, Array.Empty<BenchmarkModel>(), quality, value, speed, tip);

    /// <summary>Four weeks of sample benchmarks: five kinds of work, two models compared on each (InsightsSample.swift:68-104).</summary>
    public static readonly InsightsBenchmarks Benchmarks = new(
        Enabled: true, Readable: true, Weeks: 4, Population: null, Sessions: 107, MinSessions: 5,
        Categories: new[]
        {
            Category("bug_fix", 34,
                Pick((Opus, Claude, 930), (Codex, CodexTool, 880)),
                Pick((Sonnet, Claude, 410_000), (Opus, Claude, 1_840_000)),
                Pick((Codex, CodexTool, 840), (Opus, Claude, 1_260)),
                new BenchmarkTip("test_first", FasterPermille: 350, SessionsWithout: 12, Sessions: 34)),
            Category("feature", 28,
                Pick((Opus, Claude, 910), (Sonnet, Claude, 860)),
                Pick((Sonnet, Claude, 620_000), (Opus, Claude, 2_450_000)),
                Pick((Sonnet, Claude, 1_500), (Opus, Claude, 2_100)),
                new BenchmarkTip("cheaper_same_quality", Cheap: Sonnet, Expensive: Opus, CostRatioPermille: 250)),
            Category("refactor", 19,
                Pick((Codex, CodexTool, 940), (Opus, Claude, 900)),
                Pick((Codex, CodexTool, 780_000), (Opus, Claude, 1_960_000)),
                Pick((Opus, Claude, 1_080), (Codex, CodexTool, 1_320)),
                new BenchmarkTip("fewer_undos", Better: Codex, Worse: Opus, RatioPermille: 2_300)),
            Category("tests", 17,
                Pick((Sonnet, Claude, 950), (Opus, Claude, 930)),
                Pick((Sonnet, Claude, 290_000), (Opus, Claude, 1_100_000)),
                Pick((Sonnet, Claude, 660), (Opus, Claude, 900)),
                new BenchmarkTip("test_first", FasterPermille: 220, SessionsWithout: 5, Sessions: 17)),
            Category("docs", 9,
                Pick((Sonnet, Claude, 970), (Haiku, Claude, 950)),
                Pick((Haiku, Claude, 60_000), (Sonnet, Claude, 210_000)),
                Pick((Haiku, Claude, 240), (Sonnet, Claude, 420))),
        });

    /// <summary>The refusal every sample read answers with (InsightsSampleWeek.swift:34).</summary>
    public const string Refusal = "insights_sample_not_answered";

    /// <summary>
    /// A Claude login and a Codex login, each with a weekly window part used,
    /// resetting a few days from <paramref name="now"/>
    /// (<c>InsightsSample.subscriptions</c>, InsightsSample.swift:177-205).
    /// </summary>
    public static SubscriptionUsage Subscriptions(DateTimeOffset now) => new(new[]
    {
        new SubscriptionUsageProvider("claude", true, Logins: new[]
        {
            new SubscriptionUsageLogin("sample-claude", "Claude", "observed", DefaultLogin: true, UpdatedAt: now, Windows: new[]
            {
                new SubscriptionUsageWindow("session", UsedPermille: 340, ResetsAt: now.AddHours(2.4), DurationMins: 300),
                new SubscriptionUsageWindow("weekly", UsedPermille: 410, ResetsAt: now.AddDays(3.2), DurationMins: 10_080),
            }),
        }),
        new SubscriptionUsageProvider("codex", true, Logins: new[]
        {
            new SubscriptionUsageLogin("sample-codex", "Codex", "observed", DefaultLogin: true, UpdatedAt: now, Windows: new[]
            {
                new SubscriptionUsageWindow("weekly", UsedPermille: 220, ResetsAt: now.AddDays(4.6), DurationMins: 10_080),
            }),
        }),
    });

    /// <summary>
    /// The sample week as <c>insights_week</c> sends it (feed T): the Mac's
    /// <c>InsightsSample.weekObject</c> (InsightsSample.swift:105-140) over
    /// <c>InsightsSampleWeek</c>'s Overview, Patterns and 37 counted sessions
    /// (InsightsSampleWeek.swift:96-301), copied figure for figure so the two
    /// shells draw the same week. The Claude Code sessions' days sum exactly to
    /// <c>by_day</c>; coverage is 34 known, 2 partial, 1 unknown. The Mac's
    /// <c>work</c> and <c>goals</c> are copied too (InsightsSample.swift:34-63,
    /// :150-167); its <c>history</c> is empty and it has no <c>near_ai_credit</c>,
    /// so the sample draws no summary card and no NEAR AI card, as on the Mac.
    /// Not recorded from a daemon: the counter pass
    /// that fills <c>rollup</c> runs only from the watcher, so the K2 recorder
    /// cannot produce it, and the Mac hand-shapes this week for the same reason.
    /// </summary>
    public static JsonElement Week
    {
        get
        {
            using JsonDocument document = JsonDocument.Parse(WeekJson);
            return document.RootElement.Clone();
        }
    }

    private const string WeekJson = """
        {
          "enabled": true,
          "feed": "counter_pass",
          "readable": true,
          "iso_week": "2026-W41",
          "week_start": "2026-10-05",
          "comparable": true,
          "change_vs_last_week": [{"source": "claude_code", "permille": -120}, {"source": "codex", "permille": 40}],
          "rollup": {
            "week_start": "2026-10-05",
            "coverage": {"known": 34, "partial": 2, "unknown": 1, "reasons": {}},
            "undated_sessions": 0,
            "sources": [{"source": "claude_code", "sessions": 31, "tokens": 45000000, "largest_session_tokens": 4400000, "cache_share": {"numerator": 38500000, "denominator": 44150000}}, {"source": "codex", "sessions": 6, "tokens": 6200000, "largest_session_tokens": 1300000, "cache_share": {"numerator": 4588000, "denominator": 6200000}}],
            "codex_interval_tokens": 6200000,
            "by_model": [],
            "sessions": [
              {"source": "claude_code", "folder": "infra", "tokens": 960000, "by_day": [0, 0, 0, 0, 0, 0, 960000], "state": "known", "reasons": [], "started_at": "2026-10-11T14:09:00Z", "last_event_at": "2026-10-11T16:57:00Z", "session_ref": "t1"},
              {"source": "claude_code", "folder": "scripts", "tokens": 390000, "by_day": [0, 0, 0, 0, 0, 0, 390000], "state": "known", "reasons": [], "started_at": "2026-10-11T10:23:00Z", "last_event_at": "2026-10-11T12:19:00Z", "session_ref": "t2"},
              {"source": "claude_code", "folder": "design-system", "tokens": 580000, "by_day": [0, 0, 0, 0, 0, 0, 580000], "state": "known", "reasons": [], "started_at": "2026-10-11T09:16:00Z", "last_event_at": "2026-10-11T10:08:00Z", "session_ref": "t3"},
              {"source": "claude_code", "folder": "website", "tokens": 490000, "by_day": [0, 0, 0, 0, 0, 490000, 0], "state": "known", "reasons": [], "started_at": "2026-10-10T16:55:00Z", "last_event_at": "2026-10-10T18:35:00Z", "session_ref": "t4"},
              {"source": "claude_code", "folder": "notes", "tokens": 330000, "by_day": [0, 0, 0, 0, 0, 330000, 0], "state": "known", "reasons": [], "started_at": "2026-10-10T11:02:00Z", "last_event_at": "2026-10-10T12:46:00Z", "session_ref": "t5"},
              {"source": "codex", "folder": "website", "tokens": 700000, "state": "known", "reasons": [], "started_at": "2026-10-09T17:40:00Z", "last_event_at": "2026-10-09T18:25:00Z", "session_ref": "t6"},
              {"source": "claude_code", "folder": "trace-commons", "tokens": 620000, "by_day": [0, 0, 0, 0, 620000, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-09T15:48:00Z", "last_event_at": "2026-10-09T16:24:00Z", "session_ref": "t7"},
              {"source": "claude_code", "folder": "scripts", "tokens": 1240000, "by_day": [0, 0, 0, 0, 1240000, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-09T13:41:00Z", "last_event_at": "2026-10-09T15:13:00Z", "session_ref": "t8"},
              {"source": "claude_code", "folder": "design-system", "tokens": 1860000, "by_day": [0, 0, 0, 0, 1860000, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-09T10:34:00Z", "last_event_at": "2026-10-09T11:02:00Z", "session_ref": "t9"},
              {"source": "claude_code", "folder": "infra", "tokens": 2730000, "by_day": [0, 0, 0, 0, 2480000, 250000, 0], "state": "partial", "reasons": ["some_turns_unknown"], "started_at": "2026-10-09T09:27:00Z", "last_event_at": "2026-10-10T01:51:00Z", "session_ref": "t10"},
              {"source": "codex", "folder": "trace-commons", "tokens": 900000, "state": "known", "reasons": [], "started_at": "2026-10-09T08:20:00Z", "last_event_at": "2026-10-09T09:07:00Z", "session_ref": "t11"},
              {"source": "claude_code", "folder": "notes", "tokens": null, "by_day": null, "state": "unknown", "reasons": ["no_usage_counters"], "started_at": "2026-10-08T19:05:00Z", "last_event_at": "2026-10-08T19:40:00Z", "session_ref": "t12"},
              {"source": "claude_code", "folder": "trace-commons", "tokens": 1510000, "by_day": [0, 0, 0, 1510000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-08T16:06:00Z", "last_event_at": "2026-10-08T17:18:00Z", "session_ref": "t13"},
              {"source": "claude_code", "folder": "scripts", "tokens": 1810000, "by_day": [0, 0, 0, 1810000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-08T15:59:00Z", "last_event_at": "2026-10-08T17:07:00Z", "session_ref": "t14"},
              {"source": "claude_code", "folder": "notes", "tokens": 750000, "by_day": [0, 0, 0, 750000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-08T14:20:00Z", "last_event_at": "2026-10-08T15:40:00Z", "session_ref": "t15"},
              {"source": "claude_code", "folder": "design-system", "tokens": 2260000, "by_day": [0, 0, 0, 2260000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-08T13:52:00Z", "last_event_at": "2026-10-08T14:56:00Z", "session_ref": "t16"},
              {"source": "claude_code", "folder": "website", "tokens": 1210000, "by_day": [0, 0, 0, 1210000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-08T11:13:00Z", "last_event_at": "2026-10-08T13:29:00Z", "session_ref": "t17"},
              {"source": "codex", "folder": "infra", "tokens": 1000000, "state": "partial", "reasons": ["codex_baseline_excluded"], "started_at": "2026-10-08T09:30:00Z", "last_event_at": "2026-10-08T10:45:00Z", "session_ref": "t18"},
              {"source": "claude_code", "folder": "scripts", "tokens": 2200000, "by_day": [0, 0, 2200000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T16:17:00Z", "last_event_at": "2026-10-07T18:01:00Z", "session_ref": "t19"},
              {"source": "claude_code", "folder": "design-system", "tokens": 4400000, "by_day": [0, 0, 3600000, 800000, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T15:10:00Z", "last_event_at": "2026-10-08T01:50:00Z", "session_ref": "t20"},
              {"source": "claude_code", "folder": "website", "tokens": 1200000, "by_day": [0, 0, 1200000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T14:31:00Z", "last_event_at": "2026-10-07T16:23:00Z", "session_ref": "t21"},
              {"source": "claude_code", "folder": "trace-commons", "tokens": 1430000, "by_day": [0, 0, 1430000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T11:24:00Z", "last_event_at": "2026-10-07T12:12:00Z", "session_ref": "t22"},
              {"source": "claude_code", "folder": "infra", "tokens": 650000, "by_day": [0, 0, 650000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T10:45:00Z", "last_event_at": "2026-10-07T12:45:00Z", "session_ref": "t23"},
              {"source": "claude_code", "folder": "notes", "tokens": 900000, "by_day": [0, 0, 900000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-07T09:38:00Z", "last_event_at": "2026-10-07T10:34:00Z", "session_ref": "t24"},
              {"source": "codex", "folder": "trace-commons", "tokens": 1200000, "state": "known", "reasons": [], "started_at": "2026-10-07T08:41:00Z", "last_event_at": "2026-10-07T09:58:00Z", "session_ref": "t25"},
              {"source": "claude_code", "folder": "trace-commons", "tokens": 2360000, "by_day": [0, 2360000, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-06T14:42:00Z", "last_event_at": "2026-10-06T15:06:00Z", "session_ref": "t26"},
              {"source": "claude_code", "folder": "infra", "tokens": 950000, "by_day": [0, 950000, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-06T13:03:00Z", "last_event_at": "2026-10-06T15:39:00Z", "session_ref": "t27"},
              {"source": "claude_code", "folder": "scripts", "tokens": 3810000, "by_day": [0, 3310000, 500000, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-06T11:35:00Z", "last_event_at": "2026-10-07T01:55:00Z", "session_ref": "t28"},
              {"source": "claude_code", "folder": "notes", "tokens": 1130000, "by_day": [0, 1130000, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-06T10:56:00Z", "last_event_at": "2026-10-06T11:28:00Z", "session_ref": "t29"},
              {"source": "claude_code", "folder": "website", "tokens": 1700000, "by_day": [0, 1700000, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-06T09:49:00Z", "last_event_at": "2026-10-06T11:17:00Z", "session_ref": "t30"},
              {"source": "codex", "folder": "scripts", "tokens": 1300000, "state": "known", "reasons": [], "started_at": "2026-10-05T18:10:00Z", "last_event_at": "2026-10-05T19:20:00Z", "session_ref": "t31"},
              {"source": "claude_code", "folder": "design-system", "tokens": 710000, "by_day": [710000, 0, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-05T16:28:00Z", "last_event_at": "2026-10-05T17:44:00Z", "session_ref": "t32"},
              {"source": "claude_code", "folder": "infra", "tokens": 1070000, "by_day": [1070000, 0, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-05T15:21:00Z", "last_event_at": "2026-10-05T17:33:00Z", "session_ref": "t33"},
              {"source": "claude_code", "folder": "notes", "tokens": 1420000, "by_day": [1420000, 0, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-05T13:14:00Z", "last_event_at": "2026-10-05T14:22:00Z", "session_ref": "t34"},
              {"source": "claude_code", "folder": "website", "tokens": 1780000, "by_day": [1780000, 0, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-05T10:07:00Z", "last_event_at": "2026-10-05T12:11:00Z", "session_ref": "t35"},
              {"source": "codex", "folder": "notes", "tokens": 1100000, "state": "known", "reasons": [], "started_at": "2026-10-05T09:40:00Z", "last_event_at": "2026-10-05T10:20:00Z", "session_ref": "t36"},
              {"source": "claude_code", "folder": "trace-commons", "tokens": 2440000, "by_day": [2140000, 300000, 0, 0, 0, 0, 0], "state": "known", "reasons": [], "started_at": "2026-10-05T09:00:00Z", "last_event_at": "2026-10-06T01:00:00Z", "session_ref": "t37"}
            ]
          },
          "overview": {
            "feed": "counter_pass",
            "week_start": "2026-10-05",
            "week_end": "2026-10-11",
            "tz": 0,
            "coverage": {"known": 34, "partial": 2, "unknown": 1, "reasons": {}},
            "undated_sessions": 0,
            "sessions": 37,
            "sources": [
              {"source": "claude_code", "sessions": 31, "tokens": 45000000, "cache_share": {"numerator": 38500000, "denominator": 44150000, "permille": 872}, "largest_session": {"tokens": 4400000}, "change_permille": -120, "best": {"previous_best_permille": 840, "is_new_best": true}},
              {"source": "codex", "sessions": 6, "tokens": 6200000, "cache_share": {"numerator": 4588000, "denominator": 6200000, "permille": 740}, "largest_session": {"tokens": 1300000}, "change_permille": 40, "best_week": "not_best"}
            ],
            "by_day": [
              {"date": "2026-10-05", "uncached": 400000, "cache_read": 6100000, "cache_write": 500000, "output": 120000},
              {"date": "2026-10-06", "uncached": 600000, "cache_read": 8300000, "cache_write": 700000, "output": 150000},
              {"date": "2026-10-07", "uncached": 500000, "cache_read": 9000000, "cache_write": 800000, "output": 180000},
              {"date": "2026-10-08", "uncached": 400000, "cache_read": 7200000, "cache_write": 600000, "output": 140000},
              {"date": "2026-10-09", "uncached": 300000, "cache_read": 5400000, "cache_write": 400000, "output": 100000},
              {"date": "2026-10-10", "uncached": 50000, "cache_read": 900000, "cache_write": 100000, "output": 20000},
              {"date": "2026-10-11", "uncached": 100000, "cache_read": 1600000, "cache_write": 200000, "output": 30000}
            ],
            "codex_interval_tokens": 6200000,
            "by_model": [
              {"label": "claude-haiku-5-5", "tokens": 2100000},
              {"label": "claude-opus-5-5", "tokens": 29000000},
              {"label": "claude-sonnet-5-5", "tokens": 14000000},
              {"label": "gpt-5-codex", "tokens": 6200000},
              {"label": null, "tokens": 800000}
            ],
            "by_tool": [{"source": "claude_code", "tokens": 45000000}, {"source": "codex", "tokens": 6200000}],
            "by_project": "not_available_for_analyzed_files",
            "weeks": ["2026-10-05", "2026-09-28", "2026-09-21", "2026-09-07", "2026-08-31"]
          },
          "patterns": {
            "feed": "counter_pass",
            "week_start": "2026-10-05",
            "week_end": "2026-10-11",
            "tz": 0,
            "coverage": {"known": 34, "partial": 2, "unknown": 1, "reasons": {}},
            "sessions": 37,
            "claude_sessions": 31,
            "claude_only": true,
            "long_context_threshold": 150000,
            "cards": [],
            "reread_files": [],
            "weeks": ["2026-10-05", "2026-09-28", "2026-09-21", "2026-09-07", "2026-08-31"]
          },
          "history": [],
          "work": {"basis": "api_list", "week": {"sessions": 37, "priced_sessions": 37, "unpriced_sessions": 0, "usd_micros_known": 125500000, "usd_partial": false, "cache_savings_usd_micros_known": 61300000, "edits_known": 233, "edits_undone_known": 15, "edits_denied_known": 6, "edits_partial": false}, "last_week": {"sessions": 33, "priced_sessions": 33, "unpriced_sessions": 0, "usd_micros_known": 112000000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 198, "edits_undone_known": 17, "edits_denied_known": 4, "edits_partial": false}, "by_tool": [{"key": "claude_code", "sessions": 31, "priced_sessions": 31, "unpriced_sessions": 0, "usd_micros_known": 115700000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 190, "edits_undone_known": 12, "edits_denied_known": 5, "edits_partial": false}, {"key": "codex", "sessions": 6, "priced_sessions": 6, "unpriced_sessions": 0, "usd_micros_known": 9800000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 43, "edits_undone_known": 3, "edits_denied_known": 1, "edits_partial": false}], "by_model": [{"key": "claude-opus-5-5", "tool": "claude_code", "sessions": 14, "tokens": 29000000, "usd_micros_known": 96400000, "usd_partial": false}, {"key": "claude-sonnet-5-5", "tool": "claude_code", "sessions": 13, "tokens": 14000000, "usd_micros_known": 18200000, "usd_partial": false}, {"key": "gpt-5-codex", "tool": "codex", "sessions": 6, "tokens": 6200000, "usd_micros_known": 9800000, "usd_partial": false}, {"key": "claude-haiku-5-5", "tool": "claude_code", "sessions": 4, "tokens": 2100000, "usd_micros_known": 1100000, "usd_partial": false}], "by_folder": [{"key": "trace-commons", "sessions": 7, "priced_sessions": 7, "unpriced_sessions": 0, "usd_micros_known": 38200000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 71, "edits_undone_known": 4, "edits_denied_known": 2, "edits_partial": false}, {"key": "design-system", "sessions": 6, "priced_sessions": 6, "unpriced_sessions": 0, "usd_micros_known": 27400000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 52, "edits_undone_known": 3, "edits_denied_known": 1, "edits_partial": false}, {"key": "scripts", "sessions": 7, "priced_sessions": 7, "unpriced_sessions": 0, "usd_micros_known": 22100000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 44, "edits_undone_known": 3, "edits_denied_known": 1, "edits_partial": false}, {"key": "website", "sessions": 6, "priced_sessions": 6, "unpriced_sessions": 0, "usd_micros_known": 16300000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 31, "edits_undone_known": 2, "edits_denied_known": 1, "edits_partial": false}, {"key": "infra", "sessions": 6, "priced_sessions": 6, "unpriced_sessions": 0, "usd_micros_known": 13900000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 22, "edits_undone_known": 2, "edits_denied_known": 1, "edits_partial": false}, {"key": "notes", "sessions": 5, "priced_sessions": 5, "unpriced_sessions": 0, "usd_micros_known": 7600000, "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 13, "edits_undone_known": 1, "edits_denied_known": 0, "edits_partial": false}], "by_day": [{"date": "2026-10-05", "usd_micros_known": 18400000, "usd_partial": false}, {"date": "2026-10-06", "usd_micros_known": 24900000, "usd_partial": false}, {"date": "2026-10-07", "usd_micros_known": 27300000, "usd_partial": false}, {"date": "2026-10-08", "usd_micros_known": 21600000, "usd_partial": false}, {"date": "2026-10-09", "usd_micros_known": 16200000, "usd_partial": false}, {"date": "2026-10-10", "usd_micros_known": 6800000, "usd_partial": false}, {"date": "2026-10-11", "usd_micros_known": 10300000, "usd_partial": false}], "last_week_by_day": [{"date": "2026-09-28", "usd_micros_known": 16100000, "usd_partial": false}, {"date": "2026-09-29", "usd_micros_known": 21300000, "usd_partial": false}, {"date": "2026-09-30", "usd_micros_known": 24800000, "usd_partial": false}, {"date": "2026-10-01", "usd_micros_known": 19900000, "usd_partial": false}, {"date": "2026-10-02", "usd_micros_known": 17400000, "usd_partial": false}, {"date": "2026-10-03", "usd_micros_known": 5200000, "usd_partial": false}, {"date": "2026-10-04", "usd_micros_known": 7300000, "usd_partial": false}]},
          "goals": {"goals": [{"tool": "claude_code", "status": "met", "met": true, "tokens": 182000, "time_secs": 2460, "iterations": null, "folder": "infra", "session_ref": "sample", "updated_at": "2026-10-11T16:50:00Z"}, {"tool": "codex", "status": "complete", "met": true, "tokens": 640000, "time_secs": 2700, "iterations": null, "folder": "website", "session_ref": "sample", "updated_at": "2026-10-09T18:20:00Z"}, {"tool": "claude_code", "status": "not_met", "met": false, "tokens": 96000, "time_secs": 1380, "iterations": null, "folder": "scripts", "session_ref": "sample", "updated_at": "2026-10-09T15:05:00Z"}], "met": 2, "total": 3, "by_tool": {"claude_code": {"met": 1, "total": 2}, "codex": {"met": 1, "total": 1}}},
          "recap_card_enabled": false,
          "routing_available": false,
          "usage_by_day": [
            {"date": "2026-10-05", "claude_code": 7120000, "codex": 2400000},
            {"date": "2026-10-06", "claude_code": 9750000, "codex": 0},
            {"date": "2026-10-07", "claude_code": 10480000, "codex": 1200000},
            {"date": "2026-10-08", "claude_code": 8340000, "codex": 1000000},
            {"date": "2026-10-09", "claude_code": 6200000, "codex": 1600000},
            {"date": "2026-10-10", "claude_code": 1070000, "codex": 0},
            {"date": "2026-10-11", "claude_code": 1930000, "codex": 0}
          ]
        }
        """;
}

/// <summary>
/// The Insights service a sample launch uses: it refuses every request, so
/// nothing a sample window shows is read from this machine's store, and
/// nothing is written (<c>InsightsSampleModels</c>' <c>refuse</c> service,
/// InsightsSample.swift:219-222). It never reaches <c>tc_insights_call</c>.
/// </summary>
public sealed class SampleInsights : ILocalInsights
{
    public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken) =>
        Task.FromException<JsonElement>(new InvalidOperationException(InsightsSample.Refusal));
}
