using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The settings screen's session-source rows, across the C ABI.
///
/// Nothing in this file is a word. The sentence crosses already assembled, so
/// this shell never fills in a template and there is no fourth place the
/// wording can drift to. The same row is rendered by the GTK and macOS shells
/// from the same Rust.
/// </summary>
public static class SourceChecks
{
    public sealed class SettingsCopy
    {
        public string Heading { get; init; } = "";
        public string Explanation { get; init; } = "";
        public string SaveFailed { get; init; } = "";
        public string ConsentSaveFailed { get; init; } = "";
        public string Unavailable { get; init; } = "";
        public string NoCandidate { get; init; } = "";
        public string WatchCandidate { get; init; } = "";
        public string ChooseFolder { get; init; } = "";
        public string ToolActionAccessibilityName { get; init; } = "";
        public string Retry { get; init; } = "";
        public EvidenceCopy? Evidence { get; init; }
        public Dictionary<string, ToolCopy> Tools { get; init; } = new();
        public TrajectoryCopy? Trajectory { get; init; }

        [JsonPropertyName("opencode_version_title")]
        public string? OpenCodeVersionTitle { get; init; }
        [JsonPropertyName("opencode_version_detail")]
        public string? OpenCodeVersionDetail { get; init; }

        public string ActionName(string action, string tool) => ToolActionAccessibilityName
            .Replace("{action}", action, StringComparison.Ordinal).Replace("{tool}", tool, StringComparison.Ordinal);
    }

    public sealed record ToolCopy(string Key, string Decline, bool UnsetScansConventional,
        string? Explanation, string? ChooseFolder);

    public sealed record TrajectoryCopy(string Title, string Explanation, string Watching, string Off, string Decline);

    public sealed record EvidenceCopy(string Missing, string Empty, string CountOne, string Count, string MostRecent,
        string Relocated, string AgeJustNow, string AgeMinutes, string AgeHour, string AgeHours, string AgeYesterday,
        string AgeDays)
    {
        public string Describe(SourceCandidate candidate, DateTimeOffset now)
        {
            string line;
            if (!candidate.Exists)
            {
                line = Missing;
            }
            else if (candidate.SessionCount == 0)
            {
                line = Empty;
            }
            else
            {
                line = candidate.SessionCount == 1
                    ? CountOne
                    : Count.Replace("{count}", candidate.SessionCount.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);
                if (candidate.MostRecent is { } then)
                {
                    line = MostRecent.Replace("{traces}", line, StringComparison.Ordinal)
                        .Replace("{age}", Age(now - then), StringComparison.Ordinal);
                }
            }

            return candidate.RelocatedByEnv ? Relocated.Replace("{evidence}", line, StringComparison.Ordinal) : line;
        }

        private string Age(TimeSpan age) => age.TotalMinutes switch
        {
            < 2 => AgeJustNow,
            < 60 => Fill(AgeMinutes, (int)age.TotalMinutes),
            < 120 => AgeHour,
            < 1440 => Fill(AgeHours, (int)age.TotalHours),
            < 2880 => AgeYesterday,
            _ => Fill(AgeDays, (int)age.TotalDays),
        };

        private static string Fill(string template, int number) =>
            template.Replace("{n}", number.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);
    }

    public static SettingsCopy? ReadSettingsCopy()
    {
        var json = NativeMethods.TakeOwnedString(NativeMethods.tc_source_settings_copy());
        if (json is null) return null;
        try { return JsonSerializer.Deserialize<SettingsCopy>(json,
            new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower }); }
        catch (JsonException) { return null; }
    }

    /// <summary>The wire key for Claude Code's session source.</summary>
    public const string Claude = "claude";

    /// <summary>The wire key for Codex's session source.</summary>
    public const string Codex = "codex";

    /// <summary>
    /// One tool's row, from <c>get_settings</c>'s <c>*_source_mode</c>.
    ///
    /// PASS THE MODE, NOT THE BOOLEAN. <c>ClaudeRootConfigured</c> is
    /// (mode == "watch"), so it is false for a source the contributor
    /// declared OFF as well as for one nobody was asked about. This shell
    /// rendered one sentence on that false branch and so told a contributor
    /// who does not use Claude Code that its sessions were being read from
    /// the usual place. Nothing is read from an off source. What an UNSET
    /// source means is per tool -- claude and codex are scanned where they
    /// usually live, gemini and cline construct no adapter and open nothing
    /// -- so never carry one tool's unset sentence to another. The Rust
    /// picks the words from the mode and the tool together.
    ///
    /// Null when the call failed -- an unknown tool key, or a caught panic.
    /// The caller shows nothing rather than a word this shell made up.
    /// </summary>
    public static string? CheckLine(string tool, string sourceMode) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_source_check_line(tool, sourceMode));
}
