using System;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// The card for a trace being contributed, kept or dismissed
/// (<c>TraceTransferCard</c>, TraceTransferCard.swift): its tool's coin,
/// title and tag; the trace's heading, folder and time; and behind Details
/// the tool's memory files, which stay on this computer, and the note.
/// The words and what shows come from <see cref="TransferCardFace"/>.
/// In the notice stack the stack's own X closes it; at the map's foot it
/// carries its own.
/// </summary>
internal sealed class TransferCardView : UserControl
{
    private readonly Border _frame = new();
    private readonly ToolMarkView _coin = new();
    private readonly TextBlock _title = new() { FontWeight = FontWeights.SemiBold, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock _tag = new() { VerticalAlignment = VerticalAlignment.Center };
    private readonly Border _tagPill = new() { Padding = new Thickness(8, 2, 8, 2), VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock _eyebrow = new();
    private readonly TextBlock _folder = new() { TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly TextBlock _time = new();
    private readonly TextBlock _files = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock _stays = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Border _memory = new() { CornerRadius = new CornerRadius(9), Padding = new Thickness(12, 6, 12, 6) };
    private readonly TextBlock _note = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Expander _details = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Button? _close;

    /// <summary>The card's X, at the map's foot: closes this card alone.</summary>
    public event Action? CloseRequested;

    public TransferCardView(Style? closeStyle)
    {
        IsTabStop = false;
        _frame.BorderThickness = new Thickness(1);
        _frame.CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"];
        _frame.Padding = new Thickness(14, 12, 14, 12);
        Content = _frame;

        _title.Style = Style("TcBodyTextStyle");
        _tag.Style = Style("TcCaptionTextStyle");
        _tagPill.CornerRadius = (CornerRadius)Application.Current.Resources["TcPillCornerRadius"];
        _tagPill.Child = _tag;
        _eyebrow.Style = Style("TcEyebrowTextStyle");
        _folder.Style = Style("TcCaptionTextStyle");
        _time.Style = Style("TcCaptionTextStyle");
        _files.Style = Style("TcMonoCodeTextStyle");
        _stays.Style = Style("TcCaptionTextStyle");
        _note.Style = Style("TcCaptionTextStyle");

        var head = new Grid { ColumnSpacing = 10 };
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        head.Children.Add(_coin);
        Grid.SetColumn(_title, 1);
        head.Children.Add(_title);
        Grid.SetColumn(_tagPill, 2);
        head.Children.Add(_tagPill);
        if (closeStyle is not null)
        {
            _close = new Button
            {
                Content = new FontIcon { Glyph = "\uE711", FontSize = 10 },
                Style = closeStyle,
                VerticalAlignment = VerticalAlignment.Center,
            };
            AutomationProperties.SetName(_close, MonitorWords.Close);
            ToolTipService.SetToolTip(_close, MonitorWords.Close);
            _close.Click += (_, _) => CloseRequested?.Invoke();
            Grid.SetColumn(_close, 3);
            head.Children.Add(_close);
        }
        else
        {
            // The stack's X sits over this corner.
            head.Margin = new Thickness(0, 0, 28, 0);
        }

        // The folder gives way, never the time (:44-58).
        var summary = new Grid { ColumnSpacing = 8 };
        summary.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        summary.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        summary.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        _eyebrow.VerticalAlignment = _folder.VerticalAlignment = _time.VerticalAlignment = VerticalAlignment.Bottom;
        summary.Children.Add(_eyebrow);
        Grid.SetColumn(_folder, 1);
        summary.Children.Add(_folder);
        Grid.SetColumn(_time, 2);
        summary.Children.Add(_time);

        var lockIcon = new FontIcon { Glyph = "\uE72E", FontSize = 10, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 3, 0, 0) };
        AutomationProperties.SetAccessibilityView(lockIcon, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        var memoryRow = new Grid { ColumnSpacing = 8 };
        memoryRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        memoryRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        memoryRow.Children.Add(lockIcon);
        var lines = new StackPanel { Spacing = 2, Children = { _files, _stays } };
        Grid.SetColumn(lines, 1);
        memoryRow.Children.Add(lines);
        _memory.Child = memoryRow;

        _details.Header = MonitorWords.NoticeDetails;
        _details.Content = new StackPanel { Spacing = 8, Children = { _memory, _note } };

        _frame.Child = new StackPanel { Spacing = 8, Children = { head, summary, _details } };
        ApplyBrushes();
        ActualThemeChanged += (_, _) => ApplyBrushes();
    }

    private bool _tagOn;

    /// <summary>The theme's brushes, again on a theme change (code cannot hold a ThemeResource).</summary>
    private void ApplyBrushes()
    {
        _frame.Background = Brush("TcSurfaceCardBrush");
        _frame.BorderBrush = Brush("TcHairlineBrush");
        _eyebrow.Foreground = Brush("TcInkTertiaryBrush");
        _time.Foreground = Brush("TcInkSecondaryBrush");
        _stays.Foreground = Brush("TcInkSecondaryBrush");
        _memory.Background = Brush("TcSurfaceInsetBrush");
        _tagPill.Background = Brush("TcSurfaceInsetBrush");
        _tag.Foreground = _tagOn ? Brush("TcStatusOnTextBrush") : Brush("TcInkSecondaryBrush");
    }

    /// <summary>The transfer this card shows now.</summary>
    public string? TransferId { get; private set; }

    /// <summary>Shows <paramref name="face"/>, the card for <paramref name="transferId"/>.</summary>
    public void Show(string transferId, TransferCardFace face)
    {
        bool same = TransferId == transferId;
        TransferId = transferId;
        _coin.Art = face.Mark;
        _title.Text = face.Title;
        _tag.Text = face.Tag;
        _tagOn = face.TagOn;
        _tag.Foreground = _tagOn ? Brush("TcStatusOnTextBrush") : Brush("TcInkSecondaryBrush");
        _eyebrow.Text = face.Eyebrow;
        _folder.Text = face.Folder ?? string.Empty;
        _folder.Visibility = face.Folder is null ? Visibility.Collapsed : Visibility.Visible;
        _time.Text = face.Time is { } time ? "· " + time : string.Empty;
        _time.Visibility = face.Time is null ? Visibility.Collapsed : Visibility.Visible;
        _files.Text = face.MemoryFiles ?? string.Empty;
        _files.Visibility = face.MemoryFiles is null ? Visibility.Collapsed : Visibility.Visible;
        _stays.Text = face.MemoryStays ?? string.Empty;
        _memory.Visibility = face.MemoryStays is null ? Visibility.Collapsed : Visibility.Visible;
        _note.Text = face.Note;
        if (!same)
        {
            _details.IsExpanded = false;
        }

        AutomationProperties.SetName(this, face.AccessibleName);
    }

    private static Brush Brush(string key) => (Brush)Application.Current.Resources[key];

    private static Style Style(string key) => (Style)Application.Current.Resources[key];
}
