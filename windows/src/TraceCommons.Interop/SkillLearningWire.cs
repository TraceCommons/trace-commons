using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace TraceCommons.Interop;

/// <summary>
/// Every fixed word on the tested-skill surface, read from the core
/// (<c>tc_skill_learning_copy</c>, <c>skill_loop::copy::SkillLearningCopy</c>).
/// The Windows half of the Mac's <c>SkillLearningCopy</c>
/// (SkillLearningModels.swift:6-81).
/// </summary>
/// <remarks>
/// Decoded as strictly as the Mac's <c>Decodable</c> decodes it: every field
/// must be present and a string, or there is no table at all, and the
/// surface draws nothing rather than a half-worded screen.
/// </remarks>
public sealed class SkillLearningCopy
{
    /// <summary>
    /// The wire names the Mac's struct requires, in its order
    /// (SkillLearningModels.swift:7-73, snake-cased as its
    /// <c>convertFromSnakeCase</c> decoder reads them).
    /// </summary>
    public static readonly IReadOnlyList<string> RequiredWords = new[]
    {
        "heading", "promise", "supported_family", "learn_action", "learning", "candidate_heading",
        "generated_source", "name", "applicability", "procedure", "source_evidence", "manual_instruction",
        "test_contract", "contract_summary_format", "review_action", "reviewing", "exact_package", "digest",
        "evaluation_disclosure", "review_budget_format", "approve_and_test", "edit_skill", "testing", "results",
        "passed_gate", "failed_gate", "repository_plans", "skill_applicability", "regressions", "no_regressions",
        "inspect_runs", "model_and_budget", "model_budget_format", "open_fixture_source", "model_output", "edits",
        "commands", "checks", "baseline", "simple_instruction", "candidate_skill", "passed", "failed", "preparing",
        "review_install", "retry_install", "install_preview", "install_disclosure", "persistent_writes",
        "target_path", "skill_file", "ownership_marker", "marker_digest", "exact_marker", "coding_tool",
        "install_action", "installing", "installed", "rollback", "rolling_back", "rollback_disclosure",
        "rollback_incomplete", "rollback_confirm_title", "rollback_confirm_body", "rollback_keep", "unavailable",
    };

    private static readonly Lazy<SkillLearningCopy?> Core = new(() => Parse(SkillLearningNative.CopyJson()));

    private readonly IReadOnlyDictionary<string, string> _words;

    private SkillLearningCopy(IReadOnlyDictionary<string, string> words) => _words = words;

    /// <summary>The core's table, or null when it could not be read (the Mac's <c>model.skillLearningCopy</c> nil).</summary>
    public static SkillLearningCopy? Current => Core.Value;

    /// <summary>One of the required words, by wire name.</summary>
    public string this[string key] => _words[key];

    /// <summary>
    /// The table, or null when any field is missing or not a string
    /// (<c>SkillLearningCopy.decode(fromJSON:)</c>, SkillLearningModels.swift:75-80).
    /// </summary>
    public static SkillLearningCopy? Parse(string? json)
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

