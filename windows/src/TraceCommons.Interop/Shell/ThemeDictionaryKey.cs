namespace TraceCommons.Interop.Shell;

/// <summary>The theme dictionary a brush is read from.</summary>
public static class ThemeDictionaryKey
{
    /// <summary>
    /// <c>HighContrast</c> under a high contrast theme, else <c>Light</c> or
    /// WinUI's <c>Default</c> (the dark one) by the window's theme.
    /// </summary>
    public static string For(bool highContrast, bool light) => highContrast ? "HighContrast" : light ? "Light" : "Default";
}
