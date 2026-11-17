using System;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>What a quit request does.</summary>
public enum QuitDecision
{
    /// <summary>Nothing live: quit without asking.</summary>
    QuitNow,
    /// <summary>Show the prompt and quit only if it is confirmed.</summary>
    Ask,
    /// <summary>Something is live but the prompt could not be read: do not quit.</summary>
    Refuse,
}

/// <summary>
/// The quit prompt, decoded from the core (<c>quit_copy.rs</c>, exported as
/// <c>tc_quit_reason_prompt_json</c>). Every word is the core's.
/// </summary>
/// <remarks>
/// Quitting asks only when it would stop something live (compute, or
/// Private AI answering model calls), and the body is one short sentence
/// naming which (owner, 2026-11-12).
/// </remarks>
public sealed record QuitPrompt
{
    [JsonPropertyName("title")] public string Title { get; init; } = "";
    [JsonPropertyName("body")] public string Body { get; init; } = "";
    [JsonPropertyName("confirm")] public string Confirm { get; init; } = "";
    [JsonPropertyName("cancel")] public string Cancel { get; init; } = "";

    /// <summary>
    /// Decides a quit. <paramref name="promptJson"/> is asked only when
    /// something is live, so a failed read refuses the quit rather than
    /// becoming a silent one. It defaults to the core's export.
    /// </summary>
    public static QuitDecision Decide(
        bool stopsCompute,
        bool stopsPrivateAI,
        out QuitPrompt? prompt,
        Func<bool, bool, string?>? promptJson = null)
    {
        prompt = null;
        if (!stopsCompute && !stopsPrivateAI) return QuitDecision.QuitNow;
        promptJson ??= (compute, privateAI) => NativeMethods.TakeOwnedString(
            NativeMethods.tc_quit_reason_prompt_json(compute ? 1 : 0, privateAI ? 1 : 0));
        prompt = Parse(promptJson(stopsCompute, stopsPrivateAI));
        return prompt is null ? QuitDecision.Refuse : QuitDecision.Ask;
    }

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
