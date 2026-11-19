using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// IronWire's proof labels, as <c>inference_calls</c> passes them through
/// (<c>DaemonData.ProofLabel</c>, DaemonDataModels.swift:1774-1780), in the
/// Mac's case order. Only <see cref="Verified"/> is proof; <see cref="Failed"/>
/// is kept apart from <see cref="Outside"/>.
/// </summary>
public enum ProofLabel
{
    Verified,
    GatewayOnly,
    Unattested,
    Pending,
    Unavailable,
    Failed,
    Outside,
    Unrecorded,
}

/// <summary>A tag's tone (<c>GlassTag.Tone</c>), resolved to a brush by the view.</summary>
public enum LedgerTone
{
    On,
    Ask,
    Failed,
    Outside,
    Neutral,
}

/// <summary>Priced, not billed. <c>Known == false</c> is not zero.</summary>
public sealed record PricedCost(bool Known, long? PricedMicros);

/// <summary>One call's counters as the proxy reported them; null is unknown, never zero.</summary>
public sealed record InferenceCallTokens(uint? Input, uint? CacheRead, uint? CacheWrite, uint? Output);

/// <summary>One recorded model call (<c>DaemonData.InferenceCall</c>).</summary>
public sealed record InferenceCall(
    long Id,
    DateTimeOffset At,
    string Tool,
    string Family,
    string Model,
    string Route,
    PricedCost? Cost,
    string Proof,
    InferenceCallTokens? Tokens)
{
    /// <summary>A label this shell does not know reads as unrecorded (DaemonDataModels.swift:1720).</summary>
    public ProofLabel ProofLabel => InferenceLedger.ParseProof(Proof) ?? ProofLabel.Unrecorded;
}

/// <summary><c>inference_calls</c>: <c>readable == false</c> is no ledger, never an empty one.</summary>
public sealed record InferenceCallPage(bool Readable, int? WindowHours, IReadOnlyList<InferenceCall> Calls, string? NextCursor);

/// <summary>One model of the provisional per-model summary (<c>DaemonData.ModelSummary</c>).</summary>
public sealed record ModelSummary(string Model, string? Family, long? Calls, long? PricedMicros, IReadOnlyDictionary<string, long>? ProofCounts);

/// <summary>The provisional per-model summary (<c>DaemonData.InferenceSummary</c>).</summary>
public sealed record InferenceModelSummary(bool Readable, int? WindowHours, IReadOnlyList<ModelSummary> Models);

/// <summary>
/// What the tab's call count reads from <c>tool_destinations</c> (K14): each
/// tool's <c>counts.inference_calls</c> and the unattributed calls, over its window.
/// </summary>
public sealed record DestinationCallCounts(int? WindowHours, IReadOnlyList<long?> ToolCalls, long? UnattributedCalls);

/// <summary>One route's totals in IronWire's summary (<c>DaemonData.InferenceRouteTotal</c>).</summary>
public sealed record InferenceRouteTotal(ulong Calls, ulong? PricedCalls, double? CostUsd, ulong Verified);

/// <summary>
/// <c>inference_summary</c> (#1187): IronWire's grouped summary in a
/// <c>readable</c> wrapper. Only what the tab's header reads is kept.
/// </summary>
public sealed record NetworkInferenceSummary(bool Readable, int? WindowHours, bool? Enabled,
    InferenceRouteTotal? Routed, InferenceRouteTotal? Outside, InferenceRouteTotal? Unknown);

/// <summary>The tab's three header figures; null and the dash are unknown.</summary>
public sealed record LedgerTotals(long? Calls, long? Verified, string Priced);

/// <summary>
/// The Private AI tab's ledger (spec W19, <c>InferenceTabView</c>,
/// macos/Sources/TraceCommonsApp/Views/Monitor/InferenceViews.swift:118-497),
/// ported as pure decisions so they run beside the Mac's probe.
/// </summary>
public static class InferenceLedger
{
    /// <summary>The dash an unknown figure draws.</summary>
    public const string Dash = "—";

