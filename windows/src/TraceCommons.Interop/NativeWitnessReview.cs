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
}
