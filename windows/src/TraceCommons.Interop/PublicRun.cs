using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace TraceCommons.Interop;

/// <summary>One of the core's value-and-word pairs (<c>PublicRunValueLabel</c>).</summary>
public sealed record PublicRunValueLabel(string Value, string Label);

/// <summary>A reuse permission the editor offers, in the core's words.</summary>
public sealed record PublicRunReuseChoice(string Permission, string Label, string Explanation);

/// <summary>
/// Every fixed word on an opened History record's detail and its public-run
/// editor, read from the core (<c>tc_public_run_copy</c>,
/// <c>public_run::copy::public_run_copy</c>). The Windows half of the Mac's
/// <c>PublicRunCopy</c> (PublicRunModels.swift:60-202) and of
/// <c>TCPublicRun.copyJSON</c> (TCPublicRun.swift:7-11).
/// </summary>
/// <remarks>
/// Decoded as strictly as the Mac decodes it: every field the Mac's struct
/// declares must be present, so a table the core no longer agrees with reads
/// as no table, and the detail draws nothing rather than a half-worded
/// screen (SessionDetailView.swift:30-32).
/// </remarks>
public sealed class PublicRunCopy
{
    /// <summary>The string fields the Mac's <c>PublicRunCopy</c> requires, by wire name.</summary>
    public static readonly IReadOnlyList<string> RequiredWords = new[]
    {
        "all_contributions", "view_session", "session_detail", "reading_record", "retry_read",
        "content_unavailable", "unavailable_value", "creator_report", "task", "no_task", "outcome",
        "outcome_unavailable", "decisive_correction", "no_correction", "supporting_evidence",
        "observed_in_version", "no_evidence", "contributed_version", "contribution_details",
        "processing_status", "permitted_uses", "no_permitted_uses", "permitted_uses_unavailable",
        "unrecognized_value", "next_action", "withdraw", "keep_contribution", "envelope_version",
        "consent_policy_version", "redaction_version", "publication_after_acceptance",
        "public_workflow", "published", "open_page", "edit_page", "unpublishing", "unpublish",
        "publication_disclosure", "page_title", "public_outcome", "reusable_instructions",
        "select_evidence", "publish_correction", "reuse_permission", "choose_permission",
        "source_public_run", "source_placeholder", "source_help", "cancel_edit", "review_page",
        "exact_public_preview", "observed_evidence", "use_workflow", "edit_draft", "publishing",
        "publish_page", "update_page", "validation_required", "validation_too_long",
        "validation_correction_too_long", "validation_evidence_required",
        "validation_permission_required", "validation_source_invalid", "session_account_required",
        "session_not_found", "session_unavailable", "publication_account_required",
        "publication_conflict", "publication_trace_not_found", "publication_source_not_found",
        "publication_invalid", "publication_unavailable", "credential_storage_warning",
        "contribution_status_unavailable", "credit_outcome_unavailable",
    };

    /// <summary>The value-and-word lists the Mac's struct requires.</summary>
    public static readonly IReadOnlyList<string> RequiredLists = new[]
    {
        "task_outcome_choices", "feedback_choices", "evidence_kind_choices",
        "contribution_status_choices", "history_status_labels", "credit_outcome_labels",
        "permitted_use_choices",
    };

