using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>One numbered step of a getting-started card (<c>GetStartedCard.Step</c>).</summary>
public sealed record GuideStep(int Number, string Title, string Line)
{
    public string NumberText => Number.ToString(System.Globalization.CultureInfo.InvariantCulture);
}

/// <summary>
/// How Private AI works, step by step (spec W19, <c>PrivateAIGuideCard</c>,
/// InferenceViews.swift:369-405): first on the tab, in the managed table's
/// words (<c>tc_managed_copy_json</c>), until it is closed with its X; closed,
/// it stays closed on this computer. No step has a link.
/// </summary>
public sealed class PrivateAIGuide
{
    /// <summary>Each step's title key, then its line key (InferenceViews.swift:382-386).</summary>
    public static readonly IReadOnlyList<(string Title, string Line)> StepKeys = new[]
    {
        ("guide_step_earn_title", "guide_step_earn"),
        ("guide_step_start_title", "guide_step_start"),
        ("guide_step_use_title", "guide_step_use"),
    };

    private readonly IReadOnlyDictionary<string, string> _words;
    private readonly string _closedPath;

    public PrivateAIGuide(IReadOnlyDictionary<string, string> managedWords, string closedPath)
    {
        _words = managedWords;
        _closedPath = closedPath;
        IsShown = !File.Exists(closedPath);
    }

    /// <summary>The managed table, read without the daemon; empty when it cannot be read.</summary>
    public static IReadOnlyDictionary<string, string> ManagedWords()
    {
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_managed_copy_json());
        if (string.IsNullOrEmpty(json)) return words;
        try
        {
            using var doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object) return words;
            foreach (JsonProperty p in doc.RootElement.EnumerateObject())
                if (p.Value.ValueKind == JsonValueKind.String) words[p.Name] = p.Value.GetString()!;
        }
        catch (JsonException) { words.Clear(); }
        return words;
    }

    /// <summary>Where the closed mark is kept: the app's local data, or a throwaway directory.</summary>
    public static string ClosedPath(string directory) => Path.Combine(directory, "private-ai-guide-closed");

    private string Word(string key) => _words.TryGetValue(key, out string? w) ? w : string.Empty;

    public string Title => Word("guide_title");

    public string Lead => Word("guide_body");

    public IReadOnlyList<GuideStep> Steps => StepKeys.Select((keys, i) => new GuideStep(i + 1, Word(keys.Title), Word(keys.Line))).ToList();

    /// <summary>The X's name (<c>MonitorWords.close</c>).</summary>
    public string CloseName => MonitorShellCopy.Top("close");

    public bool IsShown { get; private set; }

    /// <summary>Closes the card and remembers it; a write that fails closes it for this run only.</summary>
    public void Close()
    {
        IsShown = false;
        try
        {
            string? directory = Path.GetDirectoryName(_closedPath);
            if (!string.IsNullOrEmpty(directory)) Directory.CreateDirectory(directory);
            File.WriteAllText(_closedPath, string.Empty);

        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
        }
    }
}
