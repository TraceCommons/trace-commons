using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>A proof tag: the core's word (and count, on a model row) in its tone.</summary>
public sealed record LedgerTag(string Text, LedgerTone Tone);

/// <summary>One call row of the Calls card (InferenceViews.swift:286-325).</summary>
public sealed record LedgerCallRow(
    long Id, string Time, string Tool, string Model, string? TokenLine, string? TokenAccessibilityLine,
    string Priced, LedgerTag Proof)
{
    public bool HasTokens => TokenLine is not null;

    /// <summary>The row read out as one element, as <c>.accessibilityElement(children: .combine)</c> does.</summary>
    public string AccessibleName => string.Join(", ", new[] { Time, Tool, Model, TokenAccessibilityLine, Priced, Proof.Text }
        .Where(part => !string.IsNullOrEmpty(part)));
}

/// <summary>
/// The Private AI tab's ledger sections (spec W19, <c>InferenceTabView.ledgerSections</c>
/// and <c>InferenceStore</c>'s reads, InferenceViews.swift:91-117,
/// InferenceStore.swift:120-245). It reads <c>inference_calls</c>,
/// <c>inference_summary</c> and <c>tool_destinations</c> together, records
/// each failure by method, and draws a read answered for an older load as
/// nothing. The per-model summary and its Models card are not read: the
/// Mac's live client never sends that provisional method
/// (LiveDaemonClient.swift:435-437), so the card never draws there.
/// </summary>
public sealed class InferenceLedgerModel : INotifyPropertyChanged
{
    /// <summary>The three methods this ledger sends, by name (one-pass F1).</summary>
    public static readonly IReadOnlyList<string> Methods = new[]
    {
        "inference_calls", "inference_summary", "tool_destinations",
    };

    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<string, string> _monitorWord;
    private readonly Func<string, string> _insightsWord;
    private readonly Func<string, string?> _toolName;
    private readonly TimeZoneInfo _zone;
    private readonly CultureInfo _culture;
    private readonly Dictionary<string, string> _failures = new(StringComparer.Ordinal);
    private long _generation;

    private InferenceCallPage? _page;
    private NetworkInferenceSummary? _network;
    private DestinationCallCounts? _destinations;

    public InferenceLedgerModel(
        Func<string, string, Task<DaemonResponse>> call,
        Func<string, string> monitorWord,
        Func<string, string> insightsWord,
        Func<string, string?> toolName,
        TimeZoneInfo zone,
        CultureInfo culture)
    {
        _call = call;
        _monitorWord = monitorWord;
        _insightsWord = insightsWord;
        _toolName = toolName;
        _zone = zone;
        _culture = culture;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Every read again, together; the newest load wins.</summary>
    public async Task LoadAsync()
    {
        long mine = ++_generation;
        Task<(string, DaemonResponse?)> Read(string method, string parameters) => ReadOne(method, parameters);
        var reads = await Task.WhenAll(
            Read("inference_calls", "{\"limit\":" + InferenceLedger.PageSize.ToString(CultureInfo.InvariantCulture) + "}"),
            Read("inference_summary", "{}"),
            Read("tool_destinations", "{}")).ConfigureAwait(true);
        if (mine != _generation) return;
        foreach ((string method, DaemonResponse? response) in reads)
        {
            JsonElement? result = response is { IsError: false, Result: { } r } ? r : null;
            if (result is null)
            {
                // No daemon to ask is the core down; any other refusal, or a
                // reply that did not parse, is a failed request
                // (MonitorScreensCopy.swift:371-374).
                _failures[method] = response?.Error is { Code: "unavailable", Message: not "malformed-response" }
                    ? "core_unreachable" : "request_failed";
            }
            else
            {
                _failures.Remove(method);
            }

            switch (method)
            {
                case "inference_calls":
                    if (result is { } page && InferenceLedger.ParsePage(page) is { } parsed) _page = parsed;
                    break;
                case "inference_summary":
                    // Only a readable network summary stands (InferenceStore.swift:240).
                    _network = result is { } n && InferenceLedger.ParseNetworkSummary(n) is { Readable: true } network ? network : null;
                    break;
                case "tool_destinations":
                    _destinations = result is { } d ? InferenceLedger.ParseDestinations(d) : null;
                    break;
            }
        }

        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
    }

    private async Task<(string, DaemonResponse?)> ReadOne(string method, string parameters)
    {
        try
        {
            return (method, await _call(method, parameters).ConfigureAwait(true));
        }
        catch (Exception)
        {
            return (method, null);
        }
    }

    /// <summary>
    /// A failed calls read, in the core's words over the last page, never the
    /// error's fixed label (InferenceViews.swift:96-98). Empty when it answered.
    /// </summary>
    public string FailureLine => _failures.TryGetValue("inference_calls", out string? key) ? _monitorWord(key) : string.Empty;

    public bool HasFailure => FailureLine.Length > 0;

    /// <summary>The spinner, only before any page and without a failure.</summary>
    public bool Loading => _page is null && !_failures.ContainsKey("inference_calls");

    /// <summary>The header is drawn over the network summary, or over a readable page.</summary>
    public bool ShowsHeader => _network is not null || _page is { Readable: true };

    private LedgerTotals Header => _network is not null
        ? InferenceLedger.NetworkHeaderTotals(_network)
        : _page is { Readable: true } page ? InferenceLedger.PageHeaderTotals(page, null, _destinations)
        : new LedgerTotals(null, null, InferenceLedger.Dash);

    public string WindowLine => !ShowsHeader ? string.Empty
        : InferenceLedger.WindowLine(
            _network is not null ? _network.WindowHours : InferenceLedger.WindowHours(_page!, _destinations),
            _monitorWord("window_last_hours"));

    public string CallsWord => _monitorWord("calls");

    public string VerifiedWord => _monitorWord("proof_verified");

    public string PricedWord => _monitorWord("priced");

    public string CallsFigure => Header.Calls?.ToString(CultureInfo.InvariantCulture) ?? InferenceLedger.Dash;

    public string VerifiedFigure => Header.Verified?.ToString(CultureInfo.InvariantCulture) ?? InferenceLedger.Dash;

    public string PricedFigure => Header.Priced;

    /// <summary>The Calls card, only for calls to list: an empty ledger is said by the totals.</summary>
    public IReadOnlyList<LedgerCallRow> Calls => _page is { Readable: true } page
        ? page.Calls.Select(Row).ToList()
        : Array.Empty<LedgerCallRow>();

    public bool ShowsCalls => Calls.Count > 0;

    private LedgerCallRow Row(InferenceCall call) => new(
        call.Id,
        InferenceLedger.CallTime(call.At, _zone, _culture),
        InferenceLedger.ToolName(call.Tool, _toolName),
        call.Model,
        call.Tokens is { } tokens ? InferenceLedger.TokenLine(tokens, _insightsWord) : null,
        call.Tokens is { } t ? InferenceLedger.TokenAccessibilityLine(t, _insightsWord, _monitorWord("unknown")) : null,
        InferenceLedger.Priced(new[] { call.Cost }),
        new LedgerTag(ProofWord(call.Proof), InferenceLedger.Tone(call.Proof)));

    private string ProofWord(string raw) => InferenceLedger.ProofWord(raw, _monitorWord);
}
