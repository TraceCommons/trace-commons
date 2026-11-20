using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The words for adding Train coding models to traces already sent, decoded
/// from <c>tc_sent_traces_training_copy_json</c> (the Mac's
/// <c>SentTracesTrainingCopy</c>, macos/Sources/TCShellCore/SentTracesTrainingSurface.swift:11-104).
/// None is written in C#: a payload with a missing or blank field is refused
/// whole. The <c>*_other</c> templates and <c>allow_accessibility_name</c>
/// carry <c>{count}</c>.
/// </summary>
public sealed record SentTracesTrainingCopy(
    string Title, string BodyOne, string BodyOther, string Scope, string AllowButton,
    string AllowAccessibilityName, string NotNowButton, string NotNowAccessibilityName,
    string LineOne, string LineOther, string ReviewButton, string ReviewAccessibilityName,
    string TraceLine, string TraceButton, string TraceAccessibilityName, string Pending, string Failed)
{
    /// <summary>The payload fields this shell decodes, by wire name (SentTracesTrainingSurface.swift:51-56).</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body_one", "body_other", "scope", "allow_button", "allow_accessibility_name",
        "not_now_button", "not_now_accessibility_name", "line_one", "line_other", "review_button",
        "review_accessibility_name", "trace_line", "trace_button", "trace_accessibility_name",
        "pending", "failed",
    };

    /// <summary>The core's words, read once; null if the export did not arrive (SentTracesTrainingModel.swift:18-21).</summary>
    public static SentTracesTrainingCopy? Current => s_current.Value;

    private static readonly Lazy<SentTracesTrainingCopy?> s_current = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_sent_traces_training_copy_json())));

    /// <summary>Null, never a partly filled value, for an unparseable payload or a blank field (:68-74).</summary>
    public static SentTracesTrainingCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        var values = new string[ConsumedFields.Count];
        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            for (int i = 0; i < ConsumedFields.Count; i++)
            {
                if (!document.RootElement.TryGetProperty(ConsumedFields[i], out JsonElement value)
                    || value.ValueKind != JsonValueKind.String
                    || Blank(value.GetString()!))
                {
                    return null;
                }

                values[i] = value.GetString()!;
            }
        }
        catch (JsonException)
        {
            return null;
        }

        return new SentTracesTrainingCopy(values[0], values[1], values[2], values[3], values[4], values[5], values[6],
            values[7], values[8], values[9], values[10], values[11], values[12], values[13], values[14], values[15], values[16]);
    }

    /// <summary>Swift's <c>trimmingCharacters(in: .whitespaces)</c>: spaces and tabs, not line breaks.</summary>
    private static bool Blank(string text) =>
        text.All(c => c == '\t' || CharUnicodeInfo.GetUnicodeCategory(c) == UnicodeCategory.SpaceSeparator);

    /// <summary><paramref name="template"/> with <c>{count}</c> filled in (:77-79).</summary>
    public static string Render(string template, ulong count) =>
        template.Replace("{count}", count.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);

    /// <summary>The sheet's body: the one-trace sentence for 1, the counted one otherwise (:83-85).</summary>
    public string Body(ulong count) => count == 1 ? BodyOne : Render(BodyOther, count);

    /// <summary>The Insights line, chosen the same way (:88-90).</summary>
    public string Line(ulong count) => count == 1 ? LineOne : Render(LineOther, count);

    /// <summary>Allow's accessibility name for the count (:92-94).</summary>
    public string AllowAccessibilityNameFor(ulong count) => Render(AllowAccessibilityName, count);
}

/// <summary><c>model_training_consent_status</c>: every eligible trace counted, and the oldest 500 ids (:99-120).</summary>
public sealed record ModelTrainingConsentStatus(ulong EligibleCount, IReadOnlyList<string> SubmissionIds);

/// <summary>
/// One trace in a <c>model_training_consent_grant</c> answer (:125-156).
/// <c>Outcome</c> is granted, already_granted or not_eligible; <c>Award</c>
/// is credited, pending or none.
/// </summary>
public sealed record ModelTrainingGrantResult(
    string SubmissionId, string Outcome, string? GrantedAt, string Award, string? AwardWithheldReason)
{
    /// <summary>Whether a credit follows: the use is on the trace and the server wrote the award or will (:152-155).</summary>
    public bool CreditFollows =>
        (Outcome == "granted" || Outcome == "already_granted") && (Award == "pending" || Award == "credited");
}

