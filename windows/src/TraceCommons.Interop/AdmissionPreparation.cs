using System;
using System.Linq;
using System.Text.Json;
namespace TraceCommons.Interop;

public static class AdmissionPreparation
{
    public const string Method = "prepare_admission_session";
    public const string Heading = "Prepare next NEAR inference";
    public const string Disclosure = "This adds an account-bound challenge to the next inference request in this session. Use your own funded NEAR AI backend, then continue the agent task and return here to review.";
    public const string Success = "Ready. Continue this session in your agent, then review the updated session.";
    public const string Failed = "This session could not be prepared. Check your supported agent, backend, and capture settings, then try again.";
    public static bool Available(DaemonResponse hello, DaemonSettingsSnapshot? settings) => settings?.AdmissionEvidenceRequired == true
        && !hello.IsError && hello.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("methods", out var methods) && methods.ValueKind == JsonValueKind.Array
        && methods.EnumerateArray().Any(m => m.ValueKind == JsonValueKind.String && m.GetString() == Method);
    public static string Request(string entryId, string backend) => JsonSerializer.Serialize(new { entry_id = entryId, backend = backend.Trim(), confirmed = true });
    public static bool IsReady(DaemonResponse response) => !response.IsError
        && response.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("status", out var status) && status.ValueKind == JsonValueKind.String && status.GetString() == "ready_for_next_inference"
        && value.TryGetProperty("expires_at", out var expiry) && expiry.ValueKind == JsonValueKind.Number && expiry.TryGetInt64(out var timestamp) && timestamp > DateTimeOffset.UtcNow.ToUnixTimeSeconds();
}
