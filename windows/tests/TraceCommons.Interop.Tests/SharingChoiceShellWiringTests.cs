using System;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces tree draws the Mac's "Automatic for every folder" eye and the
/// column titles (TracesViews.swift:185-198, NudgeViews.swift:69-71), and
/// the eye takes the same steps as the Mac's (SharingChoiceCard.swift:
/// keep-asks question, setup's two disclosures, the core's grant request,
/// the grant), through the first run's own disclosure sheets.
/// </summary>
public class SharingChoiceShellWiringTests
{
    private static string Source(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/" + file + ".txt");
        Assert.True(File.Exists(path), $"{file} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    [Fact]
    public void TheTreeDrawsTheColumnTitlesAndTheEye()
    {
        string tree = Source("Controls/TracesTreeView.cs");
        Assert.Contains("SharingChoice.ColumnTitles(_words)", tree, StringComparison.Ordinal);
        Assert.Contains("SharingChoice.ToolbarGlyph", tree, StringComparison.Ordinal);
        Assert.Contains("SharingChoice.ToolbarHelp(", tree, StringComparison.Ordinal);
        Assert.Contains("SharingChoice.AutomaticCount(_state.Tree.Folders, _words[\"automatic_count\"])", tree, StringComparison.Ordinal);
        Assert.Contains("_host.PressAutomaticForEveryFolderAsync()", tree, StringComparison.Ordinal);
        // The eye sits left of the order control, as on the Mac (left of History, then order).
        int eye = tree.IndexOf("Grid.SetColumn(_eyeSlot, 2);", StringComparison.Ordinal);
        int order = tree.IndexOf("Grid.SetColumn(order, 3);", StringComparison.Ordinal);
        Assert.True(eye > 0 && order > eye, "the eye is drawn between search and the order control");
    }

    [Fact]
    public void TheWindowTakesTheMacsSteps()
    {
        string window = Source("MainWindow.SharingChoice.cs");
        Assert.Contains("SharingChoice.PressAsync(", window, StringComparison.Ordinal);
        Assert.Contains("\"automatic_grant\"", window, StringComparison.Ordinal);
        Assert.Contains("SharingChoice.AskingFolders(", window, StringComparison.Ordinal);
        Assert.Contains("GrantAutomatic(witness, keepFolderAsks)", window, StringComparison.Ordinal);
        Assert.Contains("SetAskEachTime()", window, StringComparison.Ordinal);
        Assert.Contains("Flow1GrantRequest.Ask(", window, StringComparison.Ordinal);
        Assert.Contains("SharingDisclosureSheets.ShowAsync(", window, StringComparison.Ordinal);
        Assert.Contains("SettingsSharingRefused", window, StringComparison.Ordinal);
        Assert.Contains("SettingsSharingSaveFailed", window, StringComparison.Ordinal);
    }

    [Fact]
    public void TheFirstRunAndTheEyeShareOneSetOfDisclosureSheets()
    {
        string firstRun = Source("FirstRunWindow.xaml.cs");
        Assert.Contains("SharingDisclosureSheets.ShowAsync(", firstRun, StringComparison.Ordinal);
        Assert.DoesNotContain("private async Task<ContentDialogResult> Sheet(", firstRun, StringComparison.Ordinal);
    }
}
