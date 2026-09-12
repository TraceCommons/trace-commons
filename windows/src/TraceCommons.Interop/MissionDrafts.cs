using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

public interface ILocalMissionDrafts
{
    Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken);
}

public sealed class LocalMissionDrafts : ILocalMissionDrafts
{
    private static readonly HashSet<string> PublicErrors = new(StringComparer.Ordinal) {
        "mission-draft-request-too-large", "mission-draft-request-null", "mission-draft-request-invalid-utf8",
        "mission-draft-request-invalid", "mission-draft-response-too-large", "mission-draft-too-large",
        "mission-draft-invalid-json", "mission-draft-version-unsupported", "mission-draft-field-invalid",
        "mission-draft-url-invalid", "mission-draft-budget-invalid", "mission-draft-file-unreadable",
        "mission-draft-file-not-regular", "mission-draft-store-unavailable", "mission-draft-store-busy",
        "mission-draft-store-unreadable", "mission-draft-store-invalid", "mission-draft-store-version-unsupported",
        "mission-draft-store-full", "mission-draft-store-write-failed",
        "mission-draft-store-requires-private-directory", "mission-draft-not-found",
        "mission-draft-operation-failed"
    };
    private static readonly SemaphoreSlim Calls = new(1, 1);
    private readonly string? _storeDirectory;
    public LocalMissionDrafts(string? storeDirectory = null) => _storeDirectory = storeDirectory;

    public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(new { store_dir = _storeDirectory, operation });
        if (bytes.Length > 65536) throw new InvalidOperationException("mission-draft-request-too-large");
        return Task.Run(async () =>
        {
            await Calls.WaitAsync(cancellationToken).ConfigureAwait(false);
            try { cancellationToken.ThrowIfCancellationRequested(); return Call(bytes); }
            finally { Calls.Release(); }
        }, cancellationToken).WaitAsync(cancellationToken);
    }

    private static JsonElement Call(byte[] bytes)
    {
        IntPtr response = NativeMethods.tc_mission_drafts_call(bytes, (UIntPtr)bytes.Length, out IntPtr error);
        try
        {
            if (error != IntPtr.Zero || response == IntPtr.Zero)
            {
                string? code = error == IntPtr.Zero ? null : NativeMethods.BorrowedString(error);
                if (code != null && PublicErrors.Contains(code)) throw new MissionDraftServiceException(code);
                throw new InvalidOperationException("mission-draft-operation-failed");
            }
            string json = NativeMethods.BorrowedString(response)
                ?? throw new InvalidOperationException("mission-draft-response-invalid");
            using var document = JsonDocument.Parse(json);
            return document.RootElement.Clone();
        }
        finally
        {
            if (response != IntPtr.Zero) NativeMethods.tc_string_free(response);
            if (error != IntPtr.Zero) NativeMethods.tc_string_free(error);
        }
    }
}

public sealed class MissionDraftServiceException : InvalidOperationException
{
    public string Code { get; }
    public MissionDraftServiceException(string code) : base(code) => Code = code;
}

public static class MissionDraftDecoding
{
    public static readonly string[] RequiredCopyKeys = ["title","intro","empty","choose_file","file_selected","import","refresh","refreshed","show","delete","delete_confirm_title","delete_confirm","added","duplicate","deleted","proposal_sha256","source_count","proposal_title","source_claim","task","starting_artifact","starting_artifact_digest","source_urls","success_criteria","required_evidence","allowed_models","allowed_tools","proposed_budget","duration_seconds","input_tokens","output_tokens","author_unverified","evaluator_unverified","rubric_version","needs_curator_review","review_notice","authority_notice","display_notice","working","cancel","close","error"];
    private static readonly JsonSerializerOptions Strict = new() {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow
    };

    public static IReadOnlyDictionary<string, string> Copy(JsonElement response)
    {
        var copy = Decode<MissionCopyResponse>(response, "copy").Copy;
        foreach (string key in RequiredCopyKeys)
            if (!copy.TryGetValue(key, out string? value) || string.IsNullOrWhiteSpace(value)) Invalid();
        return copy;
    }

    public static IReadOnlyList<MissionDraftSummary> List(JsonElement response)
    {
        var drafts = Decode<MissionListResponse>(response, "list").Drafts;
        if (drafts.Count > 256) Invalid();
        foreach (var draft in drafts) ValidateSummary(draft);
        return drafts;
    }