    /// <summary>InferenceStore.pageSize: the calls one page reads.</summary>
    public const int PageSize = 50;

    private static readonly string[] ProofWire =
    {
        "verified", "gateway_only", "unattested", "pending", "unavailable", "failed", "outside", "unrecorded",
    };

    public static ProofLabel? ParseProof(string? raw)
    {
        int index = Array.IndexOf(ProofWire, raw);
        return index < 0 ? null : (ProofLabel)index;
    }

    public static string Wire(ProofLabel label) => ProofWire[(int)label];

    // ---- Decoding ----------------------------------------------------------

    public static InferenceCallPage? ParsePage(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("readable", out JsonElement readable)
            || readable.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
        {
            return null;
        }

        var calls = new List<InferenceCall>();
        if (root.TryGetProperty("calls", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement call in list.EnumerateArray())
            {
                if (ParseCall(call) is { } parsed) calls.Add(parsed);
            }
        }

        return new InferenceCallPage(readable.GetBoolean(), Int(root, "window_hours"), calls, Str(root, "next_cursor"));
    }

    private static InferenceCall? ParseCall(JsonElement call)
    {
        if (call.ValueKind != JsonValueKind.Object
            || !call.TryGetProperty("id", out JsonElement id) || !id.TryGetInt64(out long callId)
            || Str(call, "at") is not { } at
            || !DateTimeOffset.TryParse(at, CultureInfo.InvariantCulture, DateTimeStyles.None, out DateTimeOffset when))
        {
            return null;
        }

        PricedCost? cost = null;
        if (call.TryGetProperty("cost", out JsonElement c) && c.ValueKind == JsonValueKind.Object
            && c.TryGetProperty("known", out JsonElement known) && known.ValueKind is JsonValueKind.True or JsonValueKind.False)
        {
            cost = new PricedCost(known.GetBoolean(), Long(c, "priced_micros"));
        }

        InferenceCallTokens? tokens = null;
        if (call.TryGetProperty("tokens", out JsonElement t) && t.ValueKind == JsonValueKind.Object)
        {
            tokens = new InferenceCallTokens(UInt(t, "input"), UInt(t, "cache_read"), UInt(t, "cache_write"), UInt(t, "output"));
        }

        return new InferenceCall(
            callId, when, Str(call, "tool") ?? string.Empty, Str(call, "family") ?? string.Empty,
            Str(call, "model") ?? string.Empty, Str(call, "route") ?? string.Empty, cost,
            Str(call, "proof") ?? string.Empty, tokens);
    }

    public static InferenceModelSummary? ParseSummary(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("readable", out JsonElement readable)
            || readable.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
        {
            return null;
        }

        var models = new List<ModelSummary>();
        if (root.TryGetProperty("models", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement m in list.EnumerateArray())
            {
                if (Str(m, "model") is not { } name) continue;
                Dictionary<string, long>? counts = null;
                if (m.TryGetProperty("proof_counts", out JsonElement pc) && pc.ValueKind == JsonValueKind.Object)
                {
                    counts = new Dictionary<string, long>(StringComparer.Ordinal);
                    foreach (JsonProperty p in pc.EnumerateObject())
                    {
                        if (p.Value.TryGetInt64(out long n)) counts[p.Name] = n;
                    }
                }

                models.Add(new ModelSummary(name, Str(m, "family"), Long(m, "calls"), Long(m, "priced_micros"), counts));
            }
        }

        return new InferenceModelSummary(readable.GetBoolean(), Int(root, "window_hours"), models);
    }