            return new SkillLearningCopy(words);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    public string Heading => _words["heading"];
    public string Promise => _words["promise"];
    public string SupportedFamily => _words["supported_family"];
    public string LearnAction => _words["learn_action"];
    public string Learning => _words["learning"];
    public string CandidateHeading => _words["candidate_heading"];
    public string GeneratedSource => _words["generated_source"];
    public string Name => _words["name"];
    public string Applicability => _words["applicability"];
    public string Procedure => _words["procedure"];
    public string SourceEvidence => _words["source_evidence"];
    public string ManualInstruction => _words["manual_instruction"];
    public string TestContract => _words["test_contract"];
    public string ContractSummaryFormat => _words["contract_summary_format"];
    public string ReviewAction => _words["review_action"];
    public string Reviewing => _words["reviewing"];
    public string ExactPackage => _words["exact_package"];
    public string Digest => _words["digest"];
    public string EvaluationDisclosure => _words["evaluation_disclosure"];
    public string ReviewBudgetFormat => _words["review_budget_format"];
    public string ApproveAndTest => _words["approve_and_test"];
    public string EditSkill => _words["edit_skill"];
    public string Testing => _words["testing"];
    public string Results => _words["results"];
    public string PassedGate => _words["passed_gate"];
    public string FailedGate => _words["failed_gate"];
    public string RepositoryPlans => _words["repository_plans"];
    public string SkillApplicability => _words["skill_applicability"];
    public string Regressions => _words["regressions"];
    public string NoRegressions => _words["no_regressions"];
    public string InspectRuns => _words["inspect_runs"];
    public string ModelAndBudget => _words["model_and_budget"];
    public string ModelBudgetFormat => _words["model_budget_format"];
    public string OpenFixtureSource => _words["open_fixture_source"];
    public string ModelOutput => _words["model_output"];
    public string Edits => _words["edits"];
    public string Commands => _words["commands"];
    public string Checks => _words["checks"];
    public string Baseline => _words["baseline"];
    public string SimpleInstruction => _words["simple_instruction"];
    public string CandidateSkill => _words["candidate_skill"];
    public string Passed => _words["passed"];
    public string Failed => _words["failed"];
    public string Preparing => _words["preparing"];
    public string ReviewInstall => _words["review_install"];
    public string RetryInstall => _words["retry_install"];
    public string InstallPreview => _words["install_preview"];
    public string InstallDisclosure => _words["install_disclosure"];
    public string PersistentWrites => _words["persistent_writes"];
    public string TargetPath => _words["target_path"];
    public string SkillFile => _words["skill_file"];
    public string OwnershipMarker => _words["ownership_marker"];
    public string MarkerDigest => _words["marker_digest"];
    public string ExactMarker => _words["exact_marker"];
    public string CodingTool => _words["coding_tool"];
    public string InstallAction => _words["install_action"];
    public string Installing => _words["installing"];
    public string Installed => _words["installed"];
    public string Rollback => _words["rollback"];
    public string RollingBack => _words["rolling_back"];
    public string RollbackDisclosure => _words["rollback_disclosure"];
    public string RollbackIncomplete => _words["rollback_incomplete"];

    /// <summary>The rollback's confirmation (owner ruling, 2026-10-06; SkillLearningModels.swift:69-72).</summary>
    public string RollbackConfirmTitle => _words["rollback_confirm_title"];
    public string RollbackConfirmBody => _words["rollback_confirm_body"];
    public string RollbackKeep => _words["rollback_keep"];
    public string Unavailable => _words["unavailable"];

    /// <summary>
    /// Fills one of the core's positional templates (<c>%1$d</c>, <c>%3$@</c>),
    /// as Swift's <c>String(format:locale:)</c> fills them on the Mac
    /// (SkillLearningView.swift:211-219, :291-299, :354-362): integers as plain
    /// digits, strings as given. A hole with no argument is left as it is.
    /// </summary>
    public static string Format(string template, params object[] args) =>
        System.Text.RegularExpressions.Regex.Replace(template, @"%(\d+)\$[d@s]", match =>
        {
            int index = int.Parse(match.Groups[1].Value, System.Globalization.CultureInfo.InvariantCulture) - 1;
            return index >= 0 && index < args.Length
                ? Convert.ToString(args[index], System.Globalization.CultureInfo.InvariantCulture) ?? string.Empty
                : match.Value;
        });
}

/// <summary>
/// The three C ABI calls of the tested-skill surface, and nothing else: the
/// Mac's <c>TCSkillLearning</c> (TCSkillLearning.swift:9-31). Everything
/// that can be tested without the cdylib takes these as values instead.
/// </summary>
public static class SkillLearningNative
{
    /// <summary>The core's copy table as JSON (<c>copyJSON</c>, TCSkillLearning.swift:10-14).</summary>
    public static string? CopyJson() => NativeMethods.TakeOwnedString(NativeMethods.tc_skill_learning_copy());

    /// <summary>The core's sentence for a fixed daemon refusal label (<c>errorLine</c>, TCSkillLearning.swift:16-22).</summary>
    public static string? ErrorLine(string label) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_skill_learning_error_line(label));

    /// <summary>The core's verdict on an editor draft, as JSON (<c>validateDraftJSON</c>, TCSkillLearning.swift:24-30).</summary>
    public static string? ValidateDraftJson(string inputJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_skill_draft_validate(inputJson));

    /// <summary>
    /// The draft validated by the core, or null when the core could not
    /// answer (SkillLearningView.swift:194-204: encode the draft, validate,
    /// decode the verdict).
    /// </summary>
    public static SkillDraftValidation? ValidateDraft(SkillDraft draft) =>
        SkillDraftValidation.Parse(ValidateDraftJson(draft.ToJson()));
}

