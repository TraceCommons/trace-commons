using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > General, the appearance card (W17-D1 (a); <c>GeneralSection</c>,
/// GeneralSection.swift): Light, Dark or System as one segmented choice,
/// applied at once to every window and kept for the next launch. Its words
/// are the core's <c>tc_settings_general_copy_json</c> (DRAFT but for
/// Appearance); the card is not drawn when they did not arrive. The
/// section's other cards (the edge rail, promotions and the credit
/// animation switch) are the Home and tray track's (W11-W13).
/// </summary>
public sealed class GeneralSection : UserControl
{
    public GeneralSection()
    {
        if (SettingsGeneralCopy.Live is not { } words)
        {
            return;
        }

        var choice = new RadioButtons
        {
            MaxColumns = 3,
        };
        AutomationProperties.SetName(choice, words.Appearance);
        choice.Items.Add(words.Light);
        choice.Items.Add(words.Dark);
        choice.Items.Add(words.System);
        choice.SelectedIndex = Index(AppearanceChoice.Shared.Current);
        choice.SelectionChanged += (_, _) =>
        {
            // Nothing selected is not a choice: never write System for it.
            if (choice.SelectedIndex < 0)
            {
                return;
            }

            Appearance picked = choice.SelectedIndex switch
            {
                0 => Appearance.Light,
                1 => Appearance.Dark,
                _ => Appearance.System,
            };
            AppearanceChoice.Shared.Choose(picked);
        };
        Content = SettingsCards.Card(words.Appearance, choice);
    }

    private static int Index(Appearance appearance) => appearance switch
    {
        Appearance.Light => 0,
        Appearance.Dark => 1,
        _ => 2,
    };

    /// <summary>The element theme a choice sets on a window's root; Default follows the system.</summary>
    public static ElementTheme Theme(Appearance appearance) => appearance switch
    {
        Appearance.Light => ElementTheme.Light,
        Appearance.Dark => ElementTheme.Dark,
        _ => ElementTheme.Default,
    };
}