    private static readonly Lazy<PublicRunCopy?> Core =
        new(() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_public_run_copy())));

    private readonly IReadOnlyDictionary<string, string> _words;
    private readonly IReadOnlyDictionary<string, IReadOnlyList<PublicRunValueLabel>> _lists;

    private PublicRunCopy(
        IReadOnlyDictionary<string, string> words,
        IReadOnlyDictionary<string, IReadOnlyList<PublicRunValueLabel>> lists,
        IReadOnlyList<PublicRunReuseChoice> reuse)
    {
        _words = words;
        _lists = lists;
        ReusePermissions = reuse;
    }

    /// <summary>The core's table, or null when it could not be read (the Mac's <c>model.publicRunCopy</c> nil).</summary>
    public static PublicRunCopy? Current => Core.Value;

    /// <summary>The reuse permissions, in the core's order. Nothing is preselected from these.</summary>
    public IReadOnlyList<PublicRunReuseChoice> ReusePermissions { get; }

    /// <summary>One of the required words, by wire name.</summary>
    public string this[string key] => _words[key];

    /// <summary>The table, or null when any field the Mac requires is missing or mistyped.</summary>
    public static PublicRunCopy? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            var words = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (string key in RequiredWords)
            {
                if (!root.TryGetProperty(key, out JsonElement value) || value.ValueKind != JsonValueKind.String)
                {
                    return null;
                }

                words[key] = value.GetString()!;
            }

            var lists = new Dictionary<string, IReadOnlyList<PublicRunValueLabel>>(StringComparer.Ordinal);
            foreach (string key in RequiredLists)
            {
                if (!root.TryGetProperty(key, out JsonElement array) || array.ValueKind != JsonValueKind.Array)
                {
                    return null;
                }

                var pairs = new List<PublicRunValueLabel>();
                foreach (JsonElement item in array.EnumerateArray())
                {
                    if (Text(item, "value") is not { } v || Text(item, "label") is not { } l)
                    {
                        return null;
                    }

                    pairs.Add(new PublicRunValueLabel(v, l));
                }

                lists[key] = pairs;
            }

            if (!root.TryGetProperty("reuse_permissions", out JsonElement reuse) || reuse.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var choices = new List<PublicRunReuseChoice>();
            foreach (JsonElement item in reuse.EnumerateArray())
            {
                if (Text(item, "permission") is not { } p
                    || !PublicRunEditor.Permissions.Contains(p)
                    || Text(item, "label") is not { } l
                    || Text(item, "explanation") is not { } e)
                {
                    return null;
                }

                choices.Add(new PublicRunReuseChoice(p, l, e));
            }

            return new PublicRunCopy(words, lists, choices);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static string? Text(JsonElement item, string key) =>
        item.ValueKind == JsonValueKind.Object
        && item.TryGetProperty(key, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private string? Find(string list, string? value) =>
        value is null ? null : _lists[list].FirstOrDefault(pair => pair.Value == value)?.Label;

    /// <summary>
    /// The word a History row shows for a status (<c>historyStatusLabel(for:)</c>,
    /// PublicRunModels.swift:176-178): the core's, or its unavailable word.
    /// </summary>
    public string HistoryStatusLabel(string value) =>
        Find("history_status_labels", value) ?? this["contribution_status_unavailable"];

    /// <summary>A permitted use's word, or the unrecognized word (PublicRunModels.swift:186-188).</summary>
    public string PermittedUseLabel(string value) =>
        Find("permitted_use_choices", value) ?? this["unrecognized_value"];

    /// <summary>The task outcome's word, or null (PublicRunModels.swift:190-193).</summary>
    public string? TaskOutcomeLabel(string? value) => Find("task_outcome_choices", value);

    /// <summary>The feedback's word, or null (PublicRunModels.swift:195-198).</summary>
    public string? FeedbackLabel(string? value) => Find("feedback_choices", value);

    /// <summary>An evidence kind's word; a kind the core does not name reads as unrecognized (:200-202).</summary>
    public string EvidenceKindLabel(string kind) =>
        Find("evidence_kind_choices", kind) ?? this["unrecognized_value"];

    /// <summary>The reuse choice for a permission, or null.</summary>
    public PublicRunReuseChoice? ReuseChoice(string? permission) =>
        ReusePermissions.FirstOrDefault(choice => choice.Permission == permission);
}

/// <summary>One evidence excerpt the record offers (<c>SessionEvidenceCandidate</c>).</summary>
public sealed record SessionEvidence(string EventId, string Kind, string Excerpt);

/// <summary>A public page's evidence line.</summary>
public sealed record PublicRunPageEvidence(string Excerpt);

/// <summary>A public run's published page (<c>PublicRunPage</c>, PublicRunModels.swift:285-322).</summary>
public sealed record PublicRunPage(
    string Slug,
    string Title,
    string OutcomeSummary,
    string? CorrectionExcerpt,
    string Workflow,
    string ReusePermission,
    IReadOnlyList<PublicRunPageEvidence> Evidence,
    string TaskSuccess,
    string ContributedVersion,
    int Version,
    string? PublicUrl,
    string? SourceSlug,
    string? CredentialWarning);

/// <summary>
/// <c>history_detail</c>'s answer, the fields the Mac's <c>SessionDetail</c>
/// reads (PublicRunModels.swift:347-381); the ones it requires are required.
/// </summary>
public sealed record SessionDetail(
    string? OwnerScopeSha256,
    bool? ContentUnavailable,
    string? Task,
    string? TaskSuccess,
    string? UserFeedback,
    string? HumanCorrection,
    IReadOnlyList<SessionEvidence> Evidence,
    string? ContributionStatus,
    IReadOnlyList<string>? PermittedUses,
    string ContributedVersion,
    string ConsentPolicyVersion,
    string RedactionPipelineVersion,
    PublicRunPage? Publication,
    int PublicationVersion,
    string? RetainedSourceSlug)
{
    /// <summary>The record was accepted (<c>accepted</c>, PublicRunModels.swift:364).</summary>
    public bool Accepted => ContributionStatus == "accepted";

    /// <summary>The detail, or null when a field the Mac requires is missing or mistyped.</summary>
    public static SessionDetail? Parse(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object
            || !root.TryGetProperty("evidence", out JsonElement evidence)
            || evidence.ValueKind != JsonValueKind.Array
            || Str(root, "contributed_version") is not { } contributed
            || Str(root, "consent_policy_version") is not { } consent
            || Str(root, "redaction_pipeline_version") is not { } redaction
            || !root.TryGetProperty("publication_version", out JsonElement version)
            || !version.TryGetInt32(out int publicationVersion))
        {
            return null;
        }

        var items = new List<SessionEvidence>();
        foreach (JsonElement item in evidence.EnumerateArray())
        {
            if (Str(item, "event_id") is not { } id || Str(item, "kind") is not { } kind || Str(item, "excerpt") is not { } excerpt)
            {
                return null;
            }

            items.Add(new SessionEvidence(id, kind, excerpt));
        }

        IReadOnlyList<string>? uses = null;
        if (root.TryGetProperty("permitted_uses", out JsonElement usesElement) && usesElement.ValueKind == JsonValueKind.Array)
        {
            var list = new List<string>();
            foreach (JsonElement use in usesElement.EnumerateArray())
            {
                if (use.ValueKind != JsonValueKind.String)
                {
                    return null;
                }

                list.Add(use.GetString()!);
            }

            uses = list;
        }

        PublicRunPage? publication = null;
        if (root.TryGetProperty("publication", out JsonElement page) && page.ValueKind == JsonValueKind.Object)
        {
            publication = PublicRunJson.Page(page);
            if (publication is null)
            {
                return null;
            }
        }

        bool? unavailable = root.TryGetProperty("content_unavailable", out JsonElement flag)
            && flag.ValueKind is JsonValueKind.True or JsonValueKind.False
                ? flag.GetBoolean()
                : null;

        return new SessionDetail(
            Str(root, "owner_scope_sha256"),
            unavailable,
            Str(root, "task"),
            Str(root, "task_success"),
            Str(root, "user_feedback"),
            Str(root, "human_correction"),
            items,
            Str(root, "contribution_status"),
            uses,
            contributed,
            consent,
            redaction,
            publication,
            publicationVersion,
            Str(root, "retained_source_slug"));
    }

    internal static string? Str(JsonElement item, string key) =>
        item.ValueKind == JsonValueKind.Object
        && item.TryGetProperty(key, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
}

/// <summary>Decoding for the publish and unpublish answers.</summary>
public static class PublicRunJson
{
    /// <summary>A published page, or null when a field the Mac requires is missing.</summary>
    public static PublicRunPage? Page(JsonElement page)
    {
        if (SessionDetail.Str(page, "slug") is not { } slug
            || SessionDetail.Str(page, "title") is not { } title
            || SessionDetail.Str(page, "outcome_summary") is not { } outcome
            || SessionDetail.Str(page, "workflow") is not { } workflow
            || SessionDetail.Str(page, "reuse_permission") is not { } permission
            || !PublicRunEditor.Permissions.Contains(permission)
            || SessionDetail.Str(page, "task_success") is not { } task
            || SessionDetail.Str(page, "contributed_version") is not { } contributed
            || SessionDetail.Str(page, "published_at") is null
            || !page.TryGetProperty("version", out JsonElement versionElement)
            || !versionElement.TryGetInt32(out int version)
            || !page.TryGetProperty("evidence", out JsonElement evidence)
            || evidence.ValueKind != JsonValueKind.Array
            || !page.TryGetProperty("variations", out JsonElement variations)
            || variations.ValueKind != JsonValueKind.Array)
        {
            return null;
        }

        var lines = new List<PublicRunPageEvidence>();
        foreach (JsonElement item in evidence.EnumerateArray())
        {
            if (SessionDetail.Str(item, "excerpt") is not { } excerpt)
            {
                return null;
            }

            lines.Add(new PublicRunPageEvidence(excerpt));
        }

        string? source = page.TryGetProperty("source", out JsonElement link) && link.ValueKind == JsonValueKind.Object
            ? SessionDetail.Str(link, "slug")
            : null;

        return new PublicRunPage(
            slug, title, outcome, SessionDetail.Str(page, "correction_excerpt"), workflow, permission, lines,
            task, contributed, version, SessionDetail.Str(page, "public_url"), source,
            SessionDetail.Str(page, "credential_warning"));
    }

    /// <summary>
    /// <c>unpublish_public_run</c>'s answer: the publication version the next
    /// publish must expect, and any credential warning (PublicRunModels.swift:324-334).
    /// </summary>
    public static (int ExpectedPublicationVersion, string? CredentialWarning)? Unpublished(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object
            || !root.TryGetProperty("unpublished", out JsonElement done)
            || done.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || !root.TryGetProperty("expected_publication_version", out JsonElement version)
            || !version.TryGetInt32(out int expected))
        {
            return null;
        }

        return (expected, SessionDetail.Str(root, "credential_warning"));
    }
}

