using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop.Map;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Traces map's style switch (owner decision W6-D1 (a), "A switch in
/// Settings, as on the Mac"): the card the Mac draws in Settings, General
/// (GeneralSection.swift; words <c>style_title</c>, <c>style_toggle</c> and
/// <c>style_caption</c> of the core's flow-map table). On, the rocket and
/// the station; off, the laptop and the orb. Only the drawing changes.
/// </summary>
internal static class MapStyleSwitch
{
    /// <summary>The card, or null when the core gave no words for it.</summary>
    public static FrameworkElement? Build()
    {
        if (FlowMapCopy.Table is not { } words)
        {
            return null;
        }

        var toggle = new ToggleSwitch
        {
            IsOn = TracesMapStyleSetting.Load() == TracesMapStyle.Launch,
            OnContent = words.StyleToggle,
            OffContent = words.StyleToggle,
        };
        AutomationProperties.SetName(toggle, words.StyleToggle);
        AutomationProperties.SetHelpText(toggle, words.StyleCaption);
        toggle.Toggled += (_, _) => TracesMapStyleSetting.Save(toggle.IsOn ? TracesMapStyle.Launch : TracesMapStyle.Standard);
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(new TextBlock { Text = words.StyleTitle, Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"] });
        panel.Children.Add(toggle);
        panel.Children.Add(new TextBlock
        {
            Text = words.StyleCaption,
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
            TextWrapping = TextWrapping.Wrap,
        });
        return panel;
    }
}
