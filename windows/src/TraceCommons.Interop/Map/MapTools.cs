using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The tools the shell knows (<c>SourceKind</c>, SourceCandidate.swift:9-24)
/// and how each is drawn (<c>GlassTool</c>, Indicators.swift:203-245): its
/// logo and tint, or its two letters.
/// </summary>
public static class MapTools
{
    public const string ClaudeCode = "claude-code";
    public const string Codex = "codex";
    public const string GeminiCli = "gemini-cli";
    public const string Cline = "cline";
    public const string OpenCode = "opencode";

    /// <summary><c>SourceKind.allCases</c>, in its order (SourceCandidate.swift:9-14).</summary>
    public static readonly IReadOnlyList<string> All = new[] { ClaudeCode, Codex, GeminiCli, Cline, OpenCode };

    /// <summary>Whether this build knows the adapter name.</summary>
    public static bool Known(string? kind) => kind is not null && Array.IndexOf((string[])All, kind) >= 0;

    /// <summary>
    /// <c>SourceKind.displayName</c> (SourceCandidate.swift:16-24): the tools'
    /// product names, which the Mac draws as they are.
    /// </summary>
    public static string DisplayName(string kind) => kind switch
    {
        ClaudeCode => "Claude Code",
        Codex => "Codex",
        GeminiCli => "Gemini CLI",
        Cline => "Cline",
        OpenCode => "OpenCode",
        _ => kind,
    };

    /// <summary><c>SourceKind.glassTool</c> (SourceKindGlassTool.swift:8-16).</summary>
    public static GlassToolArt Art(string kind) => kind switch
    {
        ClaudeCode => GlassToolArt.ClaudeCode,
        Codex => GlassToolArt.Codex,
        GeminiCli => GlassToolArt.GeminiCli,
        Cline => GlassToolArt.Cline,
        OpenCode => GlassToolArt.OpenCode,
        _ => GlassToolArt.Other(FlowMapScene.Initials(kind)),
    };

    /// <summary>
    /// IronWire's harness ids to the tools with artwork
    /// (<c>HarnessToolArt</c>, HarnessListView.swift:377-389); null for any other.
    /// </summary>
    public static GlassToolArt? ForHarness(string id) => id switch
    {
        "claude" or "claude-code" => GlassToolArt.ClaudeCode,
        "codex" => GlassToolArt.Codex,
        "gemini" or "gemini-cli" => GlassToolArt.GeminiCli,
        "cline" => GlassToolArt.Cline,
        "opencode" => GlassToolArt.OpenCode,
        _ => null,
    };
}

/// <summary>Which tool a <see cref="GlassToolArt"/> is (Indicators.swift:203-207).</summary>
public enum GlassToolKind
{
    ClaudeCode,
    Codex,
    Antigravity,
    GeminiCli,
    Cline,
    OpenCode,
    Theia,
    Other,
}

/// <summary>
/// A tool as the glass system draws it: its logo in its tint, or its two
/// letters (<c>GlassTool</c>, Indicators.swift:203-245).
/// </summary>
public sealed record GlassToolArt(GlassToolKind Kind, string? OtherInitials = null)
{
    public static readonly GlassToolArt ClaudeCode = new(GlassToolKind.ClaudeCode);
    public static readonly GlassToolArt Codex = new(GlassToolKind.Codex);
    public static readonly GlassToolArt Antigravity = new(GlassToolKind.Antigravity);
    public static readonly GlassToolArt GeminiCli = new(GlassToolKind.GeminiCli);
    public static readonly GlassToolArt Cline = new(GlassToolKind.Cline);
    public static readonly GlassToolArt OpenCode = new(GlassToolKind.OpenCode);
    public static readonly GlassToolArt Theia = new(GlassToolKind.Theia);

    public static GlassToolArt Other(string initials) => new(GlassToolKind.Other, initials);

    /// <summary>
    /// The logo's name in <see cref="MapArt.Logos"/> (Indicators.swift:222-233);
    /// null for a tool with none (Gemini CLI, Cline, any other).
    /// </summary>
    public string? Logo => Kind switch
    {
        GlassToolKind.ClaudeCode => "claude",
        GlassToolKind.Codex => "codex",
        GlassToolKind.Antigravity => "antigravity",
        GlassToolKind.OpenCode => "openCode",
        GlassToolKind.Theia => "theia",
        _ => null,
    };

    /// <summary>The glass token the logo is drawn in (Indicators.swift:209-219).</summary>
    public string TintToken => Kind switch
    {
        GlassToolKind.ClaudeCode => "toolClaude",
        GlassToolKind.Codex => "toolCodex",
        GlassToolKind.Antigravity or GlassToolKind.GeminiCli => "toolAntigravity",
        GlassToolKind.OpenCode => "toolOpenCode",
        GlassToolKind.Theia => "toolTheia",
        _ => "textPrimary",
    };

    /// <summary>The two-letter mark (Indicators.swift:236-245).</summary>
    public string Initials => Kind switch
    {
        GlassToolKind.ClaudeCode => "CC",
        GlassToolKind.Codex => "Cx",
        GlassToolKind.Antigravity => "Ag",
        GlassToolKind.GeminiCli => "Gm",
        GlassToolKind.Cline => "Cl",
        GlassToolKind.OpenCode => "OC",
        GlassToolKind.Theia => "Th",
        _ => (OtherInitials ?? string.Empty).Length > 2 ? OtherInitials![..2] : OtherInitials ?? string.Empty,
    };
}
