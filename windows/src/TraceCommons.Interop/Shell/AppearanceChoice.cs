using System;
using System.IO;

namespace TraceCommons.Interop.Shell;

/// <summary>The app's appearance (<c>GlassAppearance</c>, GlassTheme.swift:98-138).</summary>
public enum Appearance
{
    Light,
    Dark,
    System,
}

/// <summary>
/// The appearance chosen in Settings > General (W17-D1 (a)): Light, Dark or
/// the system's, System until one is chosen. It applies at once and is kept
/// for the next launch, as the Mac keeps <c>TCAppearance</c> in its
/// defaults. Kept in its own small file beside the app's other local state;
/// a sample launch writes nothing.
/// </summary>
public sealed class AppearanceChoice
{
    private readonly string _path;
    private Appearance _current;

    public AppearanceChoice(string path)
    {
        _path = path ?? throw new ArgumentNullException(nameof(path));
        _current = Parse(TryRead(path));
    }

    private static readonly Lazy<AppearanceChoice> SharedChoice = new(() => new AppearanceChoice(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "TraceCommons",
        "appearance.txt")));

    /// <summary>The one instance every window reads, so a choice applies to all at once.</summary>
    public static AppearanceChoice Shared => SharedChoice.Value;

    /// <summary>Choices hold for this run only and are never written.</summary>
    public bool ReadOnly { get; set; }

    /// <summary>Raised after the choice changes, for every window to apply it.</summary>
    public event Action<Appearance>? Changed;

    public Appearance Current => _current;

    /// <summary>Stores the choice (unless read-only) and applies it (<c>choose()</c>).</summary>
    public void Choose(Appearance appearance)
    {
        if (appearance == _current)
        {
            return;
        }

        _current = appearance;
        if (!ReadOnly)
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
                File.WriteAllText(_path, Raw(appearance));
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                // Kept for this run; the next launch follows the system.
            }
        }

        Changed?.Invoke(appearance);
    }

    /// <summary>The Mac's raw values: <c>light</c>, <c>dark</c>, <c>system</c>.</summary>
    public static string Raw(Appearance appearance) => appearance switch
    {
        Appearance.Light => "light",
        Appearance.Dark => "dark",
        _ => "system",
    };

    /// <summary>A stored value read back; anything else is System (<c>stored</c>).</summary>
    public static Appearance Parse(string? raw) => raw?.Trim() switch
    {
        "light" => Appearance.Light,
        "dark" => Appearance.Dark,
        _ => Appearance.System,
    };

    private static string? TryRead(string path)
    {
        try
        {
            return File.ReadAllText(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return null;
        }
    }
}
