using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Where the spending prompt's action leads (review round 2, blocker 1): to
/// Settings, at the organization picker, as the Mac's "Review settings" does
/// (SpendOrganizationPrompt.swift <c>onOpenSettings</c>, PrivateAISection.swift
/// :33-35), over one spending state for the window. The WinUI shell is not
/// built on this runner, so its wiring is read from source.
/// </summary>
public sealed class SpendOrganizationShellWiringTests
{
    private static string ShellSource(string relativePath) =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt"));

    private static string Code(string relativePath) => Regex.Replace(ShellSource(relativePath), @"^\s*//.*$", string.Empty, RegexOptions.Multiline);

    [Fact]
    public void ThePromptsActionAsksForThePickerNotPlainSettings()
    {
        string ledger = Code("TraceCommons.App/Controls/PrivateInferenceView.Ledger.cs");
        Assert.Matches(
            new Regex(@"void OnSpendPromptAction\([^)]*\)\s*=>\s*ChooseSpendOrganizationRequested\?\.Invoke"),
            ledger);
        Assert.DoesNotMatch(new Regex(@"void OnSpendPromptAction\([^)]*\)\s*=>\s*OpenSettingsRequested"), ledger);
    }

    [Fact]
    public void TheWindowOpensSettingsAtThePicker()
    {
        string window = Code("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains("new PrivateInferenceView(_host, SpendOrganizations)", window, StringComparison.Ordinal);
        Assert.Matches(new Regex(@"ChooseSpendOrganizationRequested \+= \(_, _\) => ShowSpendOrganizationSettings\(\)"), window);

        string spend = Code("TraceCommons.App/MainWindow.SpendOrganization.cs");
        Assert.Matches(
            new Regex(@"void ShowSpendOrganizationSettings\(\)\s*\{\s*ShowSettingsWindow\(\);[^}]*BringSpendOrganizationIntoView\(\)"),
            spend);

        // Every Settings window gets the picker, over the window's one state.
        string shell = Code("TraceCommons.App/MainWindow.Shell.cs");
        Assert.Matches(new Regex(@"new SettingsView\([^)]*\);(?:(?!_settingsWindow = new).)*AttachSpendOrganizations\(settings\);", RegexOptions.Singleline), shell);
        Assert.Contains("settings.ShowSpendOrganizations(SpendOrganizations)", spend, StringComparison.Ordinal);
    }

    [Fact]
    public void TheSettingsPickerSitsInPrivateAIBeforeTheTerminalPicker()
    {
        string settings = Code("TraceCommons.App/Controls/SettingsView.SpendOrganization.cs");
        Assert.Matches(
            new Regex(@"section\.Children\.Insert\(section\.Children\.IndexOf\(ManagedTerminalHost\), _spendOrganizationCard\)"),
            settings);
        string card = Code("TraceCommons.App/Controls/Settings/SpendOrganizationCard.cs");
        // The picker's echo of the selection the card set is not a pick.
        Assert.Matches(new Regex(@"if \(_filling\) return;\s*await _model\.PickAsync\(_picker\.SelectedIndex\);"), card);
        Assert.Contains("_picker.IsEnabled = _model.CanPick;", card, StringComparison.Ordinal);
    }

    // One credential flag: the tab's sign-in sets the one the picker waits
    // for, wherever the picker is drawn (AppModel.swift:1076, credentialBusy).
    [Fact]
    public void TheTabsSignInIsTheFlagThePickerWaitsFor()
    {
        string view = Code("TraceCommons.App/Controls/PrivateInferenceView.xaml.cs");
        Assert.Contains("new PrivateInferenceViewModel(host, credential: _spendPrompt.Credential)", view, StringComparison.Ordinal);
        Assert.Contains("_spendPrompt.SwitchSettled += OnSpendSwitchSettled;", view, StringComparison.Ordinal);
        string ledger = Code("TraceCommons.App/Controls/PrivateInferenceView.Ledger.cs");
        Assert.Matches(new Regex(@"void OnSpendSwitchSettled\(\)\s*\{\s*try\s*\{\s*await ViewModel\.LoadBalanceAsync\(fresh: true\);"), ledger);
    }
}
