using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;

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
    public static UIElement ReadNotice(bool failed, RoutedEventHandler retry, string? retryKey = null)
    {
        if (!failed)
        {
            return new ProgressRing { IsActive = true, Width = 16, Height = 16, HorizontalAlignment = HorizontalAlignment.Left };
        }

        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(Caption(TraceCommons.Interop.SettingsReadWords.Failed));
        if (TraceCommons.Interop.SettingsReadWords.Retry is { } word)
        {
            Button button = Button(word, retry);
            Key(button, retryKey);
            row.Children.Add(button);
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
    /// One of the Settings window's own styles (<see cref="Styles"/>), whose
    /// brushes are theme resources: the only way code here gets a brush, so
    /// every one follows the window's theme and a live theme change.
    /// </summary>
    public static Style? Named(string key) => Styles?[key] as Style;

    /// <summary>A caption in the secondary ink (<c>GlassColor.textSecondary</c>).</summary>
    public static TextBlock SecondaryCaption(string text) => new()
    {
        Text = text,
        Style = Named("SettingsSecondaryCaptionStyle") ?? (Style)Resource("TcCaptionTextStyle"),
    };

    /// <summary>A caption in the tertiary ink (<c>GlassColor.textTertiary</c>).</summary>
    public static TextBlock TertiaryCaption(string text) => new()
    {
        Text = text,
        Style = Named("SettingsTertiaryCaptionStyle") ?? (Style)Resource("TcCaptionTextStyle"),
    };

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
            Style = Named(muted ? "SettingsChipSecondaryTextStyle" : "SettingsChipAccentTextStyle"),
            MaxLines = 1,
        };
        var chip = new Border
        {
            Child = text,
            Style = Named(muted ? "SettingsGlassChipMutedStyle" : "SettingsGlassChipStyle"),
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
            Style = Named(on ? "SettingsStatusDotOnStyle" : "SettingsStatusDotOffStyle"),
        });
        row.Children.Add(new TextBlock
        {
            Text = word,
            Style = Named(on ? "SettingsChipOnTextStyle" : "SettingsChipSecondaryTextStyle"),
            MaxLines = 1,
        });
        var chip = new Border
        {
            Child = row,
            Style = Named("SettingsStatusChipStyle"),
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

    /// <summary>
    /// Gives a control its focus key (its UI Automation id), by which
    /// <see cref="Replace"/> finds it again in a redrawn card.
    /// </summary>
    public static T Key<T>(T control, string? key)
        where T : FrameworkElement
    {
        if (!string.IsNullOrEmpty(key))
        {
            AutomationProperties.SetAutomationId(control, key);
        }

        return control;
    }

    /// <summary>
    /// The focus key of the control that has focus, when it sits inside
    /// <paramref name="host"/>; null otherwise. Read it before an action
    /// that turns the control off, since WinUI moves focus away then.
    /// </summary>
    public static string? FocusedKey(UIElement host)
    {
        if (host.XamlRoot is null || FocusManager.GetFocusedElement(host.XamlRoot) is not DependencyObject focused)
        {
            return null;
        }

        string? key = null;
        for (DependencyObject? at = focused; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (ReferenceEquals(at, host))
            {
                return key;
            }

            if (key is null && AutomationProperties.GetAutomationId(at) is { Length: > 0 } id)
            {
                key = id;
            }
        }

        return null;
    }

    /// <summary>
    /// Draws <paramref name="content"/> as the card, carrying keyboard focus
    /// to the same control or its nearest neighbour (<see cref="FocusCarry"/>),
    /// so a redraw does not drop a keyboard user's place (W17 review F2).
    /// </summary>
    /// <param name="focusedKey">The key read before the action began, or null to read it now.</param>
    public static void Replace(UserControl host, UIElement content, string? focusedKey = null)
    {
        string? key = focusedKey ?? FocusedKey(host);
        host.Content = content;
        if (key is null || !host.IsLoaded || content is not FrameworkElement element)
        {
            return;
        }

        void OnLoaded(object sender, RoutedEventArgs e)
        {
            element.Loaded -= OnLoaded;
            var controls = new List<Control>();
            Collect(element, controls);
            var candidates = new List<FocusCandidate>(controls.Count);
            foreach (Control control in controls)
            {
                candidates.Add(new FocusCandidate(
                    AutomationProperties.GetAutomationId(control),
                    control.IsEnabled && control.IsTabStop && control.Visibility == Visibility.Visible));
            }

            if (FocusCarry.Target(key, candidates) is { } target)
            {
                foreach (Control control in controls)
                {
                    if (AutomationProperties.GetAutomationId(control) == target)
                    {
                        control.Focus(FocusState.Programmatic);
                        break;
                    }
                }
            }
        }

        element.Loaded += OnLoaded;
    }

    private static void Collect(DependencyObject parent, List<Control> into)
    {
        int count = VisualTreeHelper.GetChildrenCount(parent);
        for (int i = 0; i < count; i++)
        {
            DependencyObject child = VisualTreeHelper.GetChild(parent, i);
            if (child is Control control && AutomationProperties.GetAutomationId(control).Length > 0)
            {
                into.Add(control);
            }

            Collect(child, into);
        }
    }
}