    /// <summary>
    /// Decodes <c>inference_summary</c>. A reply the Mac's decoder would
    /// refuse is null, which the tab treats as no network summary.
    /// </summary>
    public static NetworkInferenceSummary? ParseNetworkSummary(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("readable", out JsonElement readable)
            || readable.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
        {
            return null;
        }

        if (!root.TryGetProperty("summary", out JsonElement summary) || summary.ValueKind == JsonValueKind.Null)
        {
            return new NetworkInferenceSummary(readable.GetBoolean(), Int(root, "window_hours"), null, null, null, null);
        }

        if (summary.ValueKind != JsonValueKind.Object
            || !summary.TryGetProperty("enabled", out JsonElement enabled) || enabled.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || Route(summary, "routed") is not { } routed || Route(summary, "outside") is not { } outside
            || Route(summary, "unknown") is not { } unknown)
        {
            return null;
        }

        return new NetworkInferenceSummary(readable.GetBoolean(), Int(root, "window_hours"), enabled.GetBoolean(), routed, outside, unknown);
    }

    private static InferenceRouteTotal? Route(JsonElement summary, string key)
    {
        if (!summary.TryGetProperty(key, out JsonElement route) || route.ValueKind != JsonValueKind.Object
            || !route.TryGetProperty("calls", out JsonElement calls) || !calls.TryGetUInt64(out ulong n)
            || !route.TryGetProperty("proof", out JsonElement proof) || proof.ValueKind != JsonValueKind.Object
            || !proof.TryGetProperty("verified", out JsonElement verified) || !verified.TryGetUInt64(out ulong v))
        {
            return null;
        }

        ulong? priced = route.TryGetProperty("priced_calls", out JsonElement pc) && pc.TryGetUInt64(out ulong p) ? p : null;
        double? cost = route.TryGetProperty("cost_usd", out JsonElement c) && c.ValueKind == JsonValueKind.Number ? c.GetDouble() : null;
        return new InferenceRouteTotal(n, priced, cost, v);
    }

    /// <summary>
    /// The header over the network summary (InferenceViews.swift:162-180,
    /// HomeViews.swift:775-791): calls and verified over every route, exact or
    /// unknown; priced only when every route priced every call.
    /// </summary>
    public static LedgerTotals NetworkHeaderTotals(NetworkInferenceSummary? network)
    {
        if (network is not { Readable: true, Enabled: true, Routed: { } routed, Outside: { } outside, Unknown: { } unknown })
        {
            return new LedgerTotals(null, null, Dash);
        }

        InferenceRouteTotal[] routes = { routed, outside, unknown };
        long? calls = ExactTotal(routes.Select(r => r.Calls));
        long? verified = ExactTotal(routes.Select(r => r.Verified));
        if (calls is null) return new LedgerTotals(calls, verified, Dash);
        double usd = 0;
        foreach (InferenceRouteTotal route in routes)
        {
            if (route.PricedCalls is not { } pricedCalls || pricedCalls != route.Calls
                || route.CostUsd is not { } cost || !double.IsFinite(cost) || cost < 0)
            {
                return new LedgerTotals(calls, verified, Dash);
            }

            usd += cost;
            if (!double.IsFinite(usd)) return new LedgerTotals(calls, verified, Dash);
        }

        return new LedgerTotals(calls, verified, Usd(usd));
    }

    private static long? ExactTotal(IEnumerable<ulong> parts)
    {
        ulong total = 0;
        foreach (ulong part in parts)
        {
            if (ulong.MaxValue - total < part) return null;
            total += part;
        }

        return total <= long.MaxValue ? (long)total : null;
    }

    /// <summary>
    /// The tab's own header without a network summary (InferenceViews.swift:129-135):
    /// it draws only priced from <see cref="Totals"/>; calls and verified are dashes.
    /// </summary>
    public static LedgerTotals PageHeaderTotals(InferenceCallPage page, InferenceModelSummary? summary, DestinationCallCounts? destinations) =>
        new(null, null, Totals(page, summary, destinations).Priced);

