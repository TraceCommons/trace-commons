using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// A generated brush in the theme the windows show now, for code that sets
/// a colour itself. <c>Application.Current.Resources[key]</c> answers from
/// the theme the app started in: a box run that switched dark to light while
/// the app ran left the graph's pills, the transfer cards and the rail's card
/// in their dark colours, and switching High Contrast on left the rail's
/// card navy (tc-win-az9, 2026-11-19). XAML's ThemeResource follows by
/// itself; this is the same for code.
/// </summary>
/// <remarks>
/// The theme is the main window's root's (every window shares the app's
/// theme) and the system's high contrast setting (<see cref="SystemSettingWatch"/>),
/// which wins (<see cref="ThemeDictionaryKey"/>). <see cref="Bind"/> sets a
/// property now and again on every change, for elements built once;
/// <see cref="Changed"/> is for views that draw again instead.
/// </remarks>
internal static class ThemeBrushes
{
    // Held strongly: a weak reference holds only the projection's wrapper,
    // which the collector takes while the element stays on screen, so a
    // bound card kept its old colour (box live switch, 2026-11-19). Bind is
    // for elements built once that live as long as their window.
    private static readonly List<(DependencyObject Target, DependencyProperty Property, string Key)> Bound = new();
    private static readonly Dictionary<string, Brush> Cache = new();
    private static FrameworkElement? _root;
    private static string? _shown;
    private static string? _cached;

    /// <summary>Raised on the UI thread after the theme or high contrast changed and every bound brush was set again.</summary>
    public static event Action? Changed;

    /// <summary>Follows <paramref name="root"/>'s theme and the system's high contrast; once, from the main window.</summary>
    public static void Follow(FrameworkElement root)
    {
        if (_root is not null)
        {
            return;
        }

        _root = root;
        _shown = Key;
        root.ActualThemeChanged += (_, _) => Refresh();
        SystemSettingWatch.Changed += Refresh;
    }

    /// <summary>The theme dictionary in force now.</summary>
    public static string Key =>
        ThemeDictionaryKey.For(SystemSettingWatch.HighContrast, (_root?.ActualTheme ?? ThemeOfApp()) == ElementTheme.Light);

    /// <summary>The brush <paramref name="key"/> names in the theme in force now.</summary>
    public static Brush Get(string key)
    {
        // Read once per theme: a paint or a binding asks often, and the walk
        // goes through every merged dictionary.
        string theme = Key;
        if (theme != _cached)
        {
            Cache.Clear();
            _cached = theme;
        }

        if (Cache.TryGetValue(key, out Brush? known))
        {
            return known;
        }

        Brush found = Lookup(theme, key);
        Cache[key] = found;
        return found;
    }

    private static Brush Lookup(string theme, string key)
    {
        foreach (ResourceDictionary dictionary in Application.Current.Resources.MergedDictionaries.SelectMany(Flatten))
        {
            if (dictionary.ThemeDictionaries.TryGetValue(theme, out object? themed)
                && themed is ResourceDictionary scheme
                && scheme.TryGetValue(key, out object? value)
                && value is Brush brush)
            {
                return brush;
            }
        }

        return (Brush)Application.Current.Resources[key];
    }

    /// <summary>
    /// Sets <paramref name="property"/> to <paramref name="key"/>'s brush now
    /// and on every change of theme. For an element built once that lives as
    /// long as its window (the rail, the flyout, About): it is held.
    /// </summary>
    public static void Bind(DependencyObject target, DependencyProperty property, string key)
    {
        target.SetValue(property, Get(key));
        Bound.Add((target, property, key));
    }

    private static void Refresh()
    {
        string now = Key;
        if (now == _shown)
        {
            return;
        }

        _shown = now;
        foreach ((DependencyObject target, DependencyProperty property, string key) in Bound)
        {
            target.SetValue(property, Get(key));
        }

        Changed?.Invoke();
    }

    private static ElementTheme ThemeOfApp() =>
        Application.Current.RequestedTheme == ApplicationTheme.Light ? ElementTheme.Light : ElementTheme.Dark;

    private static IEnumerable<ResourceDictionary> Flatten(ResourceDictionary dictionary)
    {
        yield return dictionary;
        foreach (ResourceDictionary merged in dictionary.MergedDictionaries.SelectMany(Flatten))
        {
            yield return merged;
        }
    }
}
