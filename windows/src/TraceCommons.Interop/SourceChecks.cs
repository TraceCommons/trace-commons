using System;
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
        [JsonPropertyName("opencode_version_title")]
        public string? OpenCodeVersionTitle { get; init; }
        [JsonPropertyName("opencode_version_detail")]
        public string? OpenCodeVersionDetail { get; init; }
    }

    public static SettingsCopy? ReadSettingsCopy()
    {
        var json = NativeMethods.TakeOwnedString(NativeMethods.tc_source_settings_copy());
        if (json is null) return null;
        try { return JsonSerializer.Deserialize<SettingsCopy>(json,
            new JsonSerializerOptions { PropertyNameCaseInsensitive = true }); }
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