/// <summary>
/// The strict reader behind every tested-skill wire model: a required field
/// that is missing, null or mistyped refuses the whole value, and an optional
/// one that is present with the wrong type refuses it too, as Swift's
/// <c>decode</c> and <c>decodeIfPresent</c> do.
/// </summary>
internal static class SkillWire
{
    internal sealed class Refused : Exception
    {
    }

    internal static T? Decode<T>(JsonElement element, Func<JsonElement, T> read)
        where T : class
    {
        try
        {
            return read(element);
        }
        catch (Refused)
        {
            return null;
        }
    }

    internal static JsonElement Obj(JsonElement element) =>
        element.ValueKind == JsonValueKind.Object ? element : throw new Refused();

    private static JsonElement Field(JsonElement o, string key) =>
        Obj(o).TryGetProperty(key, out JsonElement value) ? value : throw new Refused();

    private static bool Absent(JsonElement o, string key, out JsonElement value) =>
        !Obj(o).TryGetProperty(key, out value) || value.ValueKind == JsonValueKind.Null;

    internal static string Str(JsonElement o, string key)
    {
        JsonElement value = Field(o, key);
        return value.ValueKind == JsonValueKind.String ? value.GetString()! : throw new Refused();
    }

    internal static string? OptStr(JsonElement o, string key)
    {
        if (Absent(o, key, out JsonElement value))
        {
            return null;
        }

        return value.ValueKind == JsonValueKind.String ? value.GetString() : throw new Refused();
    }

    internal static int Int(JsonElement o, string key)
    {
        JsonElement value = Field(o, key);
        return value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out int number) ? number : throw new Refused();
    }

    internal static int? OptInt(JsonElement o, string key)
    {
        if (Absent(o, key, out JsonElement value))
        {
            return null;
        }

        return value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out int number) ? number : throw new Refused();
    }

    internal static bool Bool(JsonElement o, string key)
    {
        JsonElement value = Field(o, key);
        return value.ValueKind is JsonValueKind.True or JsonValueKind.False ? value.GetBoolean() : throw new Refused();
    }

    internal static IReadOnlyList<T> List<T>(JsonElement o, string key, Func<JsonElement, T> read)
    {
        JsonElement value = Field(o, key);
        if (value.ValueKind != JsonValueKind.Array)
        {
            throw new Refused();
        }

        var items = new List<T>();
        foreach (JsonElement item in value.EnumerateArray())
        {
            items.Add(read(item));
        }

        return items;
    }

    internal static IReadOnlyList<string> Strings(JsonElement o, string key) =>
        List(o, key, item => item.ValueKind == JsonValueKind.String ? item.GetString()! : throw new Refused());

    internal static T Nested<T>(JsonElement o, string key, Func<JsonElement, T> read) => read(Obj(Field(o, key)));

    internal static T? OptNested<T>(JsonElement o, string key, Func<JsonElement, T> read)
        where T : class =>
        Absent(o, key, out JsonElement value) ? null : read(Obj(value));

    /// <summary>
    /// A daemon timestamp: RFC 3339, with or without fractional seconds
    /// (<c>DaemonDecoding.decoder()</c>, Models.swift:982-995). Unreadable refuses.
    /// </summary>
    internal static DateTimeOffset Timestamp(JsonElement o, string key) =>
        DateTimeOffset.TryParse(
            Str(o, key),
            CultureInfo.InvariantCulture,
            DateTimeStyles.AssumeUniversal | DateTimeStyles.AdjustToUniversal,
            out DateTimeOffset when)
            ? when
            : throw new Refused();
}

/// <summary>One excerpt the candidate cites (<c>SkillSourceEvidence</c>, SkillLearningModels.swift:83-94).</summary>
/// <remarks>
/// <c>Kind</c> stays the wire value: the Mac's <c>SessionEvidenceKind</c>
/// reads any unrecognized kind as <c>unknown</c>, and
/// <see cref="PublicRunCopy.EvidenceKindLabel"/> words it the same way.
/// </remarks>
public sealed record SkillSourceEvidence(string EventId, string Kind, string Excerpt)
{
    public string Id => EventId;

    internal static SkillSourceEvidence Read(JsonElement e) =>
        new(SkillWire.Str(e, "event_id"), SkillWire.Str(e, "kind"), SkillWire.Str(e, "excerpt"));
}