/// <summary><c>model_training_consent_grant</c>'s answer (:159-172).</summary>
public sealed record ModelTrainingGrantAnswer(IReadOnlyList<ModelTrainingGrantResult> Results, ulong RemainingEligible);

/// <summary>A refusal, routed from the daemon's fixed label (:175-191).</summary>
public enum SentTracesTrainingRefusal
{
    /// <summary>Model training not in the ceiling, no server record, or nobody enrolled: every affordance hides.</summary>
    Hidden,

    /// <summary>Anything else. Nothing changed.</summary>
    Failed,
}

/// <summary>The label routing of <see cref="SentTracesTrainingRefusal"/> (:183-190).</summary>
public static class SentTracesTrainingRefusals
{
    public static SentTracesTrainingRefusal From(string label) => label switch
    {
        "model-training-not-permitted" or "model-training-consent-unavailable" or "not-logged-in" => SentTracesTrainingRefusal.Hidden,
        _ => SentTracesTrainingRefusal.Failed,
    };
}

/// <summary>A trace's mark after the person allowed the use on it (:194-199).</summary>
public enum SentTraceTrainingMark
{
    /// <summary>Allowed, and its credit has not shown on its History row yet.</summary>
    Pending,

    /// <summary>The grant did not go through; the trace is offered again.</summary>
    Failed,
}

/// <summary>Which window raised the sheet: it is drawn there and nowhere else (:202-208).</summary>
public enum SentTracesTrainingSite
{
    /// <summary>Settings, where Train coding models was turned on.</summary>
    Settings,

    /// <summary>The main window: Home's line and Review.</summary>
    Monitor,
}

/// <summary>The sheet while it is up: where, and the count it was raised for (:211-219).</summary>
public sealed record SentTracesTrainingSheet(SentTracesTrainingSite Site, ulong Count);

/// <summary>What the shell knows about eligible traces (:227-235).</summary>
public enum SentTracesTrainingEligibility
{
    /// <summary>Not asked yet: nothing is drawn.</summary>
    Unknown,

    /// <summary>Refused, or nothing eligible: nothing is drawn.</summary>
    Hidden,

    /// <summary>Some traces are eligible.</summary>
    Eligible,
}

/// <summary>
/// What the shell knows about sent traces without Train coding models, and
/// the marks it drew after a grant (<c>SentTracesTrainingState</c>, :223-322).
/// Pure: every fact is the daemon's.
/// </summary>
public sealed class SentTracesTrainingState
{
    /// <summary>The consent scope and allowed use this is about.</summary>
    public const string Scope = "model_training";

    private ulong _count;
    private HashSet<string> _ids = new(StringComparer.Ordinal);
    private readonly Dictionary<string, SentTraceTrainingMark> _marks = new(StringComparer.Ordinal);

    public SentTracesTrainingEligibility Eligibility { get; private set; } = SentTracesTrainingEligibility.Unknown;

    /// <summary>Whether this write turns Train coding models on: absent before, present now (:243-245).</summary>
    public static bool TurnsOn(IEnumerable<string> before, IEnumerable<string> after) =>
        !before.Contains(Scope, StringComparer.Ordinal) && after.Contains(Scope, StringComparer.Ordinal);

    /// <summary>The count the line and the sheet say, or null when nothing is drawn (:249-252).</summary>
    public ulong? Count => Eligibility == SentTracesTrainingEligibility.Eligible && _count > 0 ? _count : null;

    /// <summary>The sheet a turn-on raises: only while traces are eligible (:256-258).</summary>
    public SentTracesTrainingSheet? Sheet(SentTracesTrainingSite site) =>
        Count is ulong count ? new SentTracesTrainingSheet(site, count) : null;

    /// <summary>Whether the inspector offers the use on this trace (:262-265).</summary>
    public bool Offers(string submissionId) =>
        Eligibility == SentTracesTrainingEligibility.Eligible
        && _ids.Contains(submissionId)
        && !(_marks.TryGetValue(submissionId, out SentTraceTrainingMark mark) && mark == SentTraceTrainingMark.Pending);

    /// <summary>The mark this trace draws: pending until credited, failed until offered again (:269-275).</summary>
    public SentTraceTrainingMark? Mark(string submissionId, bool credited)
    {
        if (!_marks.TryGetValue(submissionId, out SentTraceTrainingMark mark))
        {
            return null;
        }

        return mark == SentTraceTrainingMark.Pending
            ? (credited ? null : SentTraceTrainingMark.Pending)
            : (Offers(submissionId) ? SentTraceTrainingMark.Failed : null);
    }

