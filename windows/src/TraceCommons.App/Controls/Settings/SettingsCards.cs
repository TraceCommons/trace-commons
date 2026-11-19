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
        return new Border { Style = Styles?["SettingsCardStyle"] as Style, Child = stack };
    }

    /// <summary>
    /// The Settings window's own resources, whose styles take their brushes
    /// as theme resources, so a card follows the window's theme.
    /// </summary>
    public static ResourceDictionary? Styles { get; set; }

    /// <summary>A hairline between groups of the section list.</summary>
    public static Border Hairline()
    {
        var hairline = new Border { Style = Styles?["SettingsHairlineStyle"] as Style };
        AutomationProperties.SetAccessibilityView(hairline, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return hairline;
    }

    /// <summary>
    /// What a section draws where its read has not answered: a spinner while
    /// it is out, then the core's failure line with Retry (<c>SettingsReadNotice</c>).
    /// </summary>
    public static UIElement ReadNotice(bool failed, RoutedEventHandler retry)
    {
        if (!failed)
        {
            return new ProgressRing { IsActive = true, Width = 16, Height = 16, HorizontalAlignment = HorizontalAlignment.Left };
        }

        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(Caption(TraceCommons.Interop.SettingsReadWords.Failed));
        if (TraceCommons.Interop.SettingsReadWords.Retry is { } word)
        {
            row.Children.Add(Button(word, retry));
        }

        return row;
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

    /// <summary>
    /// A glass chip (<c>GlassChip(glass:muted:)</c>, Indicators.swift:70-90):
    /// mono words, 3 by 9 padding in a capsule, accent ink, or secondary ink
    /// on the inset tint while muted. One element for Narrator, named by its
    /// words.
    /// </summary>
    public static Border GlassChip(string word, bool muted)
    {
        var text = new TextBlock
        {
            Text = word,
            Style = (Style)Resource("TcMonoChipTextStyle"),
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Resource(muted ? "TcInkSecondaryBrush" : "TcAccentTextBrush"),
            MaxLines = 1,
        };
        var chip = new Border
        {
            Child = text,
            Padding = new Thickness(9, 3, 9, 3),
            CornerRadius = (CornerRadius)Resource("TcPillCornerRadius"),
            BorderThickness = new Thickness(0.5),
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Resource("TcHairlineBrush"),
            Background = muted ? (Microsoft.UI.Xaml.Media.Brush)Resource("TcSurfaceInsetBrush") : null,
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetName(chip, word);
        AutomationProperties.SetAccessibilityView(text, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return chip;
    }

    /// <summary>
    /// A status chip (<c>GlassChip(_:status:)</c>, Indicators.swift:91-104):
    /// a 6 pt dot then mono words in the status's ink, 3 by 9 padding, on the
    /// inset tint (flat pills carry no border).
    /// </summary>
    public static Border StatusChip(string word, bool on)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        row.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 6,
            Height = 6,
            Fill = (Microsoft.UI.Xaml.Media.Brush)Resource(on ? "TcStatusOnBrush" : "TcStatusOffBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        });
        row.Children.Add(new TextBlock
        {
            Text = word,
            Style = (Style)Resource("TcMonoChipTextStyle"),
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Resource(on ? "TcStatusOnTextBrush" : "TcInkSecondaryBrush"),
            MaxLines = 1,
        });
        var chip = new Border
        {
            Child = row,
            Padding = new Thickness(9, 3, 9, 3),
            CornerRadius = (CornerRadius)Resource("TcPillCornerRadius"),
            Background = (Microsoft.UI.Xaml.Media.Brush)Resource("TcSurfaceInsetBrush"),
            VerticalAlignment = VerticalAlignment.Center,
        };
        AutomationProperties.SetName(chip, word);
        AutomationProperties.SetAccessibilityView(row, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        return chip;
    }

    /// <summary>
    /// A card whose head is a title with something on its right
    /// (<c>GlassEyebrowCard(_:title:accessory:)</c>): the eyebrow when the core
    /// gives one, then the title and the accessory on one line.
    /// </summary>
    public static Border TitledCard(string eyebrow, string title, UIElement? accessory, UIElement content)
    {
        var head = new Grid { ColumnSpacing = 8 };
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var titleText = Text(title, "TcTitleCardTextStyle");
        AutomationProperties.SetHeadingLevel(titleText, Microsoft.UI.Xaml.Automation.Peers.AutomationHeadingLevel.Level3);
        head.Children.Add(titleText);
        if (accessory is FrameworkElement element)
        {
            Grid.SetColumn(element, 1);
            head.Children.Add(element);
        }

        var stack = new StackPanel { Spacing = 8 };
        if (!string.IsNullOrEmpty(eyebrow))
        {
            stack.Children.Add(Text(eyebrow, "TcEyebrowTextStyle"));
        }

        stack.Children.Add(head);
        stack.Children.Add(content);
        return new Border { Style = Styles?["SettingsCardStyle"] as Style, Child = stack };
    }
}
