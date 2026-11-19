using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// The pieces every Settings section draws, from the existing resource keys
/// (W5 owns the tokens and styles): a card on the card surface with the
/// hairline edge, its eyebrow, body and caption text, and the failure line
/// under the control it is about. No string is written here.
/// </summary>
internal static class SettingsCards
{
    private static object Resource(string key) => Application.Current.Resources[key];

    /// <summary>A card (<c>GlassEyebrowCard</c>): the eyebrow over its content.</summary>
    public static Border Card(string eyebrow, UIElement content)
    {
        var stack = new StackPanel { Spacing = 8 };
        if (!string.IsNullOrEmpty(eyebrow))
        {
            stack.Children.Add(Text(eyebrow, "TcEyebrowTextStyle"));
        }

        stack.Children.Add(content);
        return new Border
        {
            Background = (Microsoft.UI.Xaml.Media.Brush)Resource("TcSurfaceCardBrush"),
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Resource("TcHairlineBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Resource("TcCardCornerRadius"),
            Padding = new Thickness(14, 12, 14, 12),
            Child = stack,
        };
    }

    public static TextBlock Text(string text, string style = "TcBodyTextStyle") => new()
    {
        Text = text,
        Style = (Style)Resource(style),
        TextWrapping = TextWrapping.Wrap,
    };

    /// <summary>A secondary line under a body (<c>.caption</c>, textSecondary).</summary>
    public static TextBlock Caption(string text) => Text(text, "TcCaptionTextStyle");

    /// <summary>
    /// A button named for Narrator by its own words, or by a fuller name the
    /// core gives for it.
    /// </summary>
    public static Button Button(string label, RoutedEventHandler onClick, string? accessibleName = null, bool primary = false)
    {
        var button = new Button
        {
            Content = label,
            Style = primary ? (Style)Resource("TcPrimaryButtonStyle") : (Style)Resource("TcSecondaryButtonStyle"),
        };
        AutomationProperties.SetName(button, string.IsNullOrEmpty(accessibleName) ? label : accessibleName);
        button.Click += onClick;
        return button;
    }

    /// <summary>A row of buttons, left aligned.</summary>
    public static StackPanel Row(params UIElement[] children)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (UIElement child in children)
        {
            row.Children.Add(child);
        }

        return row;
    }
}
