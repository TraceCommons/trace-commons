using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

public sealed record SavedInsight(string Id, string Label);
public sealed record SummaryEvidenceLink(string Id, string Label);
public sealed record SummaryRow(string Label, string Details, IReadOnlyList<SummaryEvidenceLink> Evidence);

/// <summary>UI-thread state; all IO belongs to the handle-free service. No metrics are calculated here.</summary>
public sealed class InsightsViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly ILocalInsights _service;
    private CancellationTokenSource? _pending;
    private long _generation;
    private bool _closed;
    private Task _active = Task.CompletedTask;
    private readonly Dictionary<string, string> _copy = new();
    public InsightsViewModel(ILocalInsights? service = null) => _service = service ?? new LocalInsights();
    public event PropertyChangedEventHandler? PropertyChanged;
    public ObservableCollection<SavedInsight> Saved { get; } = new();
    public string Details { get; private set; } = "";
    public SavedInsightsSummary? Summary { get; private set; }
    public string SummaryDetails { get; private set; } = "";
    public string SummaryStatus => Summary == null ? this[Busy ? "working" : "summary_unavailable"]
        : Summary.SavedSnapshots == 0 ? this["summary_empty"] : "";
    public ObservableCollection<SummaryRow> SummaryRows { get; } = new();
    public string Status { get; private set; } = "";
    public bool Busy { get; private set; }
    public bool Idle => !Busy;
    public string? CurrentId { get; private set; }
    private static readonly string[] Categories = { "refactor", "tests", "docs", "debugging", "other", "unknown" };
    private static readonly string[] Outcomes = { "accepted", "partial", "rejected", "unknown" };
    public int CategoryIndex { get; set; } = 5;
    public int OutcomeIndex { get; set; } = 3;
    public Task SaveAssessmentAsync() => AnnotateAsync(
        Categories[Math.Clamp(CategoryIndex, 0, Categories.Length - 1)],
        Outcomes[Math.Clamp(OutcomeIndex, 0, Outcomes.Length - 1)]);
    private void ResetAssessment() { CategoryIndex = 5; OutcomeIndex = 3; }
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
    public async Task LoadAsync()
    {
        await _active;
        await Run(async token =>
        {
            var result = await _service.CallAsync(new { type = "copy" }, token);
            token.ThrowIfCancellationRequested();
            foreach (var pair in result.GetProperty("copy").EnumerateObject())
                _copy[pair.Name] = pair.Value.GetString() ?? pair.Name;
            await RefreshCoreAsync(token);
        });
    }
    public Task RefreshAsync() => Run(RefreshCoreAsync);
    private async Task RefreshCoreAsync(CancellationToken token)
    {
        ClearSummary();
        var response = await _service.CallAsync(new { type = "list" }, token);
        token.ThrowIfCancellationRequested();
        Saved.Clear();
        bool selectedStillPresent = false;
        foreach (var insight in response.GetProperty("insights").EnumerateArray())
        {
            string id = insight.GetProperty("id").GetString()!;
            Saved.Add(new SavedInsight(id,
                this[insight.GetProperty("source_format").GetString()!] + " · " + Date(insight.GetProperty("analyzed_at"))));
            if (CurrentId == id)
            {
                selectedStillPresent = true;
                Render(insight, true);
            }
        }
        // A CLI deletion or replacement invalidates the displayed saved snapshot.
        // Ephemeral previews have no saved identity and survive a list refresh.
        if (CurrentId != null && !selectedStillPresent)
        {
            CurrentId = null;
            Details = "";
            ResetAssessment();
        }
        Status = Saved.Count == 0 ? this["empty"] : "";
        await RefreshSummaryAsync(token);
    }
    public Task AnalyzeAsync(string source, string file, bool save) => Run(async token =>
    {
        if (save) ClearSummary();
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
        ClearSummary();
        await _service.CallAsync(new { type = "delete", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        CurrentId = null;
        Details = "";
        ResetAssessment();
        await RefreshCoreAsync(token);
    });
    public Task AnnotateAsync(string category, string outcome) => Run(async token =>
    {
        if (CurrentId == null) return;
        ClearSummary();
        var result = await _service.CallAsync(new { type = "annotate", id = CurrentId, category, outcome }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
        await RefreshSummaryAsync(token);
    });
    public Task ClearAnnotationAsync() => Run(async token =>
    {
        if (CurrentId == null) return;
        ClearSummary();
        var result = await _service.CallAsync(new { type = "clear_annotation", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
        await RefreshSummaryAsync(token);
    });
    private void ClearSummary()
    {
        Summary = null;
        SummaryDetails = "";
        SummaryRows.Clear();
        Changed();
    }
    private static string Number(ulong value) => value.ToString("N0", CultureInfo.CurrentCulture);
    private static string Date(DateTimeOffset value) => value.ToLocalTime().ToString("g", CultureInfo.CurrentCulture);
    private async Task RefreshSummaryAsync(CancellationToken token)
    {
        ClearSummary();
        var response = await _service.CallAsync(new { type = "summary" }, token);
        token.ThrowIfCancellationRequested();
        var summary = InsightsSummaryResponse.Decode(response);
        // Render before publishing, so a malformed response cannot leave a partial summary.
        var rows = new List<SummaryRow>();
        var snapshots = summary.Snapshots.ToDictionary(snapshot => snapshot.Id, StringComparer.Ordinal);
        IReadOnlyList<SummaryEvidenceLink> Links(string[] ids) => ids.Select(id => {
            if (!snapshots.TryGetValue(id, out var snapshot))
                throw new InvalidOperationException("insights-response-invalid");
            var label = this[snapshot.SourceFormat] + " · " + Date(snapshot.AnalyzedAt) + "\n" +
                snapshot.Id + "\n" + string.Join("\n", snapshot.Evidence.Select(evidence => evidence.SourceDigest));
            return new SummaryEvidenceLink(id, label);
        }).ToArray();
        foreach (var metric in summary.Metrics)
            rows.Add(new SummaryRow(this["metric_" + metric.Id],
                this["summary_observed_sum"] + ": " + (metric.ObservedValueSum is ulong value ? Number(value) : this["unknown"]) + "\n" +
                this["summary_available"] + ": " + Number(metric.AvailableSnapshots) + " · " +
                this["summary_missing"] + ": " + Number(metric.MissingSnapshots) + "\n" +
                this["summary_record_coverage"] + ": " + Number(metric.RecordCoverage.Observed) + "/" + Number(metric.RecordCoverage.Total) +
                " · " + this["summary_unit_" + metric.CoverageUnit], Links(metric.EvidenceSnapshotIds)));
        foreach (var category in summary.UserReported.Categories)
            rows.Add(new SummaryRow(this["category"] + " · " + this["category_" + category.Category], Number(category.Snapshots), Links(category.EvidenceSnapshotIds)));
        foreach (var outcome in summary.UserReported.Outcomes)
            rows.Add(new SummaryRow(this["outcome"] + " · " + this["outcome_" + outcome.Outcome], Number(outcome.Snapshots), Links(outcome.EvidenceSnapshotIds)));
        var lines = new List<string> {
            this["summary_scope"],
            this["summary_snapshots"] + ": " + Number(summary.SavedSnapshots),
            this["summary_assessed"] + ": " + Number(summary.UserReported.AssessedSnapshots),
            this["summary_unassessed"] + ": " + Number(summary.UserReported.UnassessedSnapshots),
            this["provider"] + ": " + summary.Provider.Id + " / " + summary.Provider.Version,
            this["rubric"] + ": " + summary.Provider.RubricVersion,
            this["summary_analysis_range"] + ": " + (summary.SnapshotAnalysisRange is { } range
                ? Date(range.Oldest) + " – " + Date(range.Newest) : this["unknown"])
        };
        lines.AddRange(summary.Limitations.Select(limitation => this["summary_limitation_" + limitation]));
        Summary = summary;
        SummaryDetails = string.Join("\n\n", lines);
        foreach (var row in rows) SummaryRows.Add(row);
    }
    public Task ExplainSummaryEvidenceAsync(string id)
    {
        if (Summary == null || !Summary.Snapshots.Any(snapshot => snapshot.Id == id))
            return Task.CompletedTask;
        return ExplainAsync(id);
    }
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
        ResetAssessment();
        if (insight.TryGetProperty("manual_annotation", out var annotation) && annotation.ValueKind != JsonValueKind.Null)
        {
            CategoryIndex = Array.IndexOf(Categories, annotation.GetProperty("category").GetString());
            OutcomeIndex = Array.IndexOf(Outcomes, annotation.GetProperty("outcome").GetString());
            if (CategoryIndex < 0) CategoryIndex = 5;
            if (OutcomeIndex < 0) OutcomeIndex = 3;
            lines.Add(AssessmentNotice + "\n" + this["category_" + annotation.GetProperty("category").GetString()] + " / " +
                this["outcome_" + annotation.GetProperty("outcome").GetString()] + " · " + Date(annotation.GetProperty("recorded_at")));
        }
        Details = string.Join("\n\n", lines);
    }
    private Task Run(Func<CancellationToken, Task> action)
    {
        if (_closed || Busy) return Task.CompletedTask;
        _active = RunCore(action);
        return _active;
    }
    private async Task RunCore(Func<CancellationToken, Task> action)
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
        catch (Exception)
        {
            if (!_closed && generation == _generation)
            {
                ClearSummary();
                Status = this["error"];
            }
        }
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