/// <summary>The draft the editor's validation returns (<c>PublicRunDraftInput</c>).</summary>
public sealed record PublicRunDraft(
    string Title,
    string OutcomeSummary,
    string? CorrectionExcerpt,
    string Workflow,
    string ReusePermission,
    IReadOnlyList<SessionEvidence> Evidence,
    string? SourceSlug,
    string RawJson);

/// <summary>What the editor holds, sent to the core for validation.</summary>
public sealed record PublicRunEditorInput(
    string Title,
    string OutcomeSummary,
    string? CorrectionExcerpt,
    string Workflow,
    string? ReusePermission,
    IReadOnlyList<(string EventId, string Excerpt)> Evidence,
    string Source);

/// <summary>
/// The public-run editor's rules that are not words: the field limits it
/// counts against, at most four evidence items, validation by the core alone
/// (<c>tc_public_run_validate_editor</c>), restoring a published page into the
/// editor, and the publish request. The Mac's <c>PublicRunEditor</c>
/// (SessionDetailView.swift:179-532).
/// </summary>
public static class PublicRunEditor
{
    /// <summary>The two permissions the protocol names (<c>PublicRunReusePermission</c>).</summary>
    public static readonly IReadOnlyList<string> Permissions = new[] { "cc_by_4_0", "cc0_1_0" };

