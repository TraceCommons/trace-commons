using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

public sealed record SavedInsight(string Id, string Label);

/// <summary>UI-thread state; all IO belongs to the handle-free service. No metrics are calculated here.</summary>
public sealed class InsightsViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly ILocalInsights _service;
    private CancellationTokenSource? _pending;
    private long _generation;
    private bool _closed;
    private readonly Dictionary<string, string> _copy = new();
    public InsightsViewModel(ILocalInsights? service = null) => _service = service ?? new LocalInsights();
    public event PropertyChangedEventHandler? PropertyChanged;
    public ObservableCollection<SavedInsight> Saved { get; } = new();
    public string Details { get; private set; } = "";
    public string Status { get; private set; } = "";
    public bool Busy { get; private set; }
    public bool Idle => !Busy;
    public string? CurrentId { get; private set; }
    public bool HasSavedSelection => CurrentId != null && Idle;
    public string this[string key] => _copy.TryGetValue(key, out var text) ? text : key;
    public string Title => this["title"];
    public string Intro => this["intro"];
    public string SnapshotNotice => this["snapshot_notice"];
    public string CancellationNotice => this["cancellation_notice"];
    public string UnknownNotice => this["unknown_notice"];
    public string CoverageNotice => this["coverage_notice"];
    public string EvidenceNotice => this["evidence_notice"];
    public string AssessmentNotice => this["assessment_notice"];

    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
    public Task LoadAsync() => Run(async token =>
    {
        var result = await _service.CallAsync(new { type = "copy" }, token);
        token.ThrowIfCancellationRequested();
        foreach (var pair in result.GetProperty("copy").EnumerateObject())
            _copy[pair.Name] = pair.Value.GetString() ?? pair.Name;
        await RefreshCoreAsync(token);
    });
    public Task RefreshAsync() => Run(RefreshCoreAsync);
    private async Task RefreshCoreAsync(CancellationToken token)
    {
        var response = await _service.CallAsync(new { type = "list" }, token);
        token.ThrowIfCancellationRequested();
        Saved.Clear();
        foreach (var insight in response.GetProperty("insights").EnumerateArray())
            Saved.Add(new SavedInsight(insight.GetProperty("id").GetString()!,
                insight.GetProperty("source_format").GetString() + " · " + Date(insight.GetProperty("analyzed_at"))));
        Status = Saved.Count == 0 ? this["empty"] : "";
    }
    public Task AnalyzeAsync(string source, string file, bool save) => Run(async token =>
    {
        var result = await _service.CallAsync(new { type = "analyze", source, file, save }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), save);
        if (save) await RefreshCoreAsync(token);
    });
    public Task ExplainAsync(string id) => Run(async token =>
    {
        var result = await _service.CallAsync(new { type = "explain", id }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
    });
    public Task DeleteAsync() => Run(async token =>
    {
        if (CurrentId == null) return;
        await _service.CallAsync(new { type = "delete", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        CurrentId = null;
        Details = "";
        await RefreshCoreAsync(token);
    });
    public Task AnnotateAsync(string category, string outcome) => Run(async token =>
    {
        if (CurrentId == null) return;
        var result = await _service.CallAsync(new { type = "annotate", id = CurrentId, category, outcome }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
    });
    public Task ClearAnnotationAsync() => Run(async token =>
    {
        if (CurrentId == null) return;
        var result = await _service.CallAsync(new { type = "clear_annotation", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
    });
    private static string Date(JsonElement value) => value.GetDateTimeOffset().ToLocalTime().ToString("g", CultureInfo.CurrentCulture);
    private void Render(JsonElement insight, bool saved)
    {
        CurrentId = saved ? insight.GetProperty("id").GetString() : null;
        var report = insight.GetProperty("report");
        var provider = report.GetProperty("provider");
        var lines = new List<string> {
            this["analyzed_at"] + ": " + Date(insight.GetProperty("analyzed_at")),
            this["provider"] + ": " + provider.GetProperty("id").GetString() + " / " + provider.GetProperty("version").GetString(),
            this["rubric"] + ": " + provider.GetProperty("rubric_version").GetString()
        };
        foreach (var metric in report.GetProperty("metrics").EnumerateArray())
        {
            var coverage = metric.GetProperty("coverage");
            var value = metric.GetProperty("value");
            lines.Add(this["metric_" + metric.GetProperty("id").GetString()] + ": " +
                (value.ValueKind == JsonValueKind.Null ? this["unknown"] : value.GetUInt64().ToString("N0", CultureInfo.CurrentCulture)) +
                " · " + this["coverage"] + " " + coverage.GetProperty("observed").GetUInt64().ToString("N0") + "/" + coverage.GetProperty("total").GetUInt64().ToString("N0"));
        }
        lines.Add(UnknownNotice);
        foreach (var evidence in report.GetProperty("evidence").EnumerateArray())
            lines.Add(this["evidence"] + ": " + evidence.GetProperty("id").GetString() + "\nSHA-256: " + evidence.GetProperty("source_digest").GetString());
        if (insight.TryGetProperty("manual_annotation", out var annotation) && annotation.ValueKind != JsonValueKind.Null)
            lines.Add(AssessmentNotice + "\n" + this["category_" + annotation.GetProperty("category").GetString()] + " / " +
                this["outcome_" + annotation.GetProperty("outcome").GetString()] + " · " + Date(annotation.GetProperty("recorded_at")));
        Details = string.Join("\n\n", lines);
    }
    private async Task Run(Func<CancellationToken, Task> action)
    {
        if (_closed || Busy) return;
        var generation = ++_generation;
        using var cancellation = new CancellationTokenSource();
        _pending = cancellation;
        Busy = true;
        Status = "";
        Changed();
        try { await action(cancellation.Token); }
        catch (OperationCanceledException) { }
        catch (Exception) { if (!_closed && generation == _generation) Status = this["error"]; }
        finally
        {
            if (!_closed && generation == _generation)
            {
                Busy = false;
                _pending = null;
                Changed();
            }
        }
    }
    public void ReportError()
    {
        if (_closed) return;
        Status = this["error"];
        Changed();
    }
    public void Cancel()
    {
        if (_closed) return;
        _pending?.Cancel();
        // Keep mutations serialized until the outstanding operation settles.
        Status = CancellationNotice;
        Changed();
    }
    public void Dispose()
    {
        _closed = true;
        ++_generation;
        _pending?.Cancel();
    }
}
