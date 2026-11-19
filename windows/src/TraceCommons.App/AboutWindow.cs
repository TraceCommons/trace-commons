using System.Reflection;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop.Shell;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;

namespace TraceCommons.App;

/// <summary>
/// The app's About window (spec W24, AboutWindowView.swift:10-39): the mark at
/// 96 by 96, the app's name, the version line with a Copy button that copies
/// exactly that line, and the copyright when there is one. Opened from the
/// tray menu and from Settings (W24-D1 (a)); the main window keeps it by
/// reference and never finds it by title.
/// </summary>
public sealed class AboutWindow : Window
{
    /// <summary>AboutWindowView.swift:38: <c>.frame(minWidth: 280)</c>.</summary>
    public const double MinWidth = 280;

    private bool _placed;

    public AboutWindow()
    {
        Title = AboutLines.Title;
        Assembly assembly = typeof(AboutWindow).Assembly;
        // Packaged, the package's version is the release's (make-msix.ps1
        // -Version); unpackaged, the assembly's, which is the contributor
        // crate's unless the build passed -p:Version.
        (string version, string? build) = PackageIdentity.IsPackaged()
            ? PackagedVersion()
            : (AboutLines.ShortVersionFrom(
                assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion), null);
        string? copyright = assembly.GetCustomAttribute<AssemblyCopyrightAttribute>()?.Copyright;
        var model = new AboutModel(version, build, copyright, CopyToClipboard);

        // The icon. Decorative, as on the Mac (accessibilityHidden): no name,
        // no tab stop. The Mac shows the app icon; the N is that icon's mark
        // (G-3 (a)) until W10's tiles land.
        var mark = new Path
        {
            Data = NearMarkGeometry.Build(),
            Width = 96,
            Height = 96,
            Stretch = Stretch.Uniform,
            Fill = (Brush)Application.Current.Resources["TcMarkAccentBrush"],
            IsHitTestVisible = false,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        AutomationProperties.SetAccessibilityView(mark, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);

        var name = new TextBlock
        {
            Text = AboutLines.Name,
            Style = (Style)Application.Current.Resources["TcTitleScreenTextStyle"],
            HorizontalAlignment = HorizontalAlignment.Center,
        };

        var versionLine = new TextBlock
        {
            Text = model.Version,
            Style = (Style)Application.Current.Resources["TcBodyTextStyle"],
            IsTextSelectionEnabled = true,
            VerticalAlignment = VerticalAlignment.Center,
        };
        var copy = new Button
        {
            Content = AboutLines.CopyWord,
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
        };
        AutomationProperties.SetName(copy, AboutLines.CopyWord);
        copy.Click += (_, _) => model.CopyVersion();
        var versionRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6, // GlassTokens.Space.s3
            HorizontalAlignment = HorizontalAlignment.Center,
            Children = { versionLine, copy },
        };

        var column = new StackPanel
        {
            Spacing = 6, // GlassTokens.Space.s3 (AboutWindowView.swift:12)
            // .padding(.horizontal, s8) .padding(.top, s6) .padding(.bottom, s8)
            Padding = new Thickness(16, 12, 16, 16),
            MinWidth = MinWidth,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Children = { mark, name, versionRow },
        };
        if (model.ShowsCopyright)
        {
            column.Children.Add(new TextBlock
            {
                Text = model.Copyright,
                Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
                Foreground = (Brush)Application.Current.Resources["TcInkSecondaryBrush"],
                HorizontalAlignment = HorizontalAlignment.Center,
            });
        }

        Content = new Grid
        {
            Background = (Brush)Application.Current.Resources["TcWindowBackgroundBrush"],
            Children = { column },
        };
    }

    private static (string, string?) PackagedVersion()
    {
        Windows.ApplicationModel.PackageVersion v = Windows.ApplicationModel.Package.Current.Id.Version;
        return AboutLines.FromPackageVersion(v.Major, v.Minor, v.Build, v.Revision);
    }

    private static void CopyToClipboard(string line)
    {
        var package = new DataPackage();
        package.SetText(line);
        Clipboard.SetContent(package);
    }

    /// <summary>Shows the window on the monitor <paramref name="owner"/> is on, sized to its content the first time.</summary>
    public void ShowNearOwner(AppWindow owner)
    {
        if (!_placed)
        {
            _placed = true;
            DisplayArea display = DisplayArea.GetFromWindowId(owner.Id, DisplayAreaFallback.Nearest);
            RectInt32 work = display.WorkArea;
            double scale = MonitorScale.For(owner);
            // The Mac's window fits its content: 96 icon, title, version row and
            // caption over 6 pt gaps and 12/16 pt padding, at least 280 wide.
            int width = System.Math.Min(work.Width, MonitorPaneLayout.ToPhysical(320, scale));
            int height = System.Math.Min(work.Height, MonitorPaneLayout.ToPhysical(220, scale));
            AppWindow.ResizeClient(new SizeInt32(width, height));
            if (AppWindow.Presenter is OverlappedPresenter presenter)
            {
                presenter.IsMaximizable = false;
                presenter.IsMinimizable = false;
                presenter.IsResizable = false;
            }

            SizeInt32 outer = AppWindow.Size;
            AppWindow.Move(new PointInt32(
                work.X + System.Math.Max(0, (work.Width - outer.Width) / 2),
                work.Y + System.Math.Max(0, (work.Height - outer.Height) / 2)));
        }

        AppWindow.Show();
        Activate();
    }
}
