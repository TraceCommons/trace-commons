using System;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>Wire decisions shared by the UI and tests. No background call requests remote review.</summary>
public static class NativeWitnessReview
{
    public const string Method = "witness_preview_request";
    public static bool Supports(DaemonResponse response) => response.Error is null
        && response.Result is { } result && result.TryGetProperty("methods", out var methods)
        && methods.ValueKind == JsonValueKind.Array
        && methods.EnumerateArray().Any(method => method.ValueKind == JsonValueKind.String && method.GetString() == Method);
    public static string ConfirmedRequest(string entryId) => JsonSerializer.Serialize(new {
        entry_id = entryId, raw_session_confirmed = true
    });
    public static bool IsReady(DaemonResponse response) => response.Error is null
        && response.Result is { } result && result.TryGetProperty("status", out var status)
        && status.ValueKind == JsonValueKind.String && status.GetString() == "ready";

    /// <summary>The sentence for a refused review, if the daemon sent one.</summary>
    /// <remarks>
    /// Read from the daemon's own <c>view.message</c>, which classifies the
    /// refusal by cause, the same way <c>AdmissionPreparation.Refusal</c>
    /// does. The daemon fills <c>view</c> on refusals too, so it is present
    /// even though <c>Error</c> is set. Fourteen causes used to arrive as one
    /// word and render one sentence here, so a reviewer that declined a
    /// receipt read exactly like a reviewer that was down.
    ///
    /// <c>null</c> for a response with no view -- a transport failure, or a
    /// daemon older than this shell -- and the caller keeps its own fallback.
    /// </remarks>
    public static string? Refusal(DaemonResponse response) =>
        response.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("view", out var view) && view.ValueKind == JsonValueKind.Object
        && view.TryGetProperty("message", out var message) && message.ValueKind == JsonValueKind.String
        && message.GetString() is { Length: > 0 } text
            ? text
            : null;

    /// <summary>"&lt;label&gt;: &lt;local time&gt;" for a review that met a busy witness.</summary>
    /// <remarks>
    /// A busy witness judged nothing, so this is not a refusal: the daemon
    /// sends <c>view.state = "Busy"</c> with the time the witness asked to be
    /// tried again (<c>retry_at</c>) and the label to show beside it
    /// (<c>retry_label</c>). <c>null</c> for any other outcome, or for a
    /// time or label this shell cannot read -- never a guessed time.
    /// </remarks>
    public static string? RetryLine(DaemonResponse response, Func<DateTimeOffset, string>? format = null) =>
        response.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("view", out var view) && view.ValueKind == JsonValueKind.Object
        && view.TryGetProperty("state", out var state) && state.ValueKind == JsonValueKind.String
        && state.GetString() == "Busy"
        && view.TryGetProperty("retry_label", out var label) && label.ValueKind == JsonValueKind.String
        && label.GetString() is { Length: > 0 } labelText
        && view.TryGetProperty("retry_at", out var at) && at.ValueKind == JsonValueKind.String
        && DateTimeOffset.TryParse(at.GetString(), System.Globalization.CultureInfo.InvariantCulture,
            System.Globalization.DateTimeStyles.AssumeUniversal, out var when)
            ? $"{labelText}: {(format ?? (d => d.ToLocalTime().ToString("t")))(when)}"
            : null;
}