    /// <summary>Whether a History row reports the trace's credit: final credit above zero (:283-285).</summary>
    public static bool Credited(double? final) => (final ?? 0) > 0;

    /// <summary>A status read answered (:287-290).</summary>
    public void Apply(ModelTrainingConsentStatus status)
    {
        if (status.EligibleCount == 0)
        {
            Eligibility = SentTracesTrainingEligibility.Hidden;
            return;
        }

        Eligibility = SentTracesTrainingEligibility.Eligible;
        _count = status.EligibleCount;
        _ids = new HashSet<string>(status.SubmissionIds, StringComparer.Ordinal);
    }

    /// <summary>A status read was refused: nothing is drawn, whatever the label (:294-296).</summary>
    public void ApplyStatusRefusal() => Eligibility = SentTracesTrainingEligibility.Hidden;

    /// <summary>
    /// A grant answered: a trace whose credit follows is marked pending, every
    /// answered trace leaves the eligible set, the count is the server's (:301-310).
    /// </summary>
    public void Apply(ModelTrainingGrantAnswer answer)
    {
        var ids = Eligibility == SentTracesTrainingEligibility.Eligible
            ? new HashSet<string>(_ids, StringComparer.Ordinal)
            : new HashSet<string>(StringComparer.Ordinal);
        foreach (ModelTrainingGrantResult result in answer.Results)
        {
            ids.Remove(result.SubmissionId);
            if (result.CreditFollows)
            {
                _marks[result.SubmissionId] = SentTraceTrainingMark.Pending;
            }
            else
            {
                _marks.Remove(result.SubmissionId);
            }
        }

        if (answer.RemainingEligible == 0)
        {
            Eligibility = SentTracesTrainingEligibility.Hidden;
            return;
        }

        Eligibility = SentTracesTrainingEligibility.Eligible;
        _count = answer.RemainingEligible;
        _ids = ids;
    }

    /// <summary>A grant for these traces was refused (:313-321).</summary>
    public void ApplyGrantRefusal(SentTracesTrainingRefusal refusal, IEnumerable<string> submissionIds)
    {
        if (refusal == SentTracesTrainingRefusal.Hidden)
        {
            Eligibility = SentTracesTrainingEligibility.Hidden;
            return;
        }

        foreach (string id in submissionIds)
        {
            _marks[id] = SentTraceTrainingMark.Failed;
        }
    }
}

/// <summary>A daemon refusal: its fixed label, safe to route on and never shown (:328-331).</summary>
public sealed class SentTracesTrainingFailure : Exception
{
    public SentTracesTrainingFailure(string label)
        : base(label) => Label = label;

    public string Label { get; }

    public SentTracesTrainingRefusal Refusal => SentTracesTrainingRefusals.From(Label);
}

/// <summary>
/// The two IPC calls (<c>SentTracesTrainingClient</c>, :326-364), over the
/// window's daemon call, so the method names and parameters are asserted
/// without a daemon.
/// </summary>
public sealed class SentTracesTrainingClient
{
    public const string StatusMethod = "model_training_consent_status";
    public const string GrantMethod = "model_training_consent_grant";

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public SentTracesTrainingClient(Func<string, string, Task<DaemonResponse>> call) => _call = call;

    public async Task<ModelTrainingConsentStatus> StatusAsync()
    {
        JsonElement result = await CallAsync(StatusMethod, "{}").ConfigureAwait(true);
        if (!TryUInt(result, "eligible_count", out ulong count))
        {
            throw new SentTracesTrainingFailure("unparseable-response");
        }

        var ids = new List<string>();
        if (result.TryGetProperty("submission_ids", out JsonElement list) && list.ValueKind != JsonValueKind.Null)
        {
            if (list.ValueKind != JsonValueKind.Array || list.EnumerateArray().Any(e => e.ValueKind != JsonValueKind.String))
            {
                throw new SentTracesTrainingFailure("unparseable-response");
            }

            ids.AddRange(list.EnumerateArray().Select(e => e.GetString()!));
        }

        return new ModelTrainingConsentStatus(count, ids);
    }

    /// <summary>Every eligible trace. Sent only after the person chose Allow (:342-344).</summary>
    public async Task<ModelTrainingGrantAnswer> GrantAllAsync() =>
        Answer(await CallAsync(GrantMethod, "{\"all_eligible\":true}").ConfigureAwait(true));

