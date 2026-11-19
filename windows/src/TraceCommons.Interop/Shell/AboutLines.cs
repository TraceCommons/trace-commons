using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The About window's lines (spec W24, <c>AboutLines</c>,
/// macos/Sources/TraceCommonsApp/Views/AboutWindowView.swift:43-73). The words
/// are the core's; the version is the build's own.
/// </summary>
public static class AboutLines
{
    private static readonly Lazy<string?> ShellWordsJson = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_shell_words_copy_json()));

    /// <summary>The window's title and the menu rows' word: "About" and the app's name (<c>about_app</c>).</summary>
    public static string Title => MonitorShellCopy.Top("about_app");

    /// <summary>The app's name, from <c>brand.rs</c>.</summary>
    public static string Name => MonitorWords.AppName;

    /// <summary>The Copy button (<c>SettingsLegacyWords.copy</c>, the Settings table's <c>copy</c>).</summary>
    public static string CopyWord => MonitorWords.WordAt(ShellWordsJson.Value, "settings", "copy");

    /// <summary>
    /// "&lt;version&gt; (&lt;build&gt;)", or the version alone when the build is
    /// missing or the same (AboutWindowView.swift:59-63).
    /// </summary>
    public static string Line(string? shortVersion, string? build)
    {
        string version = shortVersion ?? string.Empty;
        if (string.IsNullOrEmpty(build) || build == version) return version;
        return version.Length == 0 ? build : $"{version} ({build})";
    }

    /// <summary>
    /// A package's four-part version as the version and its build: the
    /// release stamps the version with a zero revision (make-msix.ps1), so the
    /// build is drawn only when a revision is set.
    /// </summary>
    public static (string Short, string? Build) FromPackageVersion(ushort major, ushort minor, ushort build, ushort revision)
    {
        string version = $"{major}.{minor}.{build}";
        return (version, revision == 0 ? null : $"{version}.{revision}");
    }

    /// <summary>The version part of an assembly's informational version, without a "+revision".</summary>
    public static string ShortVersionFrom(string? informational)
    {
        if (string.IsNullOrEmpty(informational)) return string.Empty;
        int plus = informational.IndexOf('+', StringComparison.Ordinal);
        return plus < 0 ? informational : informational[..plus];
    }
}

/// <summary>What the About window shows, and its one action.</summary>
public sealed class AboutModel
{
    private readonly Action<string> _clipboard;

    public AboutModel(string? shortVersion, string? build, string? copyright, Action<string> clipboard)
    {
        Version = AboutLines.Line(shortVersion, build);
        Copyright = copyright ?? string.Empty;
        _clipboard = clipboard;
    }

    /// <summary>The version line exactly as shown.</summary>
    public string Version { get; }

    public string Copyright { get; }

    /// <summary>The copyright line is drawn only when there is one (AboutWindowView.swift:28).</summary>
    public bool ShowsCopyright => Copyright.Length > 0;

    /// <summary>Copies exactly the shown version line (<c>AboutLines.copyVersion</c>).</summary>
    public void CopyVersion() => _clipboard(Version);
}
