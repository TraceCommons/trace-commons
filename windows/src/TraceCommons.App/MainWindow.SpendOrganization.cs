using TraceCommons.App.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// Which organization Private AI spends from (spec W19-D2 (a)): one state
/// for the window, as the Mac's AppModel holds one, shared by the Private
/// AI tab's prompt and the Settings picker the prompt leads to.
/// </summary>
public sealed partial class MainWindow
{
    private SpendOrganizationModel? _spendOrganizations;
    private SettingsView? _spendOrganizationSettings;

    /// <summary>
    /// The window's spending state, with the one credential flag the Private
    /// AI tab's sign-in sets: a pick waits for it wherever it is made.
    /// </summary>
    private SpendOrganizationModel SpendOrganizations => _spendOrganizations ??=
        new SpendOrganizationModel((m, p) => _host.CallAsync(m, p), () => _host.ConnectionGeneration, new CredentialActivity());

    /// <summary>Settings is opening: give it the picker.</summary>
    private void AttachSpendOrganizations(SettingsView settings)
    {
        _spendOrganizationSettings = settings;
        settings.ShowSpendOrganizations(SpendOrganizations);
    }

    /// <summary>
    /// The prompt's action, "Review settings": Settings, at the picker
    /// (SpendOrganizationPrompt.swift, onOpenSettings).
    /// </summary>
    private void ShowSpendOrganizationSettings()
    {
        ShowSettingsWindow();
        if (_settingsWindow is not null) _spendOrganizationSettings?.BringSpendOrganizationIntoView();
    }
}
