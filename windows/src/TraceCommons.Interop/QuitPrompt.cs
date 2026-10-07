using System;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What quitting says, decoded from the core (<c>quit_copy.rs</c>, exported
/// as <c>tc_quit_prompt_json</c>). Every word is the core's.
/// </summary>
/// <remarks>
/// The true sentence depends on which process is doing the watching, and the
/// core reads that off the handle rather than taking the shell's word for it.
/// This app hosts the daemon in-process, so with a daemon running the prompt
/// is the hosting one; with none started, or one already stopped, it is the
/// one that claims nothing stops.
/// </remarks>
public sealed record QuitPrompt
{
    /// <summary><c>hosting</c>, <c>attached</c> or <c>unavailable</c>.</summary>
    [JsonPropertyName("role")] public string Role { get; init; } = "";
    [JsonPropertyName("title")] public string Title { get; init; } = "";
    [JsonPropertyName("body")] public string Body { get; init; } = "";
    [JsonPropertyName("confirm")] public string Confirm { get; init; } = "";
    [JsonPropertyName("cancel")] public string Cancel { get; init; } = "";

    /// <summary>
    /// The prompt for a process with no watcher at all: the NULL-handle
    /// answer. Null only if the core could not produce one.
    /// </summary>
    public static QuitPrompt? WithoutWatcher() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_quit_prompt_json(IntPtr.Zero)));

    internal static QuitPrompt? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json)) return null;
        try
        {
            QuitPrompt? prompt = JsonSerializer.Deserialize<QuitPrompt>(json);
            // All or nothing: a prompt missing any of its words is not shown
            // with a gap where the core's sentence should be.
            return prompt is null
                || string.IsNullOrEmpty(prompt.Title)
                || string.IsNullOrEmpty(prompt.Body)
                || string.IsNullOrEmpty(prompt.Confirm)
                || string.IsNullOrEmpty(prompt.Cancel)
                ? null
                : prompt;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
