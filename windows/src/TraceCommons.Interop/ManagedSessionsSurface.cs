using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

public sealed record ManagedAccount(
    string Id, string Tool, string Connection, string Label,
    [property: JsonPropertyName("auth_state")] string AuthState);

public sealed record ManagedSession(
    string Id, string Tool, string Connection,
    [property: JsonPropertyName("account_id")] string AccountId,
    [property: JsonPropertyName("account_label")] string AccountLabel,
    [property: JsonPropertyName("project_label")] string ProjectLabel,
    string State, string Purpose, string? Model = null,
    [property: JsonPropertyName("collect_traces")] bool CollectTraces = false,
    string Collection = "off")
{
    [JsonIgnore] public bool HoldsAccount => State != "exited" && State != "failed";
}

public sealed record ManagedTool(string Tool, bool Installed,
    [property: JsonPropertyName("install_url")] string InstallUrl);

public sealed record ManagedCapabilities(
    [property: JsonPropertyName("managed_launch")] bool ManagedLaunch,
    [property: JsonPropertyName("terminal_launch")] bool TerminalLaunch,
    [property: JsonPropertyName("terminal_destination")] string? TerminalDestination);

public sealed record ManagedSnapshot(
    ulong Revision, ManagedAccount[] Accounts, ManagedSession[] Sessions,
    Dictionary<string, ulong> Generations, ManagedCapabilities Capabilities,
    Dictionary<string, string> Copy, ManagedTool[]? Tools = null)
{
    public ManagedTool[] VersionRecoveryTools(string? attempted) => (Tools ?? []).Where(t => t.Installed && (attempted is null || t.Tool == attempted)).ToArray();
    public bool CanLaunch(string tool) => Tools?.FirstOrDefault(t => t.Tool == tool)?.Installed != false;
    public string[] SetupTools => Tools is null ? ["claude", "codex", "opencode"] : Tools.Where(t => t.Installed).Select(t => t.Tool).ToArray();
    public static string[] Connections(string tool) => tool == "opencode" ? ["api_key", "near_ai"] : ["subscription", "api_key", "near_ai"];
    public string Text(string key) => Copy.GetValueOrDefault(key, string.Empty);
    public static ManagedSnapshot? Parse(string json) => JsonSerializer.Deserialize<ManagedSnapshot>(json,
        new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
}