/// <summary>The editable skill (<c>SkillDraft</c>, SkillLearningModels.swift:96-100).</summary>
public sealed record SkillDraft(string Name, string Description, string Procedure)
{
    /// <summary>The draft as the wire carries it, for <c>skill_review</c> and the core's validator.</summary>
    public JsonObject ToNode() => new()
    {
        ["name"] = Name,
        ["description"] = Description,
        ["procedure"] = Procedure,
    };

    /// <summary>The draft's JSON, as the Mac's <c>JSONEncoder().encode(draft)</c> writes it.</summary>
    public string ToJson() => ToNode().ToJsonString();

    internal static SkillDraft Read(JsonElement e) =>
        new(SkillWire.Str(e, "name"), SkillWire.Str(e, "description"), SkillWire.Str(e, "procedure"));
}

/// <summary>The core's verdict on a draft (<c>SkillDraftValidation</c>, SkillLearningModels.swift:102-122).</summary>
public sealed record SkillDraftValidation(
    bool Valid,
    string? Error,
    int NameChars,
    int DescriptionChars,
    int ProcedureChars,
    int NameMaxChars,
    int DescriptionMaxChars,
    int ProcedureMaxChars)
{
    /// <summary>The verdict, or null when it does not decode.</summary>
    public static SkillDraftValidation? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return SkillWire.Decode(doc.RootElement, e => new SkillDraftValidation(
                SkillWire.Bool(e, "valid"),
                SkillWire.OptStr(e, "error"),
                SkillWire.Int(e, "name_chars"),
                SkillWire.Int(e, "description_chars"),
                SkillWire.Int(e, "procedure_chars"),
                SkillWire.Int(e, "name_max_chars"),
                SkillWire.Int(e, "description_max_chars"),
                SkillWire.Int(e, "procedure_max_chars")));
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>What a test will spend and send (<c>SkillEvaluationContract</c>, SkillLearningModels.swift:124-150).</summary>
public sealed record SkillEvaluationContract(
    int TaskCount,
    int PlanTaskCount,
    int FixturePoolCount,
    int ClusterCount,
    int ArmCount,
    int TotalRequests,
    int OutputTokenLimit,
    int RequestTimeoutSeconds,
    string RequiredModelOwner,
    string FixtureScope,
    string SelectionPolicy)
{
    internal static SkillEvaluationContract Read(JsonElement e) => new(
        SkillWire.Int(e, "task_count"),
        SkillWire.Int(e, "plan_task_count"),
        SkillWire.Int(e, "fixture_pool_count"),
        SkillWire.Int(e, "cluster_count"),
        SkillWire.Int(e, "arm_count"),
        SkillWire.Int(e, "total_requests"),
        SkillWire.Int(e, "output_token_limit"),
        SkillWire.Int(e, "request_timeout_seconds"),
        SkillWire.Str(e, "required_model_owner"),
        SkillWire.Str(e, "fixture_scope"),
        SkillWire.Str(e, "selection_policy"));
}

/// <summary><c>skill_candidate</c>'s answer (<c>SkillCandidate</c>, SkillLearningModels.swift:152-188).</summary>
public sealed record SkillCandidate(
    string CandidateId,
    string Family,
    string SourceSubmissionId,
    string SourceCorrection,
    IReadOnlyList<SkillSourceEvidence> SourceEvidence,
    SkillDraft Draft,
    string? ReplacesReviewId,
    string ManualControlInstruction,
    SkillEvaluationContract EvaluationContract)
{
    /// <summary>The candidate with another draft (<c>replacingDraft</c>, SkillLearningModels.swift:175-187).</summary>
    public SkillCandidate ReplacingDraft(SkillDraft value) => this with { Draft = value };

    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillCandidate? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillCandidate Read(JsonElement e) => new(
        SkillWire.Str(e, "candidate_id"),
        SkillWire.Str(e, "family"),
        SkillWire.Str(e, "source_submission_id"),
        SkillWire.Str(e, "source_correction"),
        SkillWire.List(e, "source_evidence", item => SkillSourceEvidence.Read(SkillWire.Obj(item))),
        SkillWire.Nested(e, "draft", SkillDraft.Read),
        SkillWire.OptStr(e, "replaces_review_id"),
        SkillWire.Str(e, "manual_control_instruction"),
        SkillWire.Nested(e, "evaluation_contract", SkillEvaluationContract.Read));
}

/// <summary><c>skill_review</c>'s answer (<c>SkillReview</c>, SkillLearningModels.swift:190-210).</summary>
public sealed record SkillReview(
    string ReviewId,
    string CandidateId,
    string Family,
    string SourceSubmissionId,
    IReadOnlyList<string> SourceEvidenceIds,
    SkillDraft Draft,
    string SkillMd,
    string SkillSha256)
{
    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillReview? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillReview Read(JsonElement e) => new(
        SkillWire.Str(e, "review_id"),
        SkillWire.Str(e, "candidate_id"),
        SkillWire.Str(e, "family"),
        SkillWire.Str(e, "source_submission_id"),
        SkillWire.Strings(e, "source_evidence_ids"),
        SkillWire.Nested(e, "draft", SkillDraft.Read),
        SkillWire.Str(e, "skill_md"),
        SkillWire.Str(e, "skill_sha256"));
}

/// <summary>A test arm (<c>SkillEvaluationArm</c>, SkillLearningModels.swift:212-216).</summary>
public enum SkillEvaluationArm
{
    Baseline,
    ManualInstruction,
    CandidateSkill,
}

/// <summary>The arm's wire value, both ways.</summary>
public static class SkillEvaluationArms
{
    public static string Wire(this SkillEvaluationArm arm) => arm switch
    {
        SkillEvaluationArm.Baseline => "baseline",
        SkillEvaluationArm.ManualInstruction => "manual_instruction",
        _ => "candidate_skill",
    };

    /// <summary>An arm this build does not know refuses the value it is in, as Swift's raw-value decode does.</summary>
    internal static SkillEvaluationArm Read(JsonElement o, string key) => SkillWire.Str(o, key) switch
    {
        "baseline" => SkillEvaluationArm.Baseline,
        "manual_instruction" => SkillEvaluationArm.ManualInstruction,
        "candidate_skill" => SkillEvaluationArm.CandidateSkill,
        _ => throw new SkillWire.Refused(),
    };
}

/// <summary>A trial's token use (<c>SkillEvaluationUsage</c>, SkillLearningModels.swift:218-230).</summary>
public sealed record SkillEvaluationUsage(int? PromptTokens, int? CompletionTokens, int? ReasoningTokens, int? TotalTokens)
{
    internal static SkillEvaluationUsage Read(JsonElement e) => new(
        SkillWire.OptInt(e, "prompt_tokens"),
        SkillWire.OptInt(e, "completion_tokens"),
        SkillWire.OptInt(e, "reasoning_tokens"),
        SkillWire.OptInt(e, "total_tokens"));
}

/// <summary>The plan a trial's model proposed (<c>ProposedTaskPlan</c>, SkillLearningModels.swift:232-244).</summary>
public sealed record ProposedTaskPlan(
    string Diagnosis,
    IReadOnlyList<string> EditPaths,
    IReadOnlyList<string> Commands,
    IReadOnlyList<string> Verification)
{
    internal static ProposedTaskPlan Read(JsonElement e) => new(
        SkillWire.Str(e, "diagnosis"),
        SkillWire.Strings(e, "edit_paths"),
        SkillWire.Strings(e, "commands"),
        SkillWire.Strings(e, "verification"));
}

/// <summary>One trial (<c>SkillTrialResult</c>, SkillLearningModels.swift:246-278).</summary>
public sealed record SkillTrialResult(
    string TaskId,
    string Cluster,
    string Task,
    Uri SourceUrl,
    SkillEvaluationArm Arm,
    bool Passed,
    IReadOnlyList<string> FailureReasons,
    ProposedTaskPlan? Answer,
    string RawOutput,
    string? RequestId,
    string ServedModel,
    string FinishReason,
    SkillEvaluationUsage Usage)
{
    /// <summary>The trial's identity: its task and arm (SkillLearningModels.swift:261).</summary>
    public string Id => TaskId + "-" + Arm.Wire();

    internal static SkillTrialResult Read(JsonElement e) => new(
        SkillWire.Str(e, "task_id"),
        SkillWire.Str(e, "cluster"),
        SkillWire.Str(e, "task"),
        Uri.TryCreate(SkillWire.Str(e, "source_url"), UriKind.RelativeOrAbsolute, out Uri? url) ? url : throw new SkillWire.Refused(),
        SkillEvaluationArms.Read(e, "arm"),
        SkillWire.Bool(e, "passed"),
        SkillWire.Strings(e, "failure_reasons"),
        SkillWire.OptNested(e, "answer", ProposedTaskPlan.Read),
        SkillWire.Str(e, "raw_output"),
        SkillWire.OptStr(e, "request_id"),
        SkillWire.Str(e, "served_model"),
        SkillWire.Str(e, "finish_reason"),
        SkillWire.Nested(e, "usage", SkillEvaluationUsage.Read));
}

/// <summary>One arm's pass count (<c>SkillArmSummary</c>, SkillLearningModels.swift:280-287).</summary>
public sealed record SkillArmSummary(SkillEvaluationArm Arm, string Label, int Passed, int Total)
{
    public SkillEvaluationArm Id => Arm;

    internal static SkillArmSummary Read(JsonElement e) => new(
        SkillEvaluationArms.Read(e, "arm"),
        SkillWire.Str(e, "label"),
        SkillWire.Int(e, "passed"),
        SkillWire.Int(e, "total"));
}

/// <summary>How the test ran (<c>SkillEvaluationExecution</c>, SkillLearningModels.swift:289-309).</summary>
public sealed record SkillEvaluationExecution(
    string RequestedModel,
    string ServedModel,
    string ModelOwnedBy,
    string SourceTaskFingerprintSha256,
    IReadOnlyList<string> SelectedPlanTaskIds,
    IReadOnlyList<string> ExcludedSourceOverlapTaskIds,
    IReadOnlyList<string> ReservePlanTaskIds,
    IReadOnlyList<string> AmbientContext)
{
    internal static SkillEvaluationExecution Read(JsonElement e) => new(
        SkillWire.Str(e, "requested_model"),
        SkillWire.Str(e, "served_model"),
        SkillWire.Str(e, "model_owned_by"),
        SkillWire.Str(e, "source_task_fingerprint_sha256"),
        SkillWire.Strings(e, "selected_plan_task_ids"),
        SkillWire.Strings(e, "excluded_source_overlap_task_ids"),
        SkillWire.Strings(e, "reserve_plan_task_ids"),
        SkillWire.Strings(e, "ambient_context"));
}

/// <summary>
/// <c>skill_evaluate</c>'s answer (<c>SkillEvaluationReport</c>,
/// SkillLearningModels.swift:311-373). Its contract and execution fields sit
/// flat beside its own, and are read from the same object (:355-357).
/// </summary>
public sealed record SkillEvaluationReport(
    string EvaluationId,
    string ReviewId,
    string SkillSha256,
    SkillEvaluationContract EvaluationContract,
    SkillEvaluationExecution Execution,
    IReadOnlyList<SkillArmSummary> Summaries,
    IReadOnlyList<SkillArmSummary> PlanSummaries,
    IReadOnlyList<SkillArmSummary> ApplicabilitySummaries,
    IReadOnlyList<SkillTrialResult> Trials,
    IReadOnlyList<string> Regressions,
    bool InstallAllowed,
    string GateReason)
{
    public int OutputTokenLimit => EvaluationContract.OutputTokenLimit;
    public int RequestTimeoutSeconds => EvaluationContract.RequestTimeoutSeconds;
    public int ArmCount => EvaluationContract.ArmCount;
    public int TaskCount => EvaluationContract.TaskCount;
    public int PlanTaskCount => EvaluationContract.PlanTaskCount;
    public int FixturePoolCount => EvaluationContract.FixturePoolCount;
    public int ClusterCount => EvaluationContract.ClusterCount;
    public string SelectionPolicy => EvaluationContract.SelectionPolicy;
    public string RequestedModel => Execution.RequestedModel;
    public string ServedModel => Execution.ServedModel;
    public string ModelOwnedBy => Execution.ModelOwnedBy;
    public string SourceTaskFingerprintSha256 => Execution.SourceTaskFingerprintSha256;
    public IReadOnlyList<string> SelectedPlanTaskIds => Execution.SelectedPlanTaskIds;
    public IReadOnlyList<string> ExcludedSourceOverlapTaskIds => Execution.ExcludedSourceOverlapTaskIds;
    public IReadOnlyList<string> ReservePlanTaskIds => Execution.ReservePlanTaskIds;
    public IReadOnlyList<string> AmbientContext => Execution.AmbientContext;

    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillEvaluationReport? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillEvaluationReport Read(JsonElement e) => new(
        SkillWire.Str(e, "evaluation_id"),
        SkillWire.Str(e, "review_id"),
        SkillWire.Str(e, "skill_sha256"),
        SkillEvaluationContract.Read(SkillWire.Obj(e)),
        SkillEvaluationExecution.Read(SkillWire.Obj(e)),
        SkillWire.List(e, "summaries", item => SkillArmSummary.Read(SkillWire.Obj(item))),
        SkillWire.List(e, "plan_summaries", item => SkillArmSummary.Read(SkillWire.Obj(item))),
        SkillWire.List(e, "applicability_summaries", item => SkillArmSummary.Read(SkillWire.Obj(item))),
        SkillWire.List(e, "trials", item => SkillTrialResult.Read(SkillWire.Obj(item))),
        SkillWire.Strings(e, "regressions"),
        SkillWire.Bool(e, "install_allowed"),
        SkillWire.Str(e, "gate_reason"));
}

/// <summary><c>skill_install_plan</c>'s answer (<c>SkillInstallPlan</c>, SkillLearningModels.swift:375-403).</summary>
public sealed record SkillInstallPlan(
    string PlanId,
    string EvaluationId,
    string Tool,
    string TargetLocation,
    string SkillLocation,
    string MarkerLocation,
    bool Occupied,
    bool CanInstall,
    string SkillMd,
    string SkillSha256,
    string MarkerJson,
    string MarkerFileSha256)
{
    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillInstallPlan? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillInstallPlan Read(JsonElement e) => new(
        SkillWire.Str(e, "plan_id"),
        SkillWire.Str(e, "evaluation_id"),
        SkillWire.Str(e, "tool"),
        SkillWire.Str(e, "target_location"),
        SkillWire.Str(e, "skill_location"),
        SkillWire.Str(e, "marker_location"),
        SkillWire.Bool(e, "occupied"),
        SkillWire.Bool(e, "can_install"),
        SkillWire.Str(e, "skill_md"),
        SkillWire.Str(e, "skill_sha256"),
        SkillWire.Str(e, "marker_json"),
        SkillWire.Str(e, "marker_file_sha256"));
}

/// <summary>An installed skill's receipt (<c>SkillLearningInstall</c>, SkillLearningModels.swift:405-427).</summary>
public sealed record SkillLearningInstall(
    string InstallId,
    string EvaluationId,
    string SourceSubmissionId,
    string Tool,
    string Name,
    string TargetLocation,
    string SkillSha256,
    string MarkerSha256,
    DateTimeOffset InstalledAt)
{
    /// <summary>The receipt, or null when it does not decode.</summary>
    public static SkillLearningInstall? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    internal static SkillLearningInstall Read(JsonElement e) => new(
        SkillWire.Str(e, "install_id"),
        SkillWire.Str(e, "evaluation_id"),
        SkillWire.Str(e, "source_submission_id"),
        SkillWire.Str(e, "tool"),
        SkillWire.Str(e, "name"),
        SkillWire.Str(e, "target_location"),
        SkillWire.Str(e, "skill_sha256"),
        SkillWire.Str(e, "marker_sha256"),
        SkillWire.Timestamp(e, "installed_at"));
}

/// <summary>
/// <c>skill_install_status</c>'s answer (<c>SkillInstallStatus</c>,
/// SkillLearningModels.swift:429-432). An absent and a null <c>skill</c> are
/// both none.
/// </summary>
public sealed record SkillInstallStatus(bool Installed, SkillLearningInstall? Skill)
{
    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillInstallStatus? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillInstallStatus Read(JsonElement e) =>
        new(SkillWire.Bool(e, "installed"), SkillWire.OptNested(e, "skill", SkillLearningInstall.Read));
}

/// <summary><c>skill_install_rollback</c>'s answer (<c>SkillRollbackResult</c>, SkillLearningModels.swift:434-442).</summary>
public sealed record SkillRollbackResult(bool Removed, bool RetainedDirectory)
{
    /// <summary>The answer, or null when it does not decode.</summary>
    public static SkillRollbackResult? Parse(JsonElement e) => SkillWire.Decode(e, Read);

    private static SkillRollbackResult Read(JsonElement e) =>
        new(SkillWire.Bool(e, "removed"), SkillWire.Bool(e, "retained_directory"));
}
