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
public sealed record EpisodeRow(string Id, string Label);
public sealed record EpisodeMemberRow(string Id, string Label, string Details);
public sealed record EpisodeTarget(string Id, ulong Revision, ulong MembershipRevision, long PresentationVersion);
public sealed record CardEvidenceLink(string Id, string Label);
public sealed record QuestionCardRow(string Label, string Value);
public sealed record QuestionCardView(string Question, string State, string MetricVersion,
    IReadOnlyList<QuestionCardRow> Rows, string Coverage, string Limitations,
    IReadOnlyList<CardEvidenceLink> Evidence, IReadOnlyList<CardEvidenceLink> Episodes, string Omission);

/// <summary>Owns the frozen revision used by the control's complete member-selection draft.</summary>
public sealed class EpisodeMemberDraftBinding
{
    private EpisodeTarget? _target;
    public bool HasDraft => _target != null;
    public EpisodeTarget? Consume() { var target = _target; _target = null; return target; }
    public void Clear() => _target = null;
    public bool Reconcile(InsightsViewModel model, string expectedId)
    {
        var target = model.CaptureEpisodeTarget();
        if (target == null || target.Id != expectedId) { Clear(); return false; }
        _target = target;
        return true;
    }
}

/// <summary>UI-thread state; all IO belongs to the handle-free service. No metrics are calculated here.</summary>
public sealed class InsightsViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly ILocalInsights _service;
    private CancellationTokenSource? _pending;
    private long _generation;
    private long _selectionVersion;
    private long _episodePresentationVersion;
    private long _cardPresentationVersion;
    private bool _episodeReviewRequired;
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
    public ObservableCollection<EpisodeRow> Episodes { get; } = new();
    public ObservableCollection<EpisodeMemberRow> EpisodeMembers { get; } = new();
    public ObservableCollection<QuestionCardView> QuestionCards { get; } = new();
    public string CardStatus { get; private set; } = "";
    public bool HasQuestionCards => QuestionCards.Count != 0;
    public string EpisodeStatus { get; private set; } = "";
    public string EpisodeDetails { get; private set; } = "";
    public string? CurrentEpisodeId { get; private set; }
    public ulong CurrentEpisodeRevision { get; private set; }
    public ulong CurrentEpisodeMembershipRevision { get; private set; }
    public bool HasEpisodeSelection => CurrentEpisodeId != null && Idle;
    public string Status { get; private set; } = "";
    public string MutationNotice { get; private set; } = "";
    public bool Busy { get; private set; }
    public bool Idle => !Busy;
    public string? CurrentId { get; private set; }
    private static readonly string[] Categories = { "refactor", "tests", "docs", "debugging", "other", "unknown" };
    private static readonly string[] Outcomes = { "accepted", "partial", "rejected", "unknown" };
    public int CategoryIndex { get; set; } = 5;
    public int OutcomeIndex { get; set; } = 3;
    public int EpisodeCategoryIndex { get; set; } = 5;
    public int EpisodeOutcomeIndex { get; set; } = 3;
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
        ClearQuestionCards();
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
        await RefreshEpisodesCoreAsync(token, CurrentEpisodeId);
    }

    public Task LoadQuestionCardsAsync(IEnumerable<string> snapshotIds, IEnumerable<string> episodeIds)
    {
        if (_closed || Busy) return Task.CompletedTask;
        string[] snapshots = snapshotIds.Distinct(StringComparer.Ordinal).OrderBy(id => id, StringComparer.Ordinal).ToArray();
        string[] episodes = episodeIds.Distinct(StringComparer.Ordinal).OrderBy(id => id, StringComparer.Ordinal).ToArray();
        long presentation = ++_cardPresentationVersion;
        ClearQuestionCards(false);
        CardStatus = this["working"];
        Changed();
        return Run(async token =>
        {
            try
            {
                var response = await _service.CallAsync(new {
                    type = "question_cards",
                    questions = new[] { "recorded_activity", "episode_outcomes", "observed_models", "estimated_cost" },
                    snapshot_ids = snapshots, episode_ids = episodes
                }, token);
                token.ThrowIfCancellationRequested();
                var result = InsightCardResponses.Decode(response);
                if (presentation != _cardPresentationVersion) return;
                var rendered = RenderQuestionCards(result);
                QuestionCards.Clear();
                foreach (var card in rendered) QuestionCards.Add(card);
                CardStatus = "";
                Changed();
            }
            catch (InsightsServiceException error)
            {
                if (presentation != _cardPresentationVersion) return;
                ClearQuestionCards(false);
                CardStatus = this[CardErrorCopyKey(error.Code)];
                Changed();
            }
            catch (Exception)
            {
                if (presentation != _cardPresentationVersion) return;
                ClearQuestionCards(false);
                CardStatus = this["error"];
                Changed();
            }
        });
    }

    public Task OpenCardEvidenceAsync(string id)
    {
        if (!QuestionCards.SelectMany(card => card.Evidence).Any(link => link.Id == id)) return Task.CompletedTask;
        return ExplainAsync(id);
    }

    public Task OpenCardEpisodeAsync(string id)
    {
        if (!QuestionCards.SelectMany(card => card.Episodes).Any(link => link.Id == id)) return Task.CompletedTask;
        return OpenEpisodeAsync(id);
    }

    private IReadOnlyList<QuestionCardView> RenderQuestionCards(InsightCardResult result)
    {
        string Value(InsightCardRow row)
        {
            if (row.Value == null) return this["card_missing_" + row.MissingReason];
            return row.Value.Type switch {
                "count" => Number(row.Value.UnsignedValue!.Value),
                "milliseconds" => Number(row.Value.UnsignedValue!.Value) + " ms",
                "unix_milliseconds" => Date(DateTimeOffset.FromUnixTimeMilliseconds(row.Value.SignedValue!.Value)),
                _ => throw new InvalidOperationException("insights-response-invalid")
            };
        }
        return result.Cards.Select(card => new QuestionCardView(
            this["card_question_" + card.Question], this["card_state_" + card.State], card.MetricVersion,
            card.Rows.Select(row => new QuestionCardRow(this["card_row_" + row.Id] +
                (row.Label == null ? "" : " (" + row.Label + ")"), Value(row))).ToArray(),
            string.Join("\n", card.Coverage.Select(item => this["card_coverage_" + item.Unit] + ": " +
                Number(item.Observed) + " / " + Number(item.Eligible))),
            string.Join("\n", card.Limitations.Select(item => this["card_limitation_" + item])),
            card.EvidenceIds.Select(id => new CardEvidenceLink(id, this["card_evidence"] + " · " + id)).ToArray(),
            card.EpisodeIds.Select(id => new CardEvidenceLink(id, this["card_episodes"] + " · " + id)).ToArray(),
            card.RowsOmitted ? this["card_more_models"] : "")).ToArray();
    }

    private static string CardErrorCopyKey(string code) => code switch {
        "insights_card_snapshot_not_found" => "episode_missing_members",
        "insights_card_episode_not_found" => "episode_missing",
        "insights_card_snapshot_limit" => "episode_member_limit",
        "insights_card_episode_limit" => "episode_limit",
        _ => "error"
    };

    private void ClearQuestionCards(bool advance = true)
    {
        if (advance) ++_cardPresentationVersion;
        QuestionCards.Clear();
        CardStatus = "";
        Changed();
    }
    public Task AnalyzeAsync(string source, string file, bool save) => Run(async token =>
    {
        if (save) ClearSummary();
        ClearSelectedInsight();
        var result = await _service.CallAsync(new { type = "analyze", source, file, save }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), save);
        RenderMutationEffects(result);
        if (save) await RefreshCoreAsync(token);
    });
    public Task ExplainAsync(string id) => Run(async token =>
    {
        ClearSelectedInsight();
        var result = await _service.CallAsync(new { type = "explain", id }, token);
        token.ThrowIfCancellationRequested();
        Render(result.GetProperty("insight"), true);
    });

    public Task CreateEpisodeAsync(IEnumerable<string> snapshotIds) => Run(async token =>
    {
        string[] ids = snapshotIds.ToArray();
        if (ids.Length == 0) { EpisodeStatus = this["episode_selection_empty"]; Changed(); return; }
        try
        {
            var response = await _service.CallAsync(new { type = "episode_create", snapshot_ids = ids }, token);
            token.ThrowIfCancellationRequested();
            var episode = InsightEpisodeResponses.DecodeMutation(response, "episode_create");
            ClearQuestionCards();
            bool reconciled = await RefreshEpisodesCoreAsync(token, episode.Id);
            EpisodeStatus = reconciled ? this["episode_create_success"] : this["episode_create_success"] + "\n" + EpisodeStatus;
        }
        catch (InsightsServiceException error) { await HandleEpisodeFailureAsync(error, token, null); }
        catch (Exception) { ClearEpisodeDetail(); EpisodeStatus = this["error"]; Changed(); }
    });

    public Task OpenEpisodeAsync(string id) => Run(async token =>
    {
        ClearEpisodeDetail();
        long presentation = _episodePresentationVersion;
        try
        {
            var response = await _service.CallAsync(new { type = "episode_explain", id }, token);
            token.ThrowIfCancellationRequested();
            var detail = InsightEpisodeResponses.DecodeDetail(response);
            if (detail.Episode.Id != id) throw new InvalidOperationException("insights-response-invalid");
            if (presentation == _episodePresentationVersion) { RenderEpisode(detail); EpisodeStatus = ""; }
        }
        catch (InsightsServiceException error) { await HandleEpisodeFailureAsync(error, token, id); }
        catch (Exception) { ClearEpisodeDetail(); EpisodeStatus = this["episode_detail_unavailable"]; Changed(); }
    });

    public EpisodeTarget? CaptureEpisodeTarget() => !_closed && HasEpisodeSelection && !_episodeReviewRequired
        ? new EpisodeTarget(CurrentEpisodeId!, CurrentEpisodeRevision, CurrentEpisodeMembershipRevision, _episodePresentationVersion) : null;
    private bool ValidEpisodeTarget(EpisodeTarget target) => !_closed && !Busy && !_episodeReviewRequired && target.Id == CurrentEpisodeId &&
        target.Revision == CurrentEpisodeRevision && target.PresentationVersion == _episodePresentationVersion;

    public Task ReplaceEpisodeMembersAsync(EpisodeTarget target, IEnumerable<string> snapshotIds) =>
        MutateEpisodeAsync(target, "episode_replace_members", new {
            type = "episode_replace_members", id = target.Id, expected_revision = target.Revision,
            snapshot_ids = snapshotIds.ToArray()
        }, "episode_members_saved");
    public Task SaveEpisodeAssessmentAsync(EpisodeTarget target) => MutateEpisodeAsync(target, "episode_annotate", new {
        type = "episode_annotate", id = target.Id, expected_revision = target.Revision,
        category = Categories[Math.Clamp(EpisodeCategoryIndex, 0, Categories.Length - 1)],
        outcome = Outcomes[Math.Clamp(EpisodeOutcomeIndex, 0, Outcomes.Length - 1)]
    }, "episode_assessment_saved");
    public Task ClearEpisodeAssessmentAsync(EpisodeTarget target) => MutateEpisodeAsync(target, "episode_clear_assessment",
        new { type = "episode_clear_assessment", id = target.Id, expected_revision = target.Revision }, "episode_assessment_cleared");

    public Task DeleteEpisodeAsync(EpisodeTarget target)
    {
        if (!ValidEpisodeTarget(target)) return StaleEpisodeTarget();
        return Run(async token =>
        {
            try
            {
                var response = await _service.CallAsync(new { type = "episode_delete", id = target.Id, expected_revision = target.Revision }, token);
                token.ThrowIfCancellationRequested();
                var deleted = InsightEpisodeResponses.DecodeMutation(response, "episode_delete");
                if (deleted.Id != target.Id) throw new InvalidOperationException("insights-response-invalid");
                ClearQuestionCards();
                ClearEpisodeDetail();
                bool reconciled = await RefreshEpisodesCoreAsync(token, null);
                EpisodeStatus = reconciled ? this["episode_deleted"] : this["episode_deleted"] + "\n" + EpisodeStatus;
            }
            catch (InsightsServiceException error) { await HandleEpisodeFailureAsync(error, token, target.Id); }
            catch (Exception) { ClearEpisodeDetail(); EpisodeStatus = this["episode_detail_unavailable"]; Changed(); }
        });
    }

    private Task MutateEpisodeAsync(EpisodeTarget target, string responseType, object operation, string successKey)
    {
        if (!ValidEpisodeTarget(target)) return StaleEpisodeTarget();
        return Run(async token =>
        {
            try
            {
                var response = await _service.CallAsync(operation, token);
                token.ThrowIfCancellationRequested();
                var episode = InsightEpisodeResponses.DecodeMutation(response, responseType);
                if (episode.Id != target.Id) throw new InvalidOperationException("insights-response-invalid");
                ClearQuestionCards();
                bool reconciled = await RefreshEpisodesCoreAsync(token, target.Id);
                string committed = successKey == "episode_members_saved" && episode.MembershipRevision != target.MembershipRevision
                    ? this[successKey] + "\n" + this["episode_membership_changed"] : this[successKey];
                EpisodeStatus = reconciled ? committed : committed + "\n" + EpisodeStatus;
            }
            catch (InsightsServiceException error) { await HandleEpisodeFailureAsync(error, token, target.Id); }
            catch (Exception) { ClearEpisodeDetail(); EpisodeStatus = this["episode_detail_unavailable"]; Changed(); }
        });
    }

    private Task StaleEpisodeTarget()
    {
        if (!_closed) { EpisodeStatus = this["episode_revision_conflict"]; Changed(); }
        return Task.CompletedTask;
    }

    private async Task HandleEpisodeFailureAsync(InsightsServiceException error, CancellationToken token, string? id)
    {
        ++_episodePresentationVersion;
        if (error.Code == "insights_episode_revision_conflict")
        {
            ClearQuestionCards();
            EpisodeStatus = this["episode_revision_conflict"];
            bool reconciled = await RefreshEpisodesCoreAsync(token, id);
            _episodeReviewRequired = true;
            EpisodeStatus = reconciled ? this["episode_revision_conflict"] : this["episode_revision_conflict"] + "\n" + EpisodeStatus;
        }
        else if (error.Code == "insights_episode_not_found")
        {
            ClearEpisodeDetail();
            await RefreshEpisodesCoreAsync(token, null);
            EpisodeStatus = this["episode_missing"];
        }
        else EpisodeStatus = this[EpisodeErrorCopyKey(error.Code)];
        Changed();
    }

    private static string EpisodeErrorCopyKey(string code) => code switch {
        "insights_episode_member_limit" => "episode_member_limit",
        "insights_episode_duplicate_member" or "insights_episode_invalid" or "insights_episode_revision_overflow" => "episode_invalid",
        "insights_episode_missing_members" => "episode_missing_members",
        "insights_episode_limit_exceeded" => "episode_limit",
        "insights_response_too_large" => "episode_response_too_large",
        _ => "error"
    };

    private async Task<bool> RefreshEpisodesCoreAsync(CancellationToken token, string? detailId)
    {
        try
        {
            var response = await _service.CallAsync(new { type = "episode_list" }, token);
            token.ThrowIfCancellationRequested();
            var listed = InsightEpisodeResponses.DecodeList(response);
            Episodes.Clear();
            foreach (var item in listed)
                Episodes.Add(new EpisodeRow(item.Episode.Id, item.Episode.Id + " · " + Number((ulong)item.Episode.Members.Count) + " · " + Date(item.Episode.UpdatedAt)));
            if (detailId == null || !listed.Any(item => item.Episode.Id == detailId))
            {
                if (CurrentEpisodeId != null) ClearEpisodeDetail();
                EpisodeStatus = Episodes.Count == 0 ? this["episode_empty"] : "";
                return true;
            }
            var detailResponse = await _service.CallAsync(new { type = "episode_explain", id = detailId }, token);
            token.ThrowIfCancellationRequested();
            var detail = InsightEpisodeResponses.DecodeDetail(detailResponse);
            if (detail.Episode.Id != detailId) throw new InvalidOperationException("insights-response-invalid");
            RenderEpisode(detail);
            return true;
        }
        catch (InsightsServiceException error) when (error.Code == "insights_episode_not_found")
        {
            ClearEpisodeDetail();
            EpisodeStatus = this["episode_missing"];
            return false;
        }
        catch (OperationCanceledException) { throw; }
        catch (Exception)
        {
            if (detailId != null) ClearEpisodeDetail();
            EpisodeStatus = this[detailId == null ? "episode_list_unavailable" : "episode_detail_unavailable"];
            Changed();
            return false;
        }
    }

    private void RenderEpisode(EpisodeDetail detail)
    {
        ++_episodePresentationVersion;
        CurrentEpisodeId = detail.Episode.Id;
        CurrentEpisodeRevision = detail.Episode.Revision;
        CurrentEpisodeMembershipRevision = detail.Episode.MembershipRevision;
        _episodeReviewRequired = false;
        EpisodeMembers.Clear();
        var overlaps = detail.Overlap.ToDictionary(item => item.SnapshotId, StringComparer.Ordinal);
        foreach (var member in detail.Members)
        {
            string overlap = overlaps.TryGetValue(member.SnapshotId, out var item) && item.EpisodeIds.Count != 0
                ? this["episode_overlaps"] + ": " + string.Join(", ", item.EpisodeIds) : this["episode_no_overlap"];
            string evidence = member.Evidence.Count == 0 ? this["link_empty"] : string.Join("\n", member.Evidence.Select(item => item.Id + " · " + item.SourceDigest));
            EpisodeMembers.Add(new EpisodeMemberRow(member.SnapshotId,
                this[member.SourceFormat] + " · " + Date(member.AnalyzedAt), overlap + "\n" + this["episode_member_evidence"] + "\n" + evidence));
        }
        EpisodeCategoryIndex = detail.Episode.Assessment == null ? 5 : Array.IndexOf(Categories, detail.Episode.Assessment.Category);
        EpisodeOutcomeIndex = detail.Episode.Assessment == null ? 3 : Array.IndexOf(Outcomes, detail.Episode.Assessment.Outcome);
        var assessment = detail.Episode.Assessment == null ? this["episode_unassessed"] :
            this["category_" + detail.Episode.Assessment.Category] + " / " + this["outcome_" + detail.Episode.Assessment.Outcome] + " · " + Date(detail.Episode.Assessment.RecordedAt);
        EpisodeDetails = string.Join("\n", new[] {
            this["episode_id"] + ": " + detail.Episode.Id,
            this["episode_revision"] + ": " + Number(detail.Episode.Revision),
            this["episode_membership_revision"] + ": " + Number(detail.Episode.MembershipRevision),
            this["episode_created_at"] + ": " + Date(detail.Episode.CreatedAt),
            this["episode_updated_at"] + ": " + Date(detail.Episode.UpdatedAt),
            this["episode_resolved"] + ": " + Date(detail.ResolvedAt),
            this["episode_assessment"] + ": " + assessment
        });
        Changed();
    }

    private void ClearEpisodeDetail()
    {
        ++_episodePresentationVersion;
        CurrentEpisodeId = null;
        CurrentEpisodeRevision = 0;
        CurrentEpisodeMembershipRevision = 0;
        _episodeReviewRequired = false;
        EpisodeDetails = "";
        EpisodeMembers.Clear();
        EpisodeCategoryIndex = 5;
        EpisodeOutcomeIndex = 3;
        Changed();
    }
    public Task DeleteAsync() => Run(async token =>
    {
        if (CurrentId == null) return;
        ClearSummary();
        var result = await _service.CallAsync(new { type = "delete", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        ClearSelectedInsight();
        RenderMutationEffects(result);
        await RefreshCoreAsync(token);
    });
    public Task AnnotateAsync(string category, string outcome) => Run(async token =>
    {
        if (CurrentId == null) return;
        ClearSummary();
        var result = await _service.CallAsync(new { type = "annotate", id = CurrentId, category, outcome }, token);
        token.ThrowIfCancellationRequested();
        ClearQuestionCards();
        Render(result.GetProperty("insight"), true);
        await RefreshSummaryAsync(token);
    });
    public Task ClearAnnotationAsync() => Run(async token =>
    {
        if (CurrentId == null) return;
        ClearSummary();
        var result = await _service.CallAsync(new { type = "clear_annotation", id = CurrentId }, token);
        token.ThrowIfCancellationRequested();
        ClearQuestionCards();
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
            ClearQuestionCards();
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
            if (!models.IsSupported() || models.Scope != "declared_metadata_only" ||
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
                if (declaration.Kind is not ("claude_assistant_message" or "codex_session_metadata" or "codex_turn_context" or "codex_assistant_message" or "trajectory_metadata"))
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
    private void RenderMutationEffects(JsonElement response)
    {
        ClearQuestionCards();
        var effects = InsightMutationEffects.Decode(response);
        if (CurrentEpisodeId != null && effects.InvalidatedEpisodeIds.Contains(CurrentEpisodeId, StringComparer.Ordinal))
            ClearEpisodeDetail();
        MutationNotice = effects.InvalidatedEpisodeIds.Count == 0 ? "" :
            this["episode_invalidated_notice"] + "\n" + string.Join("\n", effects.InvalidatedEpisodeIds);
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
        MutationNotice = "";
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
        MutationNotice = "";
        Status = this["error"];
        Changed();
    }
    public void Cancel()
    {
        if (_closed) return;
        MutationNotice = "";
        ++_selectionVersion;
        ++_episodePresentationVersion;
        ClearQuestionCards();
        _pending?.Cancel();
        // Keep mutations serialized until the outstanding operation settles.
        Status = CancellationNotice;
        Changed();
    }
    public void Dispose()
    {
        if (_closed) return;
        _closed = true;
        MutationNotice = "";
        ++_selectionVersion;
        ++_episodePresentationVersion;
        ++_cardPresentationVersion;
        ++_generation;
        _pending?.Cancel();
    }
}
