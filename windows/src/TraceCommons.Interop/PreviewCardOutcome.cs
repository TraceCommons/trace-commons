using System;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The wire shape both <c>preview_request</c>'s immediate result and the
/// <c>preview_ready</c> event's payload carry.
///
/// One type for both because the daemon builds them from the same function --
/// <c>PreviewOutcome::to_value</c> in
/// <c>crates/trace-commons-contributor/src/daemon/preview_scheduler.rs</c> -- so
/// the two cannot describe the same build differently. A card fills itself in
/// from whichever one arrives first: a cache hit answers <c>preview_request</c>
/// directly with no event to follow, and a queued or running build answers
/// with a state that carries no data and is filled in later by
/// <c>preview_ready</c>.
/// </summary>
public sealed class PreviewCardOutcome
{
    public const string StateQueued = "queued";
    public const string StateRunning = "running";
    public const string StateReady = "ready";
    public const string StateTooLarge = "too_large";
    public const string StateFailed = "failed";

    public string EntryId { get; private init; } = string.Empty;

    public string State { get; private init; } = string.Empty;

    /// <summary>Only present when <see cref="State"/> is <see cref="StateReady"/>.</summary>
    public PreviewSummary? Summary { get; private init; }

    /// <summary>
    /// Only meaningful when <see cref="State"/> is <see cref="StateTooLarge"/>.
    /// A <c>stat</c> of the file, never an estimate.
    /// </summary>
    public long RawSessionBytes { get; private init; }

    /// <summary>The admission cap that refused the session, when <see cref="IsTooLarge"/>.</summary>
    public long LimitBytes { get; private init; }

    /// <summary>Only present when <see cref="State"/> is <see cref="StateFailed"/>.</summary>
    public string? Code { get; private init; }

    /// <summary>The fixed, content-free label paired with <see cref="Code"/>.</summary>
    public string? Label { get; private init; }

    /// <summary>
    /// Enqueued or already building; a <c>preview_ready</c> event will follow.
    /// A card in this state shows a pending affordance and nothing else.
    /// </summary>
    public bool IsPending => State is StateQueued or StateRunning;

    public bool IsReady => State == StateReady;

    public bool IsTooLarge => State == StateTooLarge;

    public bool IsFailed => State == StateFailed;

    /// <summary>
    /// The card's line for a refusal on size.
    ///
    /// NEVER a would-send estimate. The design spec is explicit: a number
    /// derived from anything other than the envelope that would actually be
    /// sent is a false number on a consent surface, and this card is a
    /// consent surface. Only <see cref="RawSessionBytes"/> -- a <c>stat</c>,
    /// not an estimate -- may be shown alongside this line.
    /// </summary>
    public const string TooLargeText = "too large to preview";

    /// <summary>
    /// Parses the object both <c>preview_request</c>'s result and
    /// <c>preview_ready</c>'s payload carry, or returns null if it does not
    /// fit -- which a card renders the same as "still pending" rather than as
    /// a crash.
    /// </summary>
    public static PreviewCardOutcome? Parse(JsonElement element)
    {
        if (element.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        if (!TryGetString(element, "entry_id", out string entryId)
            || !TryGetString(element, "state", out string state))
        {
            return null;
        }

        PreviewSummary? summary = null;
        if (state == StateReady
            && element.TryGetProperty("summary", out JsonElement summaryEl)
            && summaryEl.ValueKind == JsonValueKind.Object)
        {
            summary = PreviewSummary.Parse(summaryEl.GetRawText());
        }

        long rawBytes = 0;
        long limitBytes = 0;
        if (state == StateTooLarge)
        {
            rawBytes = LongField(element, "raw_session_bytes");
            limitBytes = LongField(element, "limit_bytes");
        }

        string? code = null;
        string? label = null;
        if (state == StateFailed)
        {
            code = GetOptionalString(element, "code");
            label = GetOptionalString(element, "label");
        }

        return new PreviewCardOutcome
        {
            EntryId = entryId,
            State = state,
            Summary = summary,
            RawSessionBytes = rawBytes,
            LimitBytes = limitBytes,
            Code = code,
            Label = label,
        };
    }

    /// <summary>Parses a raw JSON string, or returns null when it cannot be read.</summary>
    public static PreviewCardOutcome? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            return Parse(document.RootElement);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static long LongField(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement field) && field.TryGetInt64(out long value)
            ? value
            : 0;

    private static bool TryGetString(JsonElement element, string name, out string value)
    {
        if (element.TryGetProperty(name, out JsonElement field) && field.ValueKind == JsonValueKind.String)
        {
            value = field.GetString() ?? string.Empty;
            return true;
        }

        value = string.Empty;
        return false;
    }

    private static string? GetOptionalString(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement field) && field.ValueKind == JsonValueKind.String
            ? field.GetString()
            : null;
}
