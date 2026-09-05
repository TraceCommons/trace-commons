using System.Linq;
using System.Text.Json;
namespace TraceCommons.Interop;
public static class AdmissionPreparation
{
    public const string Method = "prepare_admission_session";
    public static AdmissionCopy? Copy => WitnessSurface.Copy()?.Admission;
    public static string Heading => Copy?.Heading ?? "";
    public static string Disclosure => Copy?.Disclosure ?? "";
    public static string Success => Copy?.Ready ?? "";
    public static string Failed => Copy?.Failed ?? "";
    public static bool Available(DaemonResponse hello, DaemonSettingsSnapshot? settings) => Copy is not null && settings?.AdmissionEvidenceRequired == true
        && !hello.IsError && hello.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("methods", out var methods) && methods.ValueKind == JsonValueKind.Array
        && methods.EnumerateArray().Any(m => m.ValueKind == JsonValueKind.String && m.GetString() == Method);
    public static string Request(string entryId, string backend) => JsonSerializer.Serialize(new { entry_id = entryId, backend = backend.Trim(), confirmed = true });
    public static bool IsReady(DaemonResponse response) => !response.IsError
        && response.Result is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("view", out var view) && view.ValueKind == JsonValueKind.Object
        && view.TryGetProperty("ready", out var ready) && ready.ValueKind == JsonValueKind.True;
}
