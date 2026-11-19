using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// Missions (W23): the Mac's <c>MissionsPage</c>
/// (Views/Monitor/MissionsViews.swift:25-71), a Home sub-page under the
/// shell's breadcrumb.
/// </summary>
/// <remarks>
/// <para>Under #1174's rules, as on the Mac. M1: one catalogue for the whole
/// commons; nothing claims a mission was matched for this person. M2: a
/// mission sends nothing by itself, and this page has no action at all; its
/// only way out is the breadcrumb, and Escape, back to Home. M3: credit is
/// projected, labelled with the core's word and note, beside the commons'
/// own statement of what it waits on, and a dash without that statement.</para>
/// <para>The catalogue is the Mac's PROVISIONAL shape, which only sample data
/// reaches (<see cref="DaemonHost.MissionCatalogue"/>). Not served, as on
/// every live launch, is a dash named with the core's word for unknown; a
/// failed read is the core's line: never a 0 that reads as "none".</para>
/// <para>Every word is the core's monitor table. Built in code, on the
/// skinned brushes; rebuilt on a theme change.</para>
/// </remarks>
public sealed class MissionsView : UserControl
{
    private readonly DaemonHost _host;
    private readonly Action _back;
    private readonly StackPanel _root = new() { Spacing = 14 };

    public MissionsView(DaemonHost host, Action back)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _back = back ?? throw new ArgumentNullException(nameof(back));
        Content = new ScrollViewer
        {
            Content = _root,
            Padding = new Thickness(22, 24, 22, 22),
            HorizontalScrollMode = ScrollMode.Disabled,
            IsTabStop = false,
        };
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, args) =>
        {
            args.Handled = true;
            _back();
        };
        KeyboardAccelerators.Add(escape);
        KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        AutomationProperties.SetAutomationId((UIElement)Content, "MissionsPage");
        ActualThemeChanged += (_, _) => Render();
        Render();
    }

    private static string Word(string key) => MonitorShellCopy.Top(key);

    /// <summary>Reads the catalogue again and redraws (each time the page is shown).</summary>
    public void Render()
    {
        _root.Children.Clear();
        (MissionRead read, string? json) = _host.MissionCatalogue();
        MissionPage page = MissionPage.Settle(read, json, Word);
        if (page.Catalogue is not { } catalogue)
        {
            if (page.FailureLine is { Length: > 0 } line)
            {
                // A failed read: the core's line in an outside-toned notice
                // (MissionsViews.swift:33-35). No figure at all, never 0.
                var notice = Card();
                notice.Child = Text(line, "TcBodyTextStyle", "TcStatusOutsideTextBrush");
                _root.Children.Add(notice);
            }
            else if (page.UnknownName is { } unknown)
            {
                // Not loaded, or a daemon with no catalogue yet: a dash,
                // never an empty list that reads as "none"
                // (MissionsViews.swift:58-64).
                TextBlock dash = Text(MissionFormat.Dash, "TcMetricValueTextStyle", "TcInkTertiaryBrush");
                AutomationProperties.SetName(dash, unknown);
                _root.Children.Add(dash);
            }

            return;
        }

        if (MissionFormat.Condition(catalogue) is { } condition)
        {
            _root.Children.Add(Text(condition, "TcCaptionTextStyle", "TcInkTertiaryBrush"));
        }

        if (MissionFormat.ShowsProjected(catalogue, Word))
        {
            _root.Children.Add(Text(Word("projected_note"), "TcCaptionTextStyle", "TcInkTertiaryBrush"));
        }

        if (catalogue.Missions.Count == 0)
        {
            TextBlock zero = Text("0", "TcMetricValueTextStyle", "TcInkTertiaryBrush");
            AutomationProperties.SetName(zero, $"{Word("missions")} 0");
            _root.Children.Add(zero);
            return;
        }

        foreach (Mission mission in catalogue.Missions)
        {
            _root.Children.Add(MissionCard(mission, MissionFormat.Credit(mission, catalogue, Word)));
        }
    }

    /// <summary>One mission: its title and summary, and its credit as projected (MissionsViews.swift:75-103).</summary>
    private static UIElement MissionCard(Mission mission, string credit)
    {
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var words = new StackPanel { Spacing = 2 };
        words.Children.Add(Text(mission.Title, "TcTitleCardTextStyle"));
        if (mission.Summary is { } summary)
        {
            words.Children.Add(Text(summary, "TcCaptionTextStyle", "TcInkSecondaryBrush"));
        }

        grid.Children.Add(words);

        var figure = new StackPanel { Spacing = 1, HorizontalAlignment = HorizontalAlignment.Right };
        TextBlock projected = Text(Word("projected"), "TcEyebrowTextStyle", "TcInkTertiaryBrush");
        projected.HorizontalAlignment = HorizontalAlignment.Right;
        figure.Children.Add(projected);
        TextBlock amount = Text(credit, "TcBodyTextStyle", "TcInkSecondaryBrush");
        amount.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        amount.HorizontalAlignment = HorizontalAlignment.Right;
        figure.Children.Add(amount);
        Grid.SetColumn(figure, 1);
        grid.Children.Add(figure);

        Border card = Card();
        card.Child = grid;
        return card;
    }

    private static T Res<T>(string key) => (T)Application.Current.Resources[key];

    private static TextBlock Text(string text, string style, string? brush = null)
    {
        var block = new TextBlock { Text = text, Style = Res<Style>(style), TextWrapping = TextWrapping.Wrap };
        if (brush is not null)
        {
            block.Foreground = Res<Brush>(brush);
        }

        return block;
    }

    private static Border Card() => new()
    {
        Background = Res<Brush>("TcSurfaceCardBrush"),
        BorderBrush = Res<Brush>("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = Res<CornerRadius>("TcCardCornerRadius"),
        Padding = new Thickness(16, 14, 16, 14),
    };
}
