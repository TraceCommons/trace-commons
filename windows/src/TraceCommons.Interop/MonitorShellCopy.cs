using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// #1146's watcher and Settings words, read from the core's monitor table
/// (<c>preview_copy::MonitorShellCopy</c>, <c>tc_monitor_screens_copy_json</c>),
/// the same fields GTK reads. Empty if the core could not produce the table,
/// so a control then says nothing rather than something typed here.
/// </summary>
public static class MonitorShellCopy
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Shell = new(Read);

    /// <summary>The watcher's pause control.</summary>
    public static string PauseWatcher => Word("pause_watcher");

    /// <summary>The watcher's resume control.</summary>
    public static string ResumeWatcher => Word("resume_watcher");

    /// <summary>Settings' projects list, before any project was seen.</summary>
    public static string ProjectsEmpty => Word("projects_empty");

    private static string Word(string key) =>
        Shell.Value.TryGetValue(key, out string? word) ? word : string.Empty;

    private static IReadOnlyDictionary<string, string> Read()
    {
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json());
        if (string.IsNullOrEmpty(json))
        {
            return words;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("shell", out JsonElement shell)
                && shell.ValueKind == JsonValueKind.Object)
            {
                foreach (JsonProperty field in shell.EnumerateObject())
                {
                    if (field.Value.ValueKind == JsonValueKind.String
                        && field.Value.GetString() is { } word)
                    {
                        words[field.Name] = word;
                    }
                }
            }
        }
        catch (JsonException)
        {
            words.Clear();
        }

        return words;
    }
}
