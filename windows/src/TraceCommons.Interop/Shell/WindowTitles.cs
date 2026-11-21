namespace TraceCommons.Interop.Shell;

/// <summary>
/// Every top-level window's title, in one place, each carrying the app's
/// name from <c>brand.rs</c> (<c>tc_app_name</c>) wherever Windows shows it.
/// </summary>
/// <remarks>
/// <para>
/// Owner rule, 2026-11-20: the app refers to itself as "Near AI Gateway" in
/// Windows' navigation and switchers. A window's title is what Alt+Tab, Task
/// View, the taskbar thumbnail and a screen reader's window announcement
/// read, so a window titled only "Settings" is an entry in the switcher
/// that does not say whose it is. The Mac titles the same windows "Settings"
/// and "About ..." because its switcher shows the app, not the window.
/// </para>
/// <para>
/// The windows that stand for the whole app take the name alone; a
/// secondary window takes its own word and then the name, as Windows' own
/// apps do ("Settings - Near AI Gateway"), so two entries in the switcher
/// are never identical. About already names the app in its own word.
/// </para>
/// <para>
/// The edge rail is the one exception. It is a tool window, never shown in
/// the switchers (<c>ToolWindows.Prepare</c>), and its title is only the
/// accessible name of the rail itself.
/// </para>
/// <para>
/// <c>WindowTitleTests</c> holds every <c>Window</c> class in the app to a
/// title from here.
/// </para>
/// </remarks>
public static class WindowTitles
{
    /// <summary>The Monitor window: the app's name.</summary>
    public static string Main => MonitorWords.AppName;

    /// <summary>The first-run window: the app's name.</summary>
    public static string FirstRun => MonitorWords.AppName;

    /// <summary>The trace-folders window shown before the Monitor: the app's name.</summary>
    public static string SessionRoots => MonitorWords.AppName;

    /// <summary>The tray flyout: the app's name, as the tray icon's own tooltip leads with.</summary>
    public static string TrayFlyout => MonitorWords.AppName;

    /// <summary>The About window: "About" and the app's name (<c>about_app</c>).</summary>
    public static string About => AboutLines.Title;

    /// <summary>The Settings window: its title word, then the app's name.</summary>
    public static string Settings => Secondary(MonitorWords.SettingsTitle);

    /// <summary>
    /// The Look inside window: its own title word
    /// (<c>windows_words_copy().preview_window_title</c>, a key apart from
    /// the button's), then the app's name.
    /// </summary>
    public static string Preview => Secondary(WindowsWords.PreviewWindowTitle);

    /// <summary>The edge rail's accessible name (<c>edge_rail.rail_label</c>); never in a switcher.</summary>
    public static string EdgeRail => MonitorShellCopy.Section("edge_rail", "rail_label");

    /// <summary>
    /// A secondary window's title: its own word, then the app's name. The
    /// name alone when the word is missing, so the title is never empty and
    /// never a dangling separator.
    /// </summary>
    public static string Secondary(string window) =>
        string.IsNullOrEmpty(window) ? MonitorWords.AppName : $"{window} - {MonitorWords.AppName}";
}
