using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The Monitor window's words, read from the core's monitor table
/// (<c>preview_copy::monitor_screens_copy</c>, <c>tc_monitor_screens_copy_json</c>):
/// its top-level words and its <c>shell</c> table, the same fields the Mac
/// (<c>MonitorScreensCopy</c>, <c>MonitorShellCopy</c>) and GTK read. Empty if
/// the core could not produce the table, so a control then says nothing
/// rather than something typed here.
/// </summary>
public static class MonitorShellCopy
{
    private static readonly Lazy<Table> Words = new(() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json())));

    /// <summary>The watcher's pause control.</summary>
    public static string PauseWatcher => Shell("pause_watcher");

    /// <summary>The watcher's resume control.</summary>
    public static string ResumeWatcher => Shell("resume_watcher");

    /// <summary>Settings' projects list, before any project was seen.</summary>
    public static string ProjectsEmpty => Shell("projects_empty");

    /// <summary>A word from the table's <c>shell</c> object; empty when absent.</summary>
    public static string Shell(string key) => Words.Value.Shell.TryGetValue(key, out string? word) ? word : string.Empty;

    /// <summary>A top-level word of the table; empty when absent.</summary>
    public static string Top(string key) => Words.Value.Top.TryGetValue(key, out string? word) ? word : string.Empty;

    /// <summary>The table's string fields, top level and <c>shell</c>.</summary>
    internal sealed record Table(IReadOnlyDictionary<string, string> Top, IReadOnlyDictionary<string, string> Shell);

    /// <summary>
    /// Decodes the table. Anything but a JSON object reads as no words at all.
    /// </summary>
    internal static Table Parse(string? json)
    {
        var top = new Dictionary<string, string>(StringComparer.Ordinal);
        var shell = new Dictionary<string, string>(StringComparer.Ordinal);
        if (string.IsNullOrEmpty(json))
        {
            return new Table(top, shell);
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return new Table(top, shell);
            }

            foreach (JsonProperty field in doc.RootElement.EnumerateObject())
            {
                if (field.Value.ValueKind == JsonValueKind.String && field.Value.GetString() is { } word)
                {
                    top[field.Name] = word;
                }
                else if (field.Name == "shell" && field.Value.ValueKind == JsonValueKind.Object)
                {
                    foreach (JsonProperty inner in field.Value.EnumerateObject())
                    {
                        if (inner.Value.ValueKind == JsonValueKind.String && inner.Value.GetString() is { } shellWord)
                        {
                            shell[inner.Name] = shellWord;
                        }
                    }
                }
            }
        }
        catch (JsonException)
        {
            top.Clear();
            shell.Clear();
        }

        return new Table(top, shell);
    }
}