    public static DestinationCallCounts? ParseDestinations(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object) return null;
        var tools = new List<long?>();
        if (root.TryGetProperty("tools", out JsonElement list) && list.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement tool in list.EnumerateArray())
            {
                tools.Add(tool.ValueKind == JsonValueKind.Object && tool.TryGetProperty("counts", out JsonElement counts)
                    && counts.ValueKind == JsonValueKind.Object ? Long(counts, "inference_calls") : null);
            }
        }

        return new DestinationCallCounts(Int(root, "window_hours"), tools, Long(root, "unattributed_calls"));
    }

    // ---- Totals (InferenceViews.swift:184-234) ------------------------------

    /// <summary>The hours the counts cover: the page's, else the destinations'.</summary>
    public static int? WindowHours(InferenceCallPage page, DestinationCallCounts? destinations) =>
        page.WindowHours ?? destinations?.WindowHours;

    /// <summary>The core's "Last {hours} hours", or the dash when no window was reported.</summary>
    public static string WindowLine(int? hours, string windowLastHours) =>
        hours is { } h ? windowLastHours.Replace("{hours}", h.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal) : Dash;

    /// <summary>
    /// The tab's totals: the call count from <c>tool_destinations</c> when it
    /// answered; verified and priced from the per-model summary when it
    /// answered, else from the page, but only when the page is the whole window.
    /// </summary>
    public static LedgerTotals Totals(InferenceCallPage page, InferenceModelSummary? summary, DestinationCallCounts? destinations)
    {
        LedgerTotals fallback = PageTotals(page, summary);
        return destinations is null ? fallback : fallback with { Calls = CallCount(destinations) };
    }

    /// <summary>Every tool's calls plus the unattributed ones; unknown when any part is.</summary>
    public static long? CallCount(DestinationCallCounts destinations)
    {
        long total = 0;
        foreach (long? part in destinations.ToolCalls.Append(destinations.UnattributedCalls))
        {
            if (part is not { } n) return null;
            total += n;
        }

        return total;
    }

    private static LedgerTotals PageTotals(InferenceCallPage page, InferenceModelSummary? summary)
    {
        if (summary is { Readable: true })
        {
            long? calls = summary.Models.Any(m => m.Calls is null) ? null : summary.Models.Sum(m => m.Calls!.Value);
            long? verified = summary.Models.Any(m => m.ProofCounts is null) ? null
                : summary.Models.Sum(m => m.ProofCounts!.TryGetValue("verified", out long n) ? n : 0);
            string priced = summary.Models.Any(m => m.PricedMicros is null) ? Dash : Money(summary.Models.Sum(m => m.PricedMicros!.Value));
            return new LedgerTotals(calls, verified, priced);
        }

        if (page.NextCursor is not null) return new LedgerTotals(null, null, Dash);
        return new LedgerTotals(page.Calls.Count, page.Calls.Count(c => c.ProofLabel == ProofLabel.Verified), Priced(page.Calls.Select(c => c.Cost)));
    }

    /// <summary>The sum of what the calls were priced at; the dash when any one is unknown.</summary>
    public static string Priced(IEnumerable<PricedCost?> costs)
    {
        long total = 0;
        foreach (PricedCost? cost in costs)
        {
            if (cost is not { Known: true, PricedMicros: { } micros }) return Dash;
            total += micros;
        }

        return Money(total);
    }

    /// <summary>
    /// USD with two to four decimals, half to even, as
    /// <c>.currency(code: "USD").precision(.fractionLength(2 ... 4))</c> draws it.
    /// </summary>
    public static string Money(long micros) => Usd(micros / 1_000_000m);

    private static string Usd(double usd) => Math.Abs(usd) < 1e20 ? Usd((decimal)usd) : Dash;

    private static string Usd(decimal amount)
    {
        decimal usd = Math.Round(amount, 4, MidpointRounding.ToEven);
        string text = Math.Abs(usd).ToString("#,0.00##", CultureInfo.GetCultureInfo("en-US"));
        return (usd < 0 ? "-$" : "$") + text;
    }

    // ---- Words ---------------------------------------------------------------

    /// <summary>
    /// The tool's name when the shell knows the adapter, the raw id when it
    /// does not, and the dash for a call that could not be attributed.
    /// </summary>
    public static string ToolName(string id, Func<string, string?> knownName) =>
        knownName(id) ?? (id == "unknown" ? Dash : id);

    /// <summary>One proof word from the core; an unknown label is the dash, never a verdict.</summary>
    public static string ProofWord(string raw, Func<string, string> monitorWord) =>
        ParseProof(raw) is { } label ? monitorWord("proof_" + Wire(label)) : Dash;

    public static LedgerTone Tone(ProofLabel label) => label switch
    {
        ProofLabel.Verified => LedgerTone.On,
        ProofLabel.Pending or ProofLabel.GatewayOnly => LedgerTone.Ask,
        ProofLabel.Failed => LedgerTone.Failed,
        ProofLabel.Outside => LedgerTone.Outside,
        _ => LedgerTone.Neutral,
    };

    /// <summary>A label this shell does not know is neutral.</summary>
    public static LedgerTone Tone(string raw) => ParseProof(raw) is { } label ? Tone(label) : LedgerTone.Neutral;

    /// <summary>Proof counts in the labels' fixed order, so the tags do not shuffle on reload.</summary>
    public static IReadOnlyList<(string Label, long Count)> SortedProofs(IReadOnlyDictionary<string, long>? counts) =>
        (counts ?? new Dictionary<string, long>())
            .OrderBy(p => ParseProof(p.Key) is { } l ? (int)l : int.MaxValue)
            .ThenBy(p => p.Key, StringComparer.Ordinal)
            .Select(p => (p.Key, p.Value))
            .ToList();

    private static readonly string[] TokenLabels =
    {
        "metric_input_tokens", "analytics_series_cache_read", "analytics_series_cache_write", "metric_output_tokens",
    };

    /// <summary>The ledger's own counters, each as reported, never added (InferenceViews.swift:499-527).</summary>
    public static string TokenLine(InferenceCallTokens tokens, Func<string, string> insightsWord) =>
        TokenPairs(tokens, insightsWord, () => insightsWord("analytics_unavailable"));

    /// <summary>The same pairs read out, with unknown where the line draws the dash.</summary>
    public static string TokenAccessibilityLine(InferenceCallTokens tokens, Func<string, string> insightsWord, string unknown) =>
        TokenPairs(tokens, insightsWord, () => unknown);

    private static string TokenPairs(InferenceCallTokens tokens, Func<string, string> word, Func<string> unknown)
    {
        uint?[] values = { tokens.Input, tokens.CacheRead, tokens.CacheWrite, tokens.Output };
        return string.Join(" · ", TokenLabels.Select((key, i) =>
            word(key) + " " + (values[i] is { } v ? CompactFigure(v) : unknown())));
    }

    /// <summary>
    /// A count in the Mac's compact form (<c>.number.notation(.compactName)</c>,
    /// en-US): below 1000 as is, else two significant digits (whole when
    /// three), half to even, over K, M, B and T.
    /// </summary>
    public static string CompactFigure(ulong value)
    {
        if (value < 1000) return value.ToString(CultureInfo.InvariantCulture);
        string[] suffixes = { "K", "M", "B", "T" };
        int scale = 0;
        decimal x = value / 1000m;
        while (true)
        {
            decimal rounded = Math.Round(x, x >= 10 ? 0 : 1, MidpointRounding.ToEven);
            if (rounded >= 1000 && scale < suffixes.Length - 1)
            {
                x /= 1000m;
                scale++;
                continue;
            }

            return rounded.ToString("0.#", CultureInfo.InvariantCulture) + suffixes[scale];
        }
    }

    /// <summary>The call's time of day in the reader's short time form.</summary>
    public static string CallTime(DateTimeOffset at, TimeZoneInfo zone, CultureInfo culture) =>
        TimeZoneInfo.ConvertTime(at, zone).ToString("t", culture);

    // ---- JSON helpers ----------------------------------------------------------

    private static string? Str(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    private static long? Long(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt64(out long n) ? n : null;

    private static int? Int(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out int n) ? n : null;

    private static uint? UInt(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetUInt32(out uint n) ? n : null;
}
