using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Which organization Private AI spends from, in the Private AI section of
/// Settings (spec W19, <c>SpendOrganizationCard</c>,
/// macos/Sources/TraceCommonsApp/Views/Settings/SpendOrganizationCard.swift).
/// It is where the Private AI tab's spending prompt leads.
/// </summary>
/// <remarks>
/// <para>
/// Drawn only for two or more organizations (<see cref="SpendOrganizationModel.ShowsPicker"/>):
/// one organization, or a listing that failed, collapses it. The picker shows
/// the organization in use, as the Mac's menu carries its check; filling it
/// sends nothing, and only a pick of a different organization switches. While
/// a switch or a sign-in is in flight the picker is held and the line under
/// it says so; a refusal is the daemon's sentence.
/// </para>
/// <para>
/// Code-built, with no XAML, so it slots into a Settings section by one
/// host: here beside the terminal picker, and after #2219 into
/// <c>BodyPrivateAI</c>, in the Mac's order (after the login change, before
/// funding). Every word is the daemon's <c>view</c>.
/// </para>
/// </remarks>
public sealed class SpendOrganizationCard : UserControl
{
    private readonly SpendOrganizationModel _model;
    private readonly TextBlock _title = new() { TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
    private readonly ComboBox _picker = new() { MinWidth = 200, VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock _what = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Grid _notice = new();
    private readonly Border _noticeWash = new() { Opacity = 0.14 };
    private readonly FontIcon _noticeIcon = new() { FontSize = 13, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 1, 0, 0) };
    private readonly TextBlock _noticeText = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, FontWeight = FontWeights.SemiBold };
    private bool _filling;
    private bool _bringIntoView;
    private bool _renderQueued;

    public SpendOrganizationCard(SpendOrganizationModel model)
    {
        _model = model;
        IsTabStop = false;

        // GlassType bodyStrong: 13 semibold.
        _title.FontSize = 13;
        _title.FontWeight = FontWeights.SemiBold;
        _what.Style = (Style)Application.Current.Resources["TcCaptionTextStyle"];
        _what.Foreground = Brush("TcInkSecondaryBrush");
        _noticeText.Foreground = Brush("TcInkPrimaryBrush");
        AutomationProperties.SetAccessibilityView(_noticeIcon, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        AutomationProperties.SetLiveSetting(_noticeText, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);

        var row = new Grid { ColumnSpacing = 16 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(_title);
        Grid.SetColumn(_picker, 1);
        row.Children.Add(_picker);

        // GlassNotice: the tone's icon in its colour, the title beside it,
        // on the tone's colour at 0.14, padded 10 all round.
        _noticeWash.CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"];
        var noticeRow = new Grid { ColumnSpacing = 8, Padding = new Thickness(10) };
        noticeRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        noticeRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        noticeRow.Children.Add(_noticeIcon);
        Grid.SetColumn(_noticeText, 1);
        noticeRow.Children.Add(_noticeText);
        _notice.Children.Add(_noticeWash);
        _notice.Children.Add(noticeRow);

        var stack = new StackPanel { Spacing = 10 };
        stack.Children.Add(row);
        stack.Children.Add(_what);
        stack.Children.Add(_notice);
        Content = new Border
        {
            Background = Brush("TcSurfaceCardBrush"),
            BorderBrush = Brush("TcHairlineBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
            Padding = new Thickness(14, 12, 14, 12),
            Child = stack,
        };

        _picker.SelectionChanged += async (_, _) =>
        {
            // The picker echoing the selection this card set is not a pick.
            if (_filling) return;
            await _model.PickAsync(_picker.SelectedIndex);
        };
        Loaded += OnLoaded;
        Unloaded += (_, _) => _model.PropertyChanged -= OnModelChanged;
        Render();
    }

    /// <summary>
    /// Scrolls this card into view and puts the keyboard on the picker, now
    /// or once it is on screen: what the spending prompt's action does.
    /// </summary>
    public void BringPickerIntoView()
    {
        _bringIntoView = true;
        if (IsLoaded) Settle();
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        _model.PropertyChanged -= OnModelChanged;
        _model.PropertyChanged += OnModelChanged;
        Render();
        if (_bringIntoView) Settle();
        // Read when the section appears (PrivateAISection.swift:93), never on
        // the sign-in; the read sends no choice.
        await _model.RefreshAsync();
    }

    private void Settle()
    {
        if (Visibility != Visibility.Visible) return;
        _bringIntoView = false;
        StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 });
        _picker.Focus(FocusState.Programmatic);
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs e)
    {
        // Queued, not inline: a pick raises this from inside the picker's own
        // SelectionChanged, and setting its selection there leaves the
        // closed picker drawing an empty box.
        if (_renderQueued) return;
        _renderQueued = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            _renderQueued = false;
            Render();
            if (_bringIntoView && IsLoaded) Settle();
        });
    }

    private void Render()
    {
        if (!_model.ShowsPicker)
        {
            Visibility = Visibility.Collapsed;
            return;
        }

        _filling = true;
        try
        {
            _title.Text = _model.PickerTitle;
            AutomationProperties.SetName(_picker, _model.PickerName);
            AutomationProperties.SetName(this, _model.PickerTitle);
            // Rows are rebuilt only when the daemon's list changed: clearing
            // a focused picker drops the keyboard and scrolls Settings away.
            IReadOnlyList<string> names = _model.OptionNames;
            if (!_picker.Items.Cast<object>().Select(o => o as string).SequenceEqual(names))
            {
                _picker.Items.Clear();
                foreach (string name in names) _picker.Items.Add(name);
            }

            _picker.SelectedIndex = _model.CurrentIndex;
            Hold(!_model.CanPick);
        }
        finally
        {
            _filling = false;
        }

        _what.Text = _model.What;
        _what.Visibility = _model.HasWhat ? Visibility.Visible : Visibility.Collapsed;

        if (_model.StateLine is { } line)
        {
            bool ask = line.Tone == SpendOrganizationTone.Ask;
            _noticeWash.Background = Brush(ask ? "TcStatusAskBrush" : "TcStatusOutsideBrush");
            _noticeIcon.Foreground = Brush(ask ? "TcStatusAskBrush" : "TcStatusOutsideBrush");
            // exclamationmark.circle.fill for ask, exclamationmark.triangle.fill for outside.
            _noticeIcon.Glyph = ask ? "" : "";
            _noticeText.Text = line.Text;
            _notice.Visibility = Visibility.Visible;
        }
        else
        {
            _notice.Visibility = Visibility.Collapsed;
        }

        Visibility = Visibility.Visible;
    }

    /// <summary>
    /// Holds the picker while a switch or sign-in is in flight
    /// (<c>.disabled(spendOrganizationSwitching || credentialBusy)</c>). A
    /// disabled control gives up the keyboard, and WinUI would hand it to
    /// the next stop and scroll Settings there, so the card keeps it for the
    /// hold and gives it back to the picker after.
    /// </summary>
    private void Hold(bool held)
    {
        if (held == !_picker.IsEnabled) return;
        if (held)
        {
            bool focused = _picker.FocusState != FocusState.Unfocused;
            IsTabStop = true;
            if (focused) Focus(FocusState.Programmatic);
            _picker.IsEnabled = false;
        }
        else
        {
            _picker.IsEnabled = true;
            bool focused = FocusState != FocusState.Unfocused;
            if (focused) _picker.Focus(FocusState.Programmatic);
            IsTabStop = false;
        }
    }

    private static Brush Brush(string key) => (Brush)Application.Current.Resources[key];
}
