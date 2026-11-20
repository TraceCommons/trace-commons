using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;
using TraceCommons.Interop.Generated;
using TraceCommons.Interop.Shell;
using TraceCommons.App.Look;

namespace TraceCommons.App;

/// <summary>
/// The tray flyout (W13; the Mac's menu-bar panel,
/// Views/Monitor/MenuBarGlassPanel.swift): opened by a click on the tray
/// icon, on the monitor that holds the click, against its taskbar. It
/// re-houses what the tray already says (the header line, the week, the
/// Private AI line) and the ways into the app; the right-click menu stays
/// as it was. A topmost tool window out of Alt-Tab and the taskbar; it takes
/// the keyboard because it was opened by a click, closes when it loses it,
/// and gives the keyboard back to the window that had it.
/// </summary>
internal sealed class TrayFlyoutWindow : Window
{
    /// <summary>The panel's width in DIPs: the open rail's peek width (GlassTokens <c>edgeRailPeekWidth</c>).</summary>
    public const double WidthDips = EdgeRailGeometry.PeekWidth;

    private readonly IntPtr _hwnd;
    private readonly StackPanel _body = new() { Spacing = HomeTrayInks.Measures.CardGap, Padding = new Thickness(HomeTrayInks.Measures.PanePadding) };
    private IntPtr _previousFocus;
    private bool _shown;

    public TrayFlyoutWindow(Action openApp, Action openSettings)
    {
        _hwnd = ToolWindows.Prepare(this);
        Title = MonitorWords.AppName;
        var root = new Border
        {
            Child = new ScrollViewer { Content = _body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled },
        };

        // Built once; the body is built again on every open.
        ThemeBrushes.Bind(root, Border.BackgroundProperty, "TcWindowBackgroundBrush");
        Content = root;
        OpenApp = openApp;
        OpenSettings = openSettings;
        Activated += (_, e) =>
        {
            if (e.WindowActivationState == WindowActivationState.Deactivated && _shown)
            {
                Dismiss();
            }
        };
        root.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Escape)
            {
                Dismiss();
                e.Handled = true;
            }
        };
    }

    private Action OpenApp { get; }

    private Action OpenSettings { get; }

    /// <summary>Fills the panel from the tray's own model, then opens it at the click.</summary>
    public void ShowAt((int X, int Y) click, string header, string week, string? privateAI, string? markTooltip)
    {
        _body.Children.Clear();
        var title = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S4 };
        title.Children.Add(new Microsoft.UI.Xaml.Shapes.Path
        {
            Data = NearMarkGeometry.Build(),
            Stretch = Stretch.Uniform,
            Fill = ThemeBrushes.Get("TcInkPrimaryBrush"),
            Width = 18,
            Height = 18,
            VerticalAlignment = VerticalAlignment.Center,
        });
        title.Children.Add(new TextBlock { Text = MonitorWords.AppName, Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"] });
        _body.Children.Add(title);
        Line(header, "TcBodyTextStyle");
        if (markTooltip is { Length: > 0 })
        {
            Line(markTooltip, "TcCaptionTextStyle");
        }

        Line(week, "TcCaptionTextStyle");
        if (privateAI is { Length: > 0 })
        {
            Line(privateAI, "TcCaptionTextStyle");
        }

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = HomeTrayInks.Measures.S4 };
        actions.Children.Add(ActionButton(MonitorShellCopy.Section("edge_rail", "open_app"), OpenApp));
        actions.Children.Add(ActionButton(MonitorWords.SettingsTitle, OpenSettings));
        _body.Children.Add(actions);

        (double scale, (int X, int Y, int Width, int Height) bounds, (int X, int Y, int Width, int Height) work) = ToolWindows.MonitorAt(click.X, click.Y);
        _body.Measure(new Windows.Foundation.Size(WidthDips, double.PositiveInfinity));
        int width = (int)Math.Round(WidthDips * scale);
        int height = (int)Math.Round(Math.Min(EdgeRailGeometry.OpenHeight, _body.DesiredSize.Height + 2) * scale);
        (int x, int y) = TrayFlyoutPlacement.Place(click, bounds, work, (width, height), (int)Math.Round(HomeTrayInks.Measures.S4 * scale));

        _previousFocus = ToolWindows.Foreground();
        ToolWindows.PlaceAndShow(this, (x, y, width, height));
        ToolWindows.TakeKeyboard(_hwnd);
        _shown = true;

        // The first action takes focus: a ring only when the panel was
        // opened from the keyboard (FocusState.Programmatic follows the last
        // input), never a nameless stop on the window itself.
        ToolWindows.FocusFirst(_body, FocusState.Programmatic, () => _shown);
    }

    private void Line(string text, string style)
    {
        if (text.Length == 0)
        {
            return;
        }

        _body.Children.Add(new TextBlock
        {
            Text = text,
            Style = (Style)Application.Current.Resources[style],
            TextWrapping = TextWrapping.WrapWholeWords,
        });
    }

    private Button ActionButton(string label, Action action)
    {
        var button = new Button { Content = label, Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"] };
        AutomationProperties.SetName(button, label);
        button.Click += (_, _) =>
        {
            Dismiss();
            action();
        };
        return button;
    }

    /// <summary>Closes the panel and gives the keyboard back to the window that had it, if it is still there.</summary>
    public void Dismiss()
    {
        if (!_shown)
        {
            return;
        }

        _shown = false;
        ToolWindows.ReleaseKeyboard(_hwnd);
        AppWindow.Hide();
        ToolWindows.ReturnFocus(_previousFocus);
        _previousFocus = IntPtr.Zero;
    }
}