    /// <summary>The counters' maxima beside the title, outcome and workflow fields (SessionDetailView.swift:283,289,295).</summary>
    public const int TitleLimit = 100;

    /// <inheritdoc cref="TitleLimit"/>
    public const int OutcomeLimit = 600;

    /// <inheritdoc cref="TitleLimit"/>
    public const int WorkflowLimit = 4_000;

    /// <summary>At most this many evidence items are selected (SessionDetailView.swift:318-320, 492-494).</summary>
    public const int EvidenceLimit = 4;

    /// <summary>A field's counter, <c>count/maximum</c> (SessionDetailView.swift:478-486).</summary>
    public static string Counter(string text, int maximum) =>
        $"{new System.Globalization.StringInfo(text ?? string.Empty).LengthInTextElements}/{maximum}";

    /// <summary>
    /// The selection after a press on one evidence item: added only while
    /// fewer than four are chosen, removed always.
    /// </summary>
    public static IReadOnlySet<string> Toggle(IReadOnlySet<string> selected, string eventId, bool on)
    {
        var next = new HashSet<string>(selected, StringComparer.Ordinal);
        if (on)
        {
            if (next.Count < EvidenceLimit)
            {
                next.Add(eventId);
            }
        }
        else
        {
            next.Remove(eventId);
        }

        return next;
    }

    /// <summary>Whether an unselected item can still be chosen (disabled at four, SessionDetailView.swift:318-321).</summary>
    public static bool CanSelect(IReadOnlySet<string> selected, string eventId) =>
        selected.Contains(eventId) || selected.Count < EvidenceLimit;

