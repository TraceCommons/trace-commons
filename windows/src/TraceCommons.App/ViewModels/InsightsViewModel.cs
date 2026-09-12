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
public sealed record EvidenceTarget(string SnapshotId, long Version);
public sealed record OutcomeEvidenceRow(string Id, string SnapshotId, string Label, string Details, string UnlinkLabel);
public sealed record ModelReferenceRow(string Label);
public sealed record SummaryEvidenceLink(string Id, string Label);
public sealed record SummaryRow(string Label, string Details, IReadOnlyList<SummaryEvidenceLink> Evidence);

/// <summary>UI-thread state; all IO belongs to the handle-free service. No metrics are calculated here.</summary>
public sealed class InsightsViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly ILocalInsights _service;
    private CancellationTokenSource? _pending;
    private long _generation;
    private long _selectionVersion;
    private bool _closed;
    private Task _active = Task.CompletedTask;
    private readonly Dictionary<string, string> _copy = new();
    public InsightsViewModel(ILocalInsights? service = null) => _service = service ?? new LocalInsights();
    public event PropertyChangedEventHandler? PropertyChanged;
    public ObservableCollection<SavedInsight> Saved { get; } = new();
    public string Details { get; private set; } = "";
    public string ModelDetails { get; private set; } = "";
    public string CommitId { get; set; } = "";
    public ObservableCollection<ModelReferenceRow> ModelReferences { get; } = new();
    public ObservableCollection<OutcomeEvidenceRow> OutcomeEvidence { get; } = new();
    public string EvidenceSelectionStatus => CurrentId == null ? this["link_saved_required"] : "";
    public string OutcomeEvidenceStatus => OutcomeEvidence.Count == 0 ? this["link_empty"] : "";
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
        ++_selectionVersion;
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
            ClearSelectedInsight();
        }
        Status = Saved.Count == 0 ? this["empty"] : "";
        await RefreshSummaryAsync(token);
    }
    public Task AnalyzeAsync(string source, string file, bool save) => Run(async token =>
    {
        if (save) ClearSummary();
        ClearSelectedInsight();
        var result = await _service.CallAsync(new { type = "analyze", source, file, save }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), save);
        if (save) await RefreshCoreAsync(token);
    });
    public Task ExplainAsync(string id) => Run(async token =>
    {
        ClearSelectedInsight();
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
        ClearSelectedInsight();
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
    public EvidenceTarget? CaptureEvidenceTarget() => !_closed && HasSavedSelection
        ? new EvidenceTarget(CurrentId!, ++_selectionVersion) : null;
    public void CancelEvidenceTarget(EvidenceTarget target)
    {
        // An older picker must not invalidate a newer picker when it closes.
        if (!_closed && target.Version == _selectionVersion && target.SnapshotId == CurrentId)
            ++_selectionVersion;
    }
    private bool ValidTarget(EvidenceTarget target) => !_closed && !Busy &&
        target.Version == _selectionVersion && target.SnapshotId == CurrentId;
    private Task LinkOperationAsync(EvidenceTarget target, object operation, string success)
    {
        if (!ValidTarget(target))
        {
            if (!_closed) { Status = this["link_changed_selection"]; Changed(); }
            return Task.CompletedTask;
        }
        return Run(async token =>
        {
            ClearSummary();
            ClearSelectedInsight();
            var result = await _service.CallAsync(operation, token);
            token.ThrowIfCancellationRequested();
            if (result.GetProperty("type").GetString() != JsonSerializer.SerializeToElement(operation).GetProperty("type").GetString())
                throw new InvalidOperationException("insights-response-invalid");
            var insight = result.GetProperty("insight");
            if (insight.GetProperty("id").GetString() != target.SnapshotId)
                throw new InvalidOperationException("insights-response-invalid");
            Render(insight, true);
            await RefreshCoreAsync(token);
            Status = this[success];
        });
    }
    public Task LinkGitAsync(EvidenceTarget target, string repository, string commit) =>
        LinkOperationAsync(target, new { type = "link_git", id = target.SnapshotId, repository, commit }, "link_success");
    public Task LinkTestReportAsync(EvidenceTarget target, string file) =>
        LinkOperationAsync(target, new { type = "link_test_report", id = target.SnapshotId, file }, "link_success");
    public Task UnlinkEvidenceAsync(EvidenceTarget target, string evidenceId) =>
        LinkOperationAsync(target, new { type = "unlink_evidence", id = target.SnapshotId, evidence_id = evidenceId }, "unlink_success");
    private void ClearSelectedInsight()
    {
        ++_selectionVersion;
        CurrentId = null;
        Details = "";
        ModelDetails = "";
        ModelReferences.Clear();
        OutcomeEvidence.Clear();
        CommitId = "";
        ResetAssessment();
        Changed();
    }
    private void RenderEvidence(InsightEvidence projection, string? snapshotId)
    {
        ModelReferences.Clear();
        OutcomeEvidence.Clear();
        if (projection.ModelObservations is { } models)
        {
            if (models.SchemaVersion != 1 || models.Scope != "declared_metadata_only" ||
                models.Coordinates is not ("jsonl_physical_lines_one_based" or "trajectory_array_indexes_zero_based"))
                throw new InvalidOperationException("insights-response-invalid");
            var lines = new List<string> { this["model_notice"],
                this[models.MixedDeclaredModels ? "model_mixed" : "model_not_proven_mixed"],
                this["model_labels"] + ": " + (models.DeclaredModels.Length == 0 ? this["model_no_labels"] : string.Join(", ", models.DeclaredModels)),
                this["model_record_count"] + ": " + Number(models.RecordCount),
                this["model_candidates"] + ": " + Number(models.CandidateRecords),
                this["model_valid"] + ": " + Number(models.ValidDeclarations),
                this["model_missing"] + ": " + Number(models.MissingDeclarations),
                this["model_invalid"] + ": " + Number(models.InvalidDeclarations),
                this["model_omitted"] + ": " + Number(models.OmittedDeclarations),
                this["model_coordinates_" + models.Coordinates],
                this["source_digest"] + ": " + models.SourceDigest
            };
            if (models.ModelLabelsOmitted) lines.Add(this["model_labels_omitted"]);
            ModelDetails = string.Join("\n", lines);
            foreach (var declaration in models.Declarations)
            {
                if (declaration.Kind is not ("codex_session_metadata" or "codex_turn_context" or "codex_assistant_message" or "trajectory_metadata"))
                    throw new InvalidOperationException("insights-response-invalid");
                ModelReferences.Add(new ModelReferenceRow(declaration.Model + " · " + this["model_kind_" + declaration.Kind] + " · " +
                    this["model_record_index"] + ": " + Number(declaration.RecordIndex)));
            }
        }
        else ModelDetails = this["model_legacy"];
        foreach (var link in projection.OutcomeLinks)
        {
            if (snapshotId == null || link.Provenance != "user_linked")
                throw new InvalidOperationException("insights-response-invalid");
            string label;
            var lines = new List<string> { this["link_user_provenance"], this["link_notice"],
                this["link_recorded_at"] + ": " + Date(link.LinkedAt), this["source_digest"] + ": " + link.SourceDigest,
                this["link_id"] + ": " + link.Id };
            switch (link.Evidence)
            {
                case GitCommitObservation { Evidence: var git }:
                    if (git.Provenance != "inspected_local_object") throw new InvalidOperationException("insights-response-invalid");
                    label = this["link_git_provenance"];
                    lines.AddRange(new[] { this["link_git_notice"], this["git_object"] + ": " + git.ObjectId,
                        this["git_tree"] + ": " + git.TreeId, this["git_parents"] + ": " + (git.ParentIds.Length == 0 ? this["git_no_parents"] : string.Join(", ", git.ParentIds)),
                        this["git_repository_digest"] + ": " + git.RepositoryPathDigest, this["git_inspected_at"] + ": " + Date(git.InspectedAt) });
                    break;
                case TestReportObservation { Evidence: var report }:
                    if (report.Provenance != "imported_report" || report.SchemaVersion != 1) throw new InvalidOperationException("insights-response-invalid");
                    label = this["link_test_provenance"];
                    lines.AddRange(new[] { this["link_test_notice"], this["test_runner"] + ": " + report.Runner,
                        this["test_passed"] + ": " + Number(report.Passed), this["test_failed"] + ": " + Number(report.Failed),
                        this["test_skipped"] + ": " + Number(report.Skipped), this["test_observed_at"] + ": " + Date(report.ObservedAt),
                        this["test_imported_at"] + ": " + Date(report.ImportedAt), this["artifact_digest"] + ": " + report.ArtifactDigest,
                        this["test_claimed_commit"] + ": " + (report.CommitId ?? this["unknown"]) });
                    break;
                default: throw new InvalidOperationException("insights-response-invalid");
            }
            OutcomeEvidence.Add(new OutcomeEvidenceRow(link.Id, snapshotId, label, string.Join("\n", lines), this["unlink_evidence"]));
        }
    }
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
        var projection = InsightEvidence.Decode(insight);
        ++_selectionVersion;
        CommitId = "";
        CurrentId = saved ? insight.GetProperty("id").GetString() : null;
        RenderEvidence(projection, CurrentId);
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
                ClearSelectedInsight();
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
        ++_selectionVersion;
        _pending?.Cancel();
        // Keep mutations serialized until the outstanding operation settles.
        Status = CancellationNotice;
        Changed();
    }
    public void Dispose()
    {
        _closed = true;
        ++_selectionVersion;
        ++_generation;
        _pending?.Cancel();
    }
}
