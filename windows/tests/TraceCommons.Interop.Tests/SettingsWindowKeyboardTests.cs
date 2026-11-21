using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Settings window takes the keyboard when it opens (W25 audit, box
/// az9): after Activate, focus goes to the first focusable control, not the
/// bare input site, a nameless first stop that Escape never reached. Escape
/// then closes the window, as the Mac's <c>.onExitCommand</c>
/// (SettingsModal.swift:57).
/// </summary>
public class SettingsWindowKeyboardTests
{
    private static string Source(string file) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "shell-source", "TraceCommons.App", file + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    [Fact]
    public void OpeningSettingsFocusesItsFirstControl()
    {
        string window = Source("SettingsWindow.xaml.cs");
        int activate = window.IndexOf("        Activate();\n", StringComparison.Ordinal);
        int focus = window.IndexOf("ToolWindows.FocusFirst(SettingsRoot, FocusState.Programmatic, () => !_windowClosed && AppWindow.IsVisible);", StringComparison.Ordinal);
        Assert.True(activate > 0 && focus > activate, $"activate {activate}, focus {focus}");

        string tools = Source("ToolWindows.cs");
        Assert.Contains("FocusManager.FindFirstFocusableElement(root) is Control first", tools);
    }

    [Fact]
    public void EscapeClosesSettings()
    {
        string window = Source("SettingsWindow.xaml.cs");
        Assert.Contains("new Microsoft.UI.Xaml.Input.KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape }", window);
        Assert.Contains("SettingsRoot.KeyboardAccelerators.Add(escape);", window);
    }
}