    /// <summary>
    /// The evidence a published page restores, matched by excerpt in the
    /// record's order, each record item used at most once
    /// (<c>populate(from:)</c>, SessionDetailView.swift:520-528).
    /// </summary>
    public static IReadOnlySet<string> Restore(IReadOnlyList<SessionEvidence> record, IReadOnlyList<PublicRunPageEvidence> published)
    {
        var remaining = record.ToList();
        var restored = new HashSet<string>(StringComparer.Ordinal);
        foreach (PublicRunPageEvidence line in published)
        {
            int index = remaining.FindIndex(item => item.Excerpt == line.Excerpt);
            if (index >= 0)
            {
                restored.Add(remaining[index].EventId);
                remaining.RemoveAt(index);
            }
        }

        return restored;
    }

    /// <summary>The editor input as the core's validation reads it.</summary>
    public static string InputJson(PublicRunEditorInput input)
    {
        var evidence = new JsonArray();
        foreach ((string id, string excerpt) in input.Evidence)
        {
            evidence.Add(new JsonObject { ["event_id"] = id, ["excerpt"] = excerpt });
        }

        return new JsonObject
        {
            ["title"] = input.Title,
            ["outcome_summary"] = input.OutcomeSummary,
            ["correction_excerpt"] = input.CorrectionExcerpt,
            ["workflow"] = input.Workflow,
            ["reuse_permission"] = input.ReusePermission,
            ["evidence"] = evidence,
            ["source"] = input.Source,
        }.ToJsonString();
    }

    /// <summary>
    /// The core's verdict on the editor: a draft to review, or the core's
    /// problem line. Null when the core gave no readable answer, which the
    /// editor shows as the core's unavailable line (SessionDetailView.swift:442-445).
    /// </summary>
    public static (PublicRunDraft? Draft, string? Error)? Validate(PublicRunEditorInput input)
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_public_run_validate_editor(InputJson(input)));
        return ParseValidation(json);
    }

    /// <summary>The validation answer, decoded.</summary>
    public static (PublicRunDraft? Draft, string? Error)? ParseValidation(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            string? error = SessionDetail.Str(root, "error");
            if (!root.TryGetProperty("draft", out JsonElement draft) || draft.ValueKind != JsonValueKind.Object)
            {
                return (null, error);
            }

            if (SessionDetail.Str(draft, "title") is not { } title
                || SessionDetail.Str(draft, "outcome_summary") is not { } outcome
                || SessionDetail.Str(draft, "workflow") is not { } workflow
                || SessionDetail.Str(draft, "reuse_permission") is not { } permission
                || !draft.TryGetProperty("evidence", out JsonElement evidence)
                || evidence.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var items = new List<SessionEvidence>();
            foreach (JsonElement item in evidence.EnumerateArray())
            {
                if (SessionDetail.Str(item, "event_id") is not { } id || SessionDetail.Str(item, "excerpt") is not { } excerpt)
                {
                    return null;
                }

                items.Add(new SessionEvidence(id, string.Empty, excerpt));
            }

            return (new PublicRunDraft(
                title, outcome, SessionDetail.Str(draft, "correction_excerpt"), workflow, permission, items,
                SessionDetail.Str(draft, "source_slug"), draft.GetRawText()), error);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// <c>publish_public_run</c>'s parameters (DaemonClient.swift:312-334):
    /// the reviewed draft as the core returned it, the detail's task outcome
    /// and contributed version, and the publication version it was read at.
    /// Null when the detail has no task outcome, as the Mac then sends nothing
    /// (AppModel.swift:4263-4265).
    /// </summary>
    public static string? PublishParams(string submissionId, PublicRunDraft draft, SessionDetail detail)
    {
        if (detail.TaskSuccess is not { } task)
        {
            return null;
        }

        return new JsonObject
        {
            ["submission_id"] = submissionId,
            ["draft"] = JsonNode.Parse(draft.RawJson),
            ["task_success"] = task,
            ["contributed_version"] = detail.ContributedVersion,
            ["expected_publication_version"] = detail.PublicationVersion,
        }.ToJsonString();
    }

    /// <summary>The submission id params <c>history_detail</c> and <c>unpublish_public_run</c> take.</summary>
    public static string SubmissionParams(string submissionId) =>
        new JsonObject { ["submission_id"] = submissionId }.ToJsonString();
}

/// <summary>The core's sentences for fixed error labels (<c>TCPublicRun</c>, TCPublicRun.swift:13-27).</summary>
public static class PublicRunErrors
{
    /// <summary>The line for a failed <c>history_detail</c> read.</summary>
    public static string SessionDetailLine(string? label) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_session_detail_error_line(label ?? string.Empty)) ?? string.Empty;

    /// <summary>The line for a failed publish or unpublish, or a credential warning.</summary>
    public static string PublicationLine(string? label) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_public_run_error_line(label ?? string.Empty)) ?? string.Empty;
}