    public static MissionDraftImport Import(JsonElement response)
    {
        var draft = Decode<MissionImportResponse>(response, "import").Draft;
        ValidateId(draft.Id); ValidateReview(draft.Review, draft.Id); return draft;
    }

    public static StoredMissionDraft Show(JsonElement response)
    {
        var draft = Decode<MissionShowResponse>(response, "show").Draft;
        ValidateId(draft.Id); ValidateReview(draft.Review, draft.Id);
        if (draft.Proposal.SchemaVersion != 1 || draft.Proposal.SourceUrls.Count is < 1 or > 8) Invalid();
        return draft;
    }

    public static MissionDraftDelete Delete(JsonElement response)
    {
        var draft = Decode<MissionDeleteResponse>(response, "delete").Draft;
        ValidateId(draft.Id); if (!draft.Deleted) Invalid(); return draft;
    }

    private static void ValidateSummary(MissionDraftSummary draft)
    { ValidateId(draft.Id); if (draft.SourceCount is < 1 or > 8 || draft.Status != "needs_curator_review") Invalid(); }
    private static void ValidateReview(MissionDraftReview review, string id)
    {
        if (review.SchemaVersion != 1 || review.ProposalSha256 != id || review.Status != "needs_curator_review"
            || review.PublicationAuthorized || review.ExternalSourcesVerified || review.RequiredReviews.Count == 0) Invalid();
    }
    private static void ValidateId(string id)
    {
        if (id.Length != 64) Invalid();
        foreach (char value in id) if (!(value is >= '0' and <= '9' or >= 'a' and <= 'f')) Invalid();
    }
    private static void Invalid() => throw new InvalidOperationException("mission-draft-response-invalid");

    private static T Decode<T>(JsonElement response, string expected) where T : MissionResponse
    {
        T value = response.Deserialize<T>(Strict) ?? throw new InvalidOperationException("mission-draft-response-invalid");
        if (value.Type != expected) throw new InvalidOperationException("mission-draft-response-invalid");
        return value;
    }
}

public abstract class MissionResponse { public required string Type { get; init; } }
public sealed class MissionCopyResponse : MissionResponse { public required Dictionary<string,string> Copy { get; init; } }
public sealed class MissionListResponse : MissionResponse { public required List<MissionDraftSummary> Drafts { get; init; } }
public sealed class MissionImportResponse : MissionResponse { public required MissionDraftImport Draft { get; init; } }
public sealed class MissionShowResponse : MissionResponse { public required StoredMissionDraft Draft { get; init; } }
public sealed class MissionDeleteResponse : MissionResponse { public required MissionDraftDelete Draft { get; init; } }
public sealed class MissionDraftSummary { public required string Id { get; init; } public required ulong SourceCount { get; init; } public required string Status { get; init; } }
public sealed class MissionDraftImport { public required string Id { get; init; } public required MissionDraftReview Review { get; init; } public required bool Inserted { get; init; } }
public sealed class MissionDraftDelete { public required string Id { get; init; } public required bool Deleted { get; init; } }
public sealed class StoredMissionDraft { public required string Id { get; init; } public required MissionProposal Proposal { get; init; } public required MissionDraftReview Review { get; init; } }
public sealed class MissionDraftReview { public required uint SchemaVersion { get; init; } public required string ProposalSha256 { get; init; } public required string Status { get; init; } public required bool PublicationAuthorized { get; init; } public required bool ExternalSourcesVerified { get; init; } public required List<string> RequiredReviews { get; init; } }
public sealed class MissionProposal { public required uint SchemaVersion { get; init; } public required string AuthorId { get; init; } public required string Title { get; init; } public required List<string> SourceUrls { get; init; } public required string ClaimToTest { get; init; } public required string Task { get; init; } public required StartingArtifact StartingArtifact { get; init; } public required string EvaluatorId { get; init; } public required string RubricVersion { get; init; } public required List<string> SuccessCriteria { get; init; } public required List<string> RequiredEvidence { get; init; } public required List<string> AllowedModels { get; init; } public required List<string> AllowedTools { get; init; } public required MissionBudget Budget { get; init; } }
public sealed class StartingArtifact { public required string Url { get; init; } public required string Sha256 { get; init; } }
public sealed class MissionBudget { public required uint MaxDurationSeconds { get; init; } public required ulong MaxInputTokens { get; init; } public required ulong MaxOutputTokens { get; init; } }