    /// <summary>These traces only. Sent only after the person chose Allow on them (:347-350).</summary>
    public async Task<ModelTrainingGrantAnswer> GrantAsync(IReadOnlyList<string> submissionIds) =>
        Answer(await CallAsync(GrantMethod, JsonSerializer.Serialize(new { submission_ids = submissionIds })).ConfigureAwait(true));

    private async Task<JsonElement> CallAsync(string method, string parameters)
    {
        DaemonResponse response = await _call(method, parameters).ConfigureAwait(true);
        if (response.Error is { } error)
        {
            throw new SentTracesTrainingFailure(string.IsNullOrEmpty(error.Message) ? "unavailable" : error.Message);
        }

        if (response.Result is not JsonElement result || result.ValueKind != JsonValueKind.Object)
        {
            throw new SentTracesTrainingFailure("unparseable-response");
        }

        return result;
    }

    private static ModelTrainingGrantAnswer Answer(JsonElement result)
    {
        if (!TryUInt(result, "remaining_eligible", out ulong remaining)
            || !result.TryGetProperty("results", out JsonElement list) || list.ValueKind != JsonValueKind.Array)
        {
            throw new SentTracesTrainingFailure("unparseable-response");
        }

        var results = new List<ModelTrainingGrantResult>();
        foreach (JsonElement item in list.EnumerateArray())
        {
            if (Str(item, "submission_id") is not { } id || Str(item, "outcome") is not { } outcome || Str(item, "award") is not { } award)
            {
                throw new SentTracesTrainingFailure("unparseable-response");
            }

            results.Add(new ModelTrainingGrantResult(id, outcome, Str(item, "granted_at"), award, Str(item, "award_withheld_reason")));
        }

        return new ModelTrainingGrantAnswer(results, remaining);
    }

    private static string? Str(JsonElement item, string name) =>
        item.ValueKind == JsonValueKind.Object && item.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static bool TryUInt(JsonElement item, string name, out ulong value)
    {
        value = 0;
        return item.TryGetProperty(name, out JsonElement element)
            && element.ValueKind == JsonValueKind.Number
            && element.TryGetUInt64(out value);
    }
}

/// <summary>Which of the sheet's two choices a button is.</summary>
public enum SentTracesTrainingChoice
{
    NotNow,
    Allow,
}

/// <summary>
/// One of the sheet's two buttons (<c>SentTracesTrainingSheetView.actions</c>,
/// SentTracesTrainingViews.swift:44-56): the same style, neither the default.
/// </summary>
public sealed record SentTracesTrainingAction(
    SentTracesTrainingChoice Choice, string Title, string AccessibilityName, bool IsEnabled, bool IsDefault);

/// <summary>
/// Sent traces without Train coding models: how many there are, the one
/// sheet that offers the use on all of them, and the marks a grant leaves
/// (<c>SentTracesTrainingModel</c>, macos/Sources/TraceCommonsApp/SentTracesTrainingModel.swift).
/// Nothing is preselected and nothing is sent until the person chooses: the
/// sheet is raised once per turn-on and by Home's Review; Not now closes it
/// and changes nothing.
/// </summary>
public sealed class SentTracesTrainingModel : INotifyPropertyChanged
{
    private readonly SentTracesTrainingClient? _client;
    private long _generation;