/// <summary>
/// Which statuses a contribution can still be withdrawn from, and the
/// gates the detail draws its editor behind. The Mac's
/// <c>ContributionStatusPresentation</c> (PublicRunModels.swift:205-227) and
/// <c>SkillLearningGate</c> (SkillLearningView.swift:666-680).
/// </summary>
public static class ContributionGate
{
    private static readonly HashSet<string> OpenValues = new(StringComparer.Ordinal)
    {
        "submitted", "received", "accepted", "quarantined", "awaiting_pii_backstop", "rejected",
    };

    /// <summary>Closed, or a status this build does not recognize; null is not terminal.</summary>
    public static bool IsTerminal(string? status) => status is not null && !OpenValues.Contains(status);

    /// <summary>The withdrawal finished: a terminal record, or a withdrawal this session completed.</summary>
    public static bool WithdrawalCompleted(string recordStatus, bool withdrawnHere) =>
        IsTerminal(recordStatus) || withdrawnHere;

    /// <summary>The contribution is withdrawn or closed.</summary>
    public static bool IsWithdrawn(SessionDetail detail, string recordStatus, bool withdrawalCompleted) =>
        withdrawalCompleted || IsTerminal(detail.ContributionStatus ?? recordStatus);

    /// <summary>The editor is offered: accepted, with a task outcome, and not withdrawn.</summary>
    public static bool IsActive(SessionDetail detail, string recordStatus, bool withdrawalCompleted) =>
        detail.Accepted && detail.TaskSuccess is not null && !IsWithdrawn(detail, recordStatus, withdrawalCompleted);
}

/// <summary>
/// The opened History row's inspector: which row it shows, and whether its
/// own tag says the status. The Mac's <c>HistorySelection</c>
/// (HistoryInspector.swift:136-158) and <c>HeldExplanations</c> (:180-194).
/// </summary>
public static class HistoryInspection
{
    /// <summary>
    /// The row the inspector shows: only on History's page, only while the
    /// opened row is still in the list.
    /// </summary>
    public static T? Opened<T>(string? submissionId, bool onHistory, IEnumerable<T>? rows, Func<T, string> id)
        where T : class
    {
        if (!onHistory || string.IsNullOrEmpty(submissionId) || rows is null)
        {
            return null;
        }

        return rows.FirstOrDefault(row => id(row) == submissionId);
    }

    /// <summary>The inspector's own tag says the status until the detail's overview is drawn.</summary>
    public static bool TagsStatus(bool recordResolves, SessionDetail? detail) => !recordResolves || detail is null;

    /// <summary>
    /// The server's explanation lines, distinct, first-seen order, without any
    /// line carrying an opaque digest.
    /// </summary>
    public static IReadOnlyList<string> ExplanationLines(IEnumerable<IEnumerable<string>> explanations)
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var lines = new List<string>();
        foreach (IEnumerable<string> record in explanations)
        {
            foreach (string line in record)
            {
                if (!line.Contains("sha256:", StringComparison.Ordinal) && seen.Add(line))
                {
                    lines.Add(line);
                }
            }
        }

        return lines;
    }

    /// <summary>
    /// The status tag's tone: accepted reads as done; held and submitted as
    /// waiting; anything else neutral (<c>HomeFormat.tone</c>, HomeViews.swift:2270-2276).
    /// </summary>
    public static StatusTone Tone(string? status) => status switch
    {
        "accepted" => StatusTone.On,
        "submitted" or "processing" or "quarantined" => StatusTone.Ask,
        _ => StatusTone.Neutral,
    };

    /// <summary>
    /// The status word: the core's for the status, the core's unavailable
    /// word for none, and no word at all without the core's table
    /// (<c>HomeFormat.historyStatusLabel</c>, HomeViews.swift:2262-2266).
    /// </summary>
    public static string? StatusWord(PublicRunCopy? copy, string? status)
    {
        if (copy is null)
        {
            return null;
        }

        return string.IsNullOrEmpty(status)
            ? copy["contribution_status_unavailable"]
            : copy.HistoryStatusLabel(status);
    }
}

