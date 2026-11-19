using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// <c>insights_benchmarks</c>, as the Mac decodes it
/// (<c>DaemonData.InsightsBenchmarks</c>, InsightsWorkModels.swift:129-240).
/// </summary>
public sealed record InsightsBenchmarks(
    bool Enabled, bool? Readable, int? Weeks, string? Population, int? Sessions, int? MinSessions,
    IReadOnlyList<BenchmarkCategory>? Categories)
{
    /// <summary>The answer, or null when the reply does not decode (the Mac's <c>.undecodable</c>).</summary>
    public static InsightsBenchmarks? Parse(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("enabled", out JsonElement enabled)
            || enabled.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
        {
            return null;
        }

        List<BenchmarkCategory>? categories = null;
        if (root.TryGetProperty("categories", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            categories = new List<BenchmarkCategory>();
            foreach (JsonElement row in list.EnumerateArray())
            {
                if (BenchmarkCategory.Parse(row) is not { } category) return null;
                categories.Add(category);
            }
        }
        else if (root.TryGetProperty("categories", out JsonElement other) && other.ValueKind != JsonValueKind.Null)
        {
            return null;
        }

        return new InsightsBenchmarks(enabled.GetBoolean(), Bool(root, "readable"), Int(root, "weeks"),
            Text(root, "population"), Int(root, "sessions"), Int(root, "min_sessions"), categories);
    }

    internal static string? Text(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    internal static int? Int(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out int i) ? i : null;

    internal static long? Long(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt64(out long i) ? i : null;

    private static bool? Bool(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : null;
}

public sealed record BenchmarkEntry(string Label, string Tool, long Figure);

/// <summary>The leader and the runner-up: permille for quality, micro-dollars for value, seconds for speed.</summary>
public sealed record BenchmarkPick(BenchmarkEntry Leader, BenchmarkEntry Next);

public sealed record BenchmarkModel(
    string Label, string Tool, int Sessions, int? KeptPermille, long? UsdMicrosMedian, long? ActiveSecsMedian);

/// <summary>One tip; <c>Kind</c> names which, and the other fields are those it carries.</summary>
public sealed record BenchmarkTip(
    string Kind, int? FasterPermille = null, int? SessionsWithout = null, int? Sessions = null,
    string? Cheap = null, string? Expensive = null, int? CostRatioPermille = null,
    string? Better = null, string? Worse = null, int? RatioPermille = null);

public sealed record BenchmarkCategory(
    string Key, int Sessions, IReadOnlyList<BenchmarkModel> Models,
    BenchmarkPick? Quality, BenchmarkPick? Value, BenchmarkPick? Speed, BenchmarkTip? Tip)
{
    internal static BenchmarkCategory? Parse(JsonElement row)
    {
        if (row.ValueKind != JsonValueKind.Object || InsightsBenchmarks.Text(row, "key") is not { } key
            || InsightsBenchmarks.Int(row, "sessions") is not { } sessions)
        {
            return null;
        }

        var models = new List<BenchmarkModel>();
        if (row.TryGetProperty("models", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement m in list.EnumerateArray())
            {
                if (InsightsBenchmarks.Text(m, "label") is not { } label || InsightsBenchmarks.Text(m, "tool") is not { } tool
                    || InsightsBenchmarks.Int(m, "sessions") is not { } n)
                {
                    return null;
                }

                models.Add(new BenchmarkModel(label, tool, n, InsightsBenchmarks.Int(m, "kept_permille"),
                    InsightsBenchmarks.Long(m, "usd_micros_median"), InsightsBenchmarks.Long(m, "active_secs_median")));
            }
        }

        BenchmarkTip? tip = null;
        if (row.TryGetProperty("tip", out JsonElement t) && t.ValueKind == JsonValueKind.Object)
        {
            if (InsightsBenchmarks.Text(t, "kind") is not { } kind) return null;
            tip = new BenchmarkTip(kind, InsightsBenchmarks.Int(t, "faster_permille"), InsightsBenchmarks.Int(t, "sessions_without"),
                InsightsBenchmarks.Int(t, "sessions"), InsightsBenchmarks.Text(t, "cheap"), InsightsBenchmarks.Text(t, "expensive"),
                InsightsBenchmarks.Int(t, "cost_ratio_permille"), InsightsBenchmarks.Text(t, "better"),
                InsightsBenchmarks.Text(t, "worse"), InsightsBenchmarks.Int(t, "ratio_permille"));
        }

        return new BenchmarkCategory(key, sessions, models, Pick(row, "quality"), Pick(row, "value"), Pick(row, "speed"), tip);
    }

    private static BenchmarkPick? Pick(JsonElement row, string key)
    {
        if (!row.TryGetProperty(key, out JsonElement p) || p.ValueKind != JsonValueKind.Object) return null;
        BenchmarkEntry? Entry(string side) =>
            p.TryGetProperty(side, out JsonElement e) && e.ValueKind == JsonValueKind.Object
            && InsightsBenchmarks.Text(e, "label") is { } label && InsightsBenchmarks.Text(e, "tool") is { } tool
            && InsightsBenchmarks.Long(e, "figure") is { } figure
                ? new BenchmarkEntry(label, tool, figure) : null;
        return Entry("leader") is { } leader && Entry("next") is { } next ? new BenchmarkPick(leader, next) : null;
    }
}

/// <summary>A picker option: its label in the core's words and the value it stands for.</summary>
public sealed record BenchmarkChoice(string Label, string Population, int Weeks);

/// <summary>A metric row on a card: its label and info, the leading model, its figure and the runner-up line.</summary>
public sealed record BenchmarkPickRow(string Id, string Title, string Info, string Model, string Figure, string Next);

/// <summary>One model's own figures on a kind of work (the "Yours" grid).</summary>
public sealed record BenchmarkModelRow(string Id, string Model, string Traces, string Kept, string Cost, string Time);

/// <summary>One share of a model's traces in the work-mix card.</summary>
public sealed record BenchmarkShare(string Id, string Task, double Fraction, string Label);

public sealed record BenchmarkMix(string Id, string Model, string Traces, IReadOnlyList<BenchmarkShare> Shares);

/// <summary>A kind-of-work card as drawn (<c>InsightsBenchmarksTab.card</c>).</summary>
public sealed record BenchmarkCard(
    string Key, string Name, string Traces, IReadOnlyList<BenchmarkPickRow> Picks, IReadOnlyList<BenchmarkModelRow> Rows,
    string? CompareLater, string? Tip, string YoursTitle = "", string KeptTitle = "", string CostTitle = "", string TimeTitle = "")
{
    public bool HasPicks => Picks.Count > 0;

    public bool HasRows => Rows.Count > 0;

    public bool HasCompareLater => CompareLater is not null;

    public bool HasTip => Tip is not null;
}

/// <summary>
/// The Benchmarks figures and words (<c>InsightsBenchmarkWords</c>,
/// InsightsBenchmarksTab.swift:66-243), every word from the core's Insights
/// table (<c>tc_insights_copy_json</c>).
/// </summary>
public static class InsightsBenchmarkWords
{
    /// <summary>The order the tab shows the kinds of work in (InsightsBenchmarksTab.swift:144).</summary>
    public static readonly IReadOnlyList<string> Categories =
        new[] { "bug_fix", "feature", "refactor", "tests", "docs", "understand", "review", "setup" };

    /// <summary>The populations the picker offers, in its order.</summary>
    public static readonly IReadOnlyList<string> Populations = new[] { "own", "spawned" };

    /// <summary>The periods the picker offers, in weeks.</summary>
    public static readonly IReadOnlyList<int> PeriodWeeks = new[] { 4, 13 };

    private static readonly CultureInfo Us = CultureInfo.GetCultureInfo("en-US");

    public static string Fill(string template, IReadOnlyDictionary<string, string> values) =>
        values.Aggregate(template, (result, hole) => result.Replace("{" + hole.Key + "}", hole.Value, StringComparison.Ordinal));

    private static string T(string key, Func<string, string> copy) => copy(key);

    public static string Name(string key, Func<string, string> copy) => T("analytics_bench_cat_" + key, copy);

    public static string Traces(int count, Func<string, string> copy) =>
        Fill(T("analytics_bench_traces", copy), new Dictionary<string, string> { ["n"] = count.ToString(CultureInfo.InvariantCulture) });

    /// <summary>Swift's <c>Double.rounded()</c>: half away from zero.</summary>
    private static long Round(double value) => (long)Math.Round(value, MidpointRounding.AwayFromZero);

    private static string Percent(long permille) => Round(permille / 10.0).ToString(CultureInfo.InvariantCulture);

    /// <summary>
    /// "14m", "1h 5m": <c>DateComponentsFormatter</c>, abbreviated, hours
    /// and minutes, at most two units, zeros dropped, never under a minute.
    /// </summary>
    public static string Duration(long seconds)
    {
        long total = Math.Max(60, seconds);
        long hours = total / 3600;
        long minutes = (total % 3600) / 60;
        if (hours == 0) return minutes.ToString(CultureInfo.InvariantCulture) + "m";
        return minutes == 0
            ? hours.ToString(CultureInfo.InvariantCulture) + "h"
            : hours.ToString(CultureInfo.InvariantCulture) + "h " + minutes.ToString(CultureInfo.InvariantCulture) + "m";
    }

    /// <summary>
    /// Dollars as Insights draws them (<c>InsightsWorkWords.dollars</c>,
    /// InsightsWorkViews.swift:55-59): two decimals from a cent up; under a
    /// cent the app's money form (two to four decimals; under a hundredth
    /// of a cent one significant digit, so a charge never reads zero).
    /// </summary>
    public static string Dollars(long micros)
    {
        if (micros >= 10_000)
        {
            return (micros / 1_000_000m).ToString("C2", Us);
        }

        long m = Math.Max(0, micros);
        if (m > 0 && m < 100)
        {
            decimal value = m / 1_000_000m;
            int digits = -(int)Math.Floor(Math.Log10((double)value));
            return Math.Round(value, digits, MidpointRounding.ToEven).ToString("$0." + new string('0', digits), Us);
        }

        return InferenceLedger.Money(m);
    }

    /// <summary>A money figure, or the core's unknown word for none.</summary>
    public static string Amount(long? micros, Func<string, string> copy) =>
        micros is { } m && m >= 0 ? Dollars(m) : T("analytics_unavailable", copy);

    public static string Figure(string kind, long value, Func<string, string> copy) => kind switch
    {
        "quality" => Fill(T("analytics_bench_kept", copy), new Dictionary<string, string> { ["p"] = Percent(value) }),
        "value" => Fill(T("analytics_bench_per_trace", copy), new Dictionary<string, string> { ["x"] = Amount(value, copy) }),
        _ => Duration(value),
    };

    /// <summary>The traces compared: those on a kind of work, not the window's total.</summary>
    public static int Compared(InsightsBenchmarks answer) => (answer.Categories ?? Array.Empty<BenchmarkCategory>()).Sum(c => c.Sessions);

    public static string Population(string key, Func<string, string> copy) => T("analytics_bench_population_" + key, copy);

    public static string Period(int weeks, Func<string, string> copy) =>
        Fill(T("analytics_bench_weeks", copy), new Dictionary<string, string> { ["n"] = weeks.ToString(CultureInfo.InvariantCulture) });

    /// <summary>The heading over the cards: how many traces were compared, and whose.</summary>
    public static string Intro(int count, string population, Func<string, string> copy) =>
        Fill(T(population == "spawned" ? "analytics_bench_intro_spawned" : "analytics_bench_intro", copy),
            new Dictionary<string, string> { ["n"] = count.ToString(CultureInfo.InvariantCulture) });

    /// <summary>The categories in the tab's order, whatever order they came in; the first of a repeated key wins.</summary>
    public static IReadOnlyList<BenchmarkCategory> Ordered(InsightsBenchmarks answer)
    {
        var byKey = new Dictionary<string, BenchmarkCategory>(StringComparer.Ordinal);
        foreach (BenchmarkCategory category in answer.Categories ?? Array.Empty<BenchmarkCategory>())
        {
            byKey.TryAdd(category.Key, category);
        }

        return Categories.Where(byKey.ContainsKey).Select(k => byKey[k]).ToList();
    }

    /// <summary>Quality, value and speed, each present only when two models qualify.</summary>
    public static IReadOnlyList<BenchmarkPickRow> Picks(BenchmarkCategory category, Func<string, string> copy) =>
        new[] { ("quality", category.Quality), ("value", category.Value), ("speed", category.Speed) }
            .Where(p => p.Item2 is not null)
            .Select(p =>
            {
                (string kind, BenchmarkPick? pick) = p;
                string next = Fill(T("analytics_bench_next", copy), new Dictionary<string, string>
                {
                    ["label"] = pick!.Next.Label, ["x"] = Figure(kind, pick.Next.Figure, copy),
                });
                return new BenchmarkPickRow(kind, T("analytics_bench_" + kind, copy), T("analytics_bench_" + kind + "_detail", copy),
                    pick.Leader.Label, Figure(kind, pick.Leader.Figure, copy), next);
            })
            .ToList();

    /// <summary>The card's one observation in the core's words; null for a kind this build has no words for.</summary>
    public static string? Tip(BenchmarkTip? tip, Func<string, string> copy)
    {
        if (tip is null) return null;
        string P(int? permille) => Percent(permille ?? 0);
        string N(int? n) => (n ?? 0).ToString(CultureInfo.InvariantCulture);
        return tip.Kind switch
        {
            "test_first" => Fill(T("analytics_bench_tip_test_first", copy), new Dictionary<string, string>
            {
                ["p"] = P(tip.FasterPermille), ["k"] = N(tip.SessionsWithout), ["n"] = N(tip.Sessions),
            }),
            "cheaper_same_quality" => Fill(T("analytics_bench_tip_cheaper", copy), new Dictionary<string, string>
            {
                ["first"] = tip.Cheap ?? string.Empty, ["second"] = tip.Expensive ?? string.Empty, ["p"] = P(tip.CostRatioPermille),
            }),
            "fewer_undos" => Fill(T("analytics_bench_tip_fewer_undos", copy), new Dictionary<string, string>
            {
                ["first"] = tip.Better ?? string.Empty, ["second"] = tip.Worse ?? string.Empty,
                // fractionLength(0...1): at most one decimal, none when whole.
                ["x"] = Math.Round((tip.RatioPermille ?? 0) / 1000.0, 1, MidpointRounding.ToEven).ToString("0.#", Us),
            }),
            _ => null,
        };
    }

    /// <summary>Every model on a kind of work with its own figures, most traces first, then by name.</summary>
    public static IReadOnlyList<BenchmarkModelRow> Rows(BenchmarkCategory category, Func<string, string> copy)
    {
        string dash = T("analytics_unavailable", copy);
        return category.Models
            .OrderByDescending(m => m.Sessions).ThenBy(m => m.Label, StringComparer.Ordinal)
            .Select(m => new BenchmarkModelRow(m.Label + "/" + m.Tool, m.Label, Traces(m.Sessions, copy),
                m.KeptPermille is { } kept ? Percent(kept) + "%" : dash,
                m.UsdMicrosMedian is { } cost ? Amount(cost, copy) : dash,
                m.ActiveSecsMedian is { } secs ? Duration(secs) : dash))
            .ToList();
    }

    /// <summary>Each model's work mix: every kind of work it did with its share, most first.</summary>
    public static IReadOnlyList<BenchmarkMix> Mix(InsightsBenchmarks answer, Func<string, string> copy)
    {
        var byModel = new Dictionary<string, (string Label, List<(string Key, int Sessions)> Tasks)>(StringComparer.Ordinal);
        foreach (BenchmarkCategory category in Ordered(answer))
        {
            foreach (BenchmarkModel model in category.Models.Where(m => m.Sessions > 0))
            {
                string id = model.Label + "/" + model.Tool;
                if (!byModel.TryGetValue(id, out var entry)) byModel[id] = entry = (model.Label, new List<(string, int)>());
                entry.Tasks.Add((category.Key, model.Sessions));
            }
        }

        return byModel
            .OrderByDescending(e => e.Value.Tasks.Sum(t => t.Sessions)).ThenBy(e => e.Value.Label, StringComparer.Ordinal)
            .Select(e =>
            {
                int total = e.Value.Tasks.Sum(t => t.Sessions);
                var shares = e.Value.Tasks
                    .OrderByDescending(t => t.Sessions).ThenBy(t => t.Key, StringComparer.Ordinal)
                    .Select(t =>
                    {
                        double fraction = (double)t.Sessions / Math.Max(total, 1);
                        return new BenchmarkShare(t.Key, Name(t.Key, copy), fraction,
                            Fill(T("analytics_bench_mix_share", copy), new Dictionary<string, string>
                            {
                                ["p"] = Round(fraction * 100).ToString(CultureInfo.InvariantCulture), ["n"] = Traces(t.Sessions, copy),
                            }));
                    })
                    .ToList();
                return new BenchmarkMix(e.Key, e.Value.Label, Traces(total, copy), shares);
            })
            .ToList();
    }

    /// <summary>The cards: a kind of work with no trace has none.</summary>
    public static IReadOnlyList<BenchmarkCard> Cards(InsightsBenchmarks answer, Func<string, string> copy)
    {
        int minimum = answer.MinSessions ?? 5;
        return Ordered(answer).Where(c => c.Sessions > 0).Select(c =>
        {
            IReadOnlyList<BenchmarkPickRow> picks = Picks(c, copy);
            return new BenchmarkCard(c.Key, Name(c.Key, copy), Traces(c.Sessions, copy), picks, Rows(c, copy),
                picks.Count == 0
                    ? Fill(T("analytics_bench_compare_later", copy), new Dictionary<string, string> { ["n"] = minimum.ToString(CultureInfo.InvariantCulture) })
                    : null,
                Tip(c.Tip, copy), T("analytics_bench_yours", copy), T("analytics_bench_col_kept", copy),
                T("analytics_bench_col_cost", copy), T("analytics_bench_col_time", copy));
        }).ToList();
    }
}

/// <summary>
/// The Benchmarks tab's state (<c>InsightsBenchmarksModel</c>,
/// InsightsBenchmarksTab.swift:11-62): a reader for <c>insights_benchmarks</c>
/// with a period (4 or 13 weeks) and a population (own or spawned), or a fixed
/// sample with no reader. The latest request wins.
/// </summary>
public sealed class InsightsBenchmarksModel : INotifyPropertyChanged
{
    public const string Method = "insights_benchmarks";

    private readonly Func<string, string, Task<DaemonResponse>>? _call;
    private readonly Func<string, string> _copy;
    private long _generation;

    public InsightsBenchmarksModel(Func<string, string, Task<DaemonResponse>>? call, Func<string, string> copy)
    {
        _call = call;
        _copy = copy;
    }

    /// <summary>A fixed answer and no reader: the sample under the get-started card.</summary>
    public InsightsBenchmarksModel(InsightsBenchmarks sample, Func<string, string> copy)
    {
        _copy = copy;
        Answer = sample;
        Settled = true;
        IsSample = true;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public InsightsBenchmarks? Answer { get; private set; }

    public bool Failed { get; private set; }

    public bool Busy { get; private set; }

    public bool Settled { get; private set; }

    public bool IsSample { get; }

    /// <summary>Whether this daemon can answer at all.</summary>
    public bool Available => _call is not null;

    public int Weeks { get; private set; } = 4;

    public string Population { get; private set; } = "own";

    /// <summary>The population picker's options, in its order, in the core's words.</summary>
    public IReadOnlyList<BenchmarkChoice> PopulationChoices =>
        InsightsBenchmarkWords.Populations.Select(p => new BenchmarkChoice(InsightsBenchmarkWords.Population(p, _copy), p, 0)).ToList();

    /// <summary>The period picker's options: 4 and 13 weeks.</summary>
    public IReadOnlyList<BenchmarkChoice> PeriodChoices =>
        InsightsBenchmarkWords.PeriodWeeks.Select(w => new BenchmarkChoice(InsightsBenchmarkWords.Period(w, _copy), string.Empty, w)).ToList();

    public int PopulationIndex => Math.Max(0, InsightsBenchmarkWords.Populations.ToList().IndexOf(Population));

    public int PeriodIndex => Math.Max(0, InsightsBenchmarkWords.PeriodWeeks.ToList().IndexOf(Weeks));

    public string PopulationName => _copy("analytics_bench_population");

    public string PeriodName => _copy("analytics_tab_benchmarks");

    public string MixTitle => _copy("analytics_bench_mix_title");

    public string YoursTitle => _copy("analytics_bench_yours");

    public IReadOnlyList<string> ColumnTitles => new[] { "kept", "cost", "time" }.Select(c => _copy("analytics_bench_col_" + c)).ToList();

    /// <summary>The request's parameters: an older daemon that knows no population is asked as before.</summary>
    public static string Parameters(int weeks, string population) =>
        population == "own"
            ? "{\"weeks\":" + weeks.ToString(CultureInfo.InvariantCulture) + "}"
            : "{\"weeks\":" + weeks.ToString(CultureInfo.InvariantCulture) + ",\"population\":" + JsonSerializer.Serialize(population) + "}";

    public Task SelectAsync(int weeks, string population)
    {
        if (weeks == Weeks && population == Population && Settled) return Task.CompletedTask;
        Weeks = weeks;
        Population = population;
        return LoadAsync();
    }

    public async Task LoadAsync()
    {
        if (_call is null)
        {
            Settled = true;
            Publish();
            return;
        }

        long mine = ++_generation;
        Busy = true;
        Publish();
        InsightsBenchmarks? reply = null;
        try
        {
            DaemonResponse response = await _call(Method, Parameters(Weeks, Population)).ConfigureAwait(true);
            if (response is { IsError: false, Result: { } result }) reply = InsightsBenchmarks.Parse(result);
        }
        catch (Exception)
        {
            reply = null;
        }

        if (mine != _generation) return;
        Answer = reply;
        Failed = reply is null;
        Settled = true;
        Busy = false;
        Publish();
    }

    /// <summary>Whether the cards draw: an enabled answer the daemon could read.</summary>
    private bool Readable => Answer is { Enabled: true } a && a.Readable != false;

    public IReadOnlyList<BenchmarkCard> Cards => Readable ? InsightsBenchmarkWords.Cards(Answer!, _copy) : Array.Empty<BenchmarkCard>();

    public IReadOnlyList<BenchmarkMix> Mixes => Readable && Cards.Count > 0 ? InsightsBenchmarkWords.Mix(Answer!, _copy) : Array.Empty<BenchmarkMix>();

    public bool HasMixes => Mixes.Count > 0;

    /// <summary>The intro heading, over the person's own benchmarks only (never over the sample).</summary>
    public string Intro => !IsSample && Answer is { } a
        ? InsightsBenchmarkWords.Intro(InsightsBenchmarkWords.Compared(a), a.Population ?? "own", _copy)
        : string.Empty;

    public bool HasIntro => Intro.Length > 0;

    /// <summary>
    /// The note in place of cards: not enough traces for any card, or the
    /// core's unknown word when the read failed, did not decode, or there is
    /// no daemon to ask (InsightsBenchmarksTab.swift:257-269).
    /// </summary>
    public string Note => Readable && Cards.Count == 0 ? _copy("analytics_bench_not_enough")
        : !Readable && (Failed || Answer is not null || !Available) ? _copy("analytics_unavailable")
        : string.Empty;

    public bool HasNote => Note.Length > 0;

    private void Publish() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
}