    public SentTracesTrainingModel(Func<string, string, Task<DaemonResponse>>? call, SentTracesTrainingCopy? copy)
    {
        _client = call is null ? null : new SentTracesTrainingClient(call);
        Copy = copy;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>The core's words; null means nothing is drawn and no sheet is raised.</summary>
    public SentTracesTrainingCopy? Copy { get; }

    public SentTracesTrainingState State { get; } = new();

    /// <summary>The sheet while it is up, and the window it is drawn in.</summary>
    public SentTracesTrainingSheet? Sheet { get; private set; }

    /// <summary>A grant is in flight: the sheet cannot be closed and no second grant starts.</summary>
    public bool Busy { get; private set; }

    /// <summary>The sheet's grant was refused; the sheet stays up and says so.</summary>
    public bool SheetFailed { get; private set; }

    /// <summary>Not now, then Allow: one style, neither the default (SentTracesTrainingViews.swift:44-56).</summary>
    public static IReadOnlyList<SentTracesTrainingAction> Actions(SentTracesTrainingCopy copy, ulong count, bool busy) => new[]
    {
        new SentTracesTrainingAction(SentTracesTrainingChoice.NotNow, copy.NotNowButton, copy.NotNowAccessibilityName, !busy, IsDefault: false),
        new SentTracesTrainingAction(SentTracesTrainingChoice.Allow, copy.AllowButton, copy.AllowAccessibilityNameFor(count), !busy, IsDefault: false),
    };

    /// <summary>Reads the eligible traces again. A refusal of any kind hides every affordance (:37-50).</summary>
    public async Task RefreshAsync()
    {
        long current = ++_generation;
        if (Copy is null || _client is null)
        {
            State.ApplyStatusRefusal();
            Changed(nameof(State));
            return;
        }

        ModelTrainingConsentStatus? status = null;
        try
        {
            status = await _client.StatusAsync().ConfigureAwait(true);
        }
        catch (SentTracesTrainingFailure)
        {
        }

        if (current != _generation)
        {
            return;
        }

        if (status is null)
        {
            State.ApplyStatusRefusal();
        }
        else
        {
            State.Apply(status);
        }

        Changed(nameof(State));
    }

    /// <summary>Train coding models was just turned on: read, and raise the sheet once at <paramref name="site"/> (:54-59).</summary>
    public async Task OfferAfterTurnOnAsync(SentTracesTrainingSite site)
    {
        await RefreshAsync().ConfigureAwait(true);
        if (Sheet is null && State.Sheet(site) is { } raised)
        {
            SheetFailed = false;
            Sheet = raised;
            Changed(nameof(SheetFailed));
            Changed(nameof(Sheet));
        }
    }

    /// <summary>Home's Review: the same sheet, on the main window (:62-66).</summary>
    public void Review()
    {
        if (Sheet is null && State.Sheet(SentTracesTrainingSite.Monitor) is { } raised)
        {
            SheetFailed = false;
            Sheet = raised;
            Changed(nameof(SheetFailed));
            Changed(nameof(Sheet));
        }
    }

    /// <summary>Not now: closes the sheet and sends nothing (:69-73).</summary>
    public void NotNow()
    {
        if (Busy)
        {
            return;
        }

        Sheet = null;
        SheetFailed = false;
        Changed(nameof(Sheet));
        Changed(nameof(SheetFailed));
    }

    /// <summary>
    /// Allow: adds the use to every eligible trace. Closes on an answer, stays
    /// up and says it failed on a refusal, closes on a hiding refusal (:78-95).
    /// </summary>
    public async Task AllowAllAsync()
    {
        if (Busy || Sheet is null || _client is null)
        {
            return;
        }

        SetBusy(true);
        SheetFailed = false;
        Changed(nameof(SheetFailed));
        ModelTrainingGrantAnswer? answer = null;
        SentTracesTrainingRefusal refusal = SentTracesTrainingRefusal.Failed;
        try
        {
            answer = await _client.GrantAllAsync().ConfigureAwait(true);
        }
        catch (SentTracesTrainingFailure failure)
        {
            refusal = failure.Refusal;
        }

        SetBusy(false);
        if (answer is not null)
        {
            State.Apply(answer);
            Sheet = null;
            Changed(nameof(State));
            Changed(nameof(Sheet));
            await RefreshAsync().ConfigureAwait(true);
            return;
        }

        State.ApplyGrantRefusal(refusal, Array.Empty<string>());
        if (refusal == SentTracesTrainingRefusal.Hidden)
        {
            Sheet = null;
            Changed(nameof(Sheet));
        }
        else
        {
            SheetFailed = true;
            Changed(nameof(SheetFailed));
        }

        Changed(nameof(State));
    }

    /// <summary>The inspector's Allow: adds the use to this one trace (:98-111).</summary>
    public async Task AllowAsync(string submissionId)
    {
        if (Busy || !State.Offers(submissionId) || _client is null)
        {
            return;
        }

        SetBusy(true);
        string[] ids = { submissionId };
        ModelTrainingGrantAnswer? answer = null;
        SentTracesTrainingRefusal refusal = SentTracesTrainingRefusal.Failed;
        try
        {
            answer = await _client.GrantAsync(ids).ConfigureAwait(true);
        }
        catch (SentTracesTrainingFailure failure)
        {
            refusal = failure.Refusal;
        }

        SetBusy(false);
        if (answer is not null)
        {
            State.Apply(answer);
        }
        else
        {
            State.ApplyGrantRefusal(refusal, ids);
        }

        Changed(nameof(State));
    }

    private void SetBusy(bool busy)
    {
        Busy = busy;
        Changed(nameof(Busy));
    }

    private void Changed([CallerMemberName] string? name = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
