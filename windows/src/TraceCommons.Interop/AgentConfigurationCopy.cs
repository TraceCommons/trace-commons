using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>The optional agent handoff from the shared first-run copy table.</summary>
public sealed record AgentConfigurationCopy
{
    [JsonPropertyName("action")] public string Action { get; init; } = "";
    [JsonPropertyName("title")] public string Title { get; init; } = "";
    [JsonPropertyName("body")] public string Body { get; init; } = "";
    [JsonPropertyName("prerequisite")] public string Prerequisite { get; init; } = "";
    [JsonPropertyName("prompt")] public string Prompt { get; init; } = "";
    [JsonPropertyName("copy")] public string Copy { get; init; } = "";
    [JsonPropertyName("copied")] public string Copied { get; init; } = "";
    [JsonPropertyName("close")] public string Close { get; init; } = "";

    public static AgentConfigurationCopy? Load() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_first_run_copy_json()));

    internal static AgentConfigurationCopy? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json)) return null;
        try
        {
            var copy = JsonSerializer.Deserialize<FirstRunPayload>(json)?.AgentConfiguration;
            if (copy is null) return null;
            foreach (string? value in new[] { copy.Action, copy.Title, copy.Body, copy.Prerequisite,
                copy.Prompt, copy.Copy, copy.Copied, copy.Close })
            {
                if (string.IsNullOrWhiteSpace(value)) return null;
            }
            return copy;
        }
        catch (JsonException) { return null; }
    }

    private sealed record FirstRunPayload
    {
        [JsonPropertyName("agent_configuration")]
        public AgentConfigurationCopy? AgentConfiguration { get; init; }
    }
}