/// <summary>The tones a status tag is drawn in (<c>GlassTag.Tone</c>).</summary>
public enum StatusTone
{
    Neutral,
    On,
    Ask,
}

/// <summary>
/// An opened record's detail and the outcome of its reads and publication
/// requests: the Mac's <c>AppModel</c> transitions for one record
/// (<c>loadSessionDetail</c>, <c>publishPublicRun</c>,
/// <c>unpublishPublicRun</c>, AppModel.swift:4195-4327), so the inspector's
/// rules are tested off Windows. Every line is the core's.
/// </summary>
public sealed class HistoryDetailState
{
    /// <summary>
    /// The labels after which nothing read under the account may stay on
    /// screen (AppModel.swift:4230-4237): the account ended, the record is not
    /// this account's, or the account changed under the read.
    /// </summary>
    public static readonly IReadOnlyList<string> AccountEndingLabels = new[]
    {
        "account-session-required", "session-detail-not-found", "session-owner-changed",
    };

    private string? _ownerScope;

    /// <summary>The detail held, kept across a failed reload unless the failure ends the account.</summary>
    public SessionDetail? Detail { get; private set; }

    /// <summary>The core's line for the last failed read; null after a good one.</summary>
    public string? ReadError { get; private set; }

    /// <summary>The core's line for the last failed or warned publication request.</summary>
    public string? PublicationError { get; private set; }

    /// <summary>A read is starting: its earlier error goes.</summary>
    public void ReadStarted() => ReadError = null;

    /// <summary>
    /// A read answered. A detail under a different account than the one held
    /// replaces everything held (<c>reconcileAccountOwnedContent</c>, :4243-4249).
    /// </summary>
    public void ReadSucceeded(SessionDetail detail)
    {
        ArgumentNullException.ThrowIfNull(detail);
        if (detail.OwnerScopeSha256 is { } scope)
        {
            if (_ownerScope is { } previous && previous != scope)
            {
                Clear();
            }

            _ownerScope = scope;
        }

        Detail = detail;
        ReadError = null;
    }

    /// <summary>A read failed: the core's line, and an account-ending label clears what was held.</summary>
    public void ReadFailed(string? label)
    {
        if (label is not null && AccountEndingLabels.Contains(label))
        {
            Clear();
        }

        ReadError = PublicRunErrors.SessionDetailLine(label);
    }

    /// <summary>
    /// A read made under an earlier account answered after a newer one
    /// (<see cref="AccountOwnedContent.ReadSucceeded"/> refused it): nothing
    /// from it is shown, and the core's line says so, with Retry
    /// (AppModel.swift:4219-4225). What is held stays.
    /// </summary>
    public void ReadStale() => ReadError = PublicRunErrors.SessionDetailLine("session-owner-changed");

    /// <summary>The window's account content was cleared: this record's goes with it.</summary>
    public void AccountCleared() => Clear();

    /// <summary>A publication request is starting: its earlier error goes.</summary>
    public void PublicationStarted() => PublicationError = null;

    /// <summary>Published: the record now carries the page at its version; a credential warning is said.</summary>
    public void Published(PublicRunPage page)
    {
        ArgumentNullException.ThrowIfNull(page);
        if (Detail is { } detail)
        {
            Detail = detail with { Publication = page, PublicationVersion = page.Version };
        }

        PublicationError = page.CredentialWarning is { } warning ? PublicRunErrors.PublicationLine(warning) : null;
    }

    /// <summary>Unpublished: back to private, at the version the next publish must expect.</summary>
    public void Unpublished(int expectedPublicationVersion, string? credentialWarning)
    {
        if (Detail is { } detail)
        {
            Detail = detail with { Publication = null, PublicationVersion = expectedPublicationVersion };
        }

        PublicationError = credentialWarning is null ? null : PublicRunErrors.PublicationLine(credentialWarning);
    }

    /// <summary>A refused publish or unpublish: the record stays exactly as it was; the core's line is said.</summary>
    public void PublicationFailed(string? label) => PublicationError = PublicRunErrors.PublicationLine(label);

    private void Clear()
    {
        _ownerScope = null;
        Detail = null;
        ReadError = null;
        PublicationError = null;
    }
}
