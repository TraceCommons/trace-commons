using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The spending-organization picker in Settings' Private AI section (spec
/// W19, PrivateAISection.swift:33-35), where the Private AI tab's prompt
/// leads. A partial of its own, filled in code beside the terminal picker,
/// so the Settings layout (#2219) and this card merge without touching each
/// other: #2219 hosts the same card in <c>BodyPrivateAI</c>.
/// </summary>
public sealed partial class SettingsView
{
    private SpendOrganizationCard? _spendOrganizationCard;

    /// <summary>
    /// Draws the picker over the window's one spending state, so a choice
    /// here ends the prompt on the tab. Called once, as the window opens.
    /// </summary>
    public void ShowSpendOrganizations(SpendOrganizationModel model)
    {
        if (_spendOrganizationCard is not null) return;
        _spendOrganizationCard = new SpendOrganizationCard(model);
        // In the Mac's order: before the terminal picker (PrivateAISection.swift).
        if (ManagedTerminalHost.Parent is Panel section)
        {
            section.Children.Insert(section.Children.IndexOf(ManagedTerminalHost), _spendOrganizationCard);
        }
    }

    /// <summary>Scrolls to the picker and focuses it: the prompt's action.</summary>
    public void BringSpendOrganizationIntoView() => _spendOrganizationCard?.BringPickerIntoView();
}
