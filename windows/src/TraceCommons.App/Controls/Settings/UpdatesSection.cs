using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Updates (W17-D3 (a); <c>UpdatesSection</c>,
/// StartupSection.swift:180-227): the version, then what
/// <see cref="UpdatesCard"/> says for this copy: the tag, the last-check
/// line, the caption, then Check Now. Check Now asks the window to check,
/// which raises the update banner when the feed offers one; the window's
/// <see cref="UpdateCheckClock"/> then moves the last-check line.
/// </summary>
public sealed class UpdatesSection : UserControl
{
    private readonly Action _checkNow;
    private readonly UpdateCheckClock _checks;
    private readonly bool _packaged;
    private readonly string _version;

    public UpdatesSection(Action checkNow, UpdateCheckClock checks)
    {
        IsTabStop = false;
        _checkNow = checkNow;
        _checks = checks;
        _packaged = PackageIdentity.IsPackaged();
        _version = CurrentVersion(_packaged);
        _checks.Changed += Draw;
        Unloaded += (_, _) => _checks.Changed -= Draw;
        Loaded += (_, _) =>
        {
            _checks.Changed -= Draw;
            _checks.Changed += Draw;
            Draw();
        };
        Draw();
    }

    /// <summary>
    /// Draws the card again, so the last-check line's relative time is read
    /// against the clock now (the window is kept and shown again).
    /// </summary>
    public void Draw()
    {
        UpdatesCard card = UpdatesCard.Live(_packaged, _version, _checks.LastCompleted);

        var body = new StackPanel { Spacing = 12 };
        var versionRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        TextBlock label = SettingsCards.Text(card.VersionLabel, "TcCaptionTextStyle");
        label.Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcInkSecondaryBrush"];
        versionRow.Children.Add(label);
        TextBlock version = SettingsCards.Text(card.Version, "TcMonoCodeTextStyle");
        version.IsTextSelectionEnabled = true;
        AutomationProperties.SetName(version, card.Version);
        versionRow.Children.Add(version);
        body.Children.Add(versionRow);

        if (card.Tag is { } tag)
        {
            body.Children.Add(new ContentControl { IsTabStop = false, Content = SettingsCards.GlassChip(tag, muted: !card.TagOn) });
        }

        if (card.LastChecked is { } lastChecked)
        {
            body.Children.Add(SettingsCards.Caption(lastChecked));
        }

        if (card.Caption is { } caption)
        {
            body.Children.Add(SettingsCards.Caption(caption));
        }

        if (card.CheckNow is { } word)
        {
            Button check = SettingsCards.Button(word, (_, _) => _checkNow());
            body.Children.Add(SettingsCards.Row(check));
        }

        Content = SettingsCards.Card(MonitorShellCopy.Shell("updates_eyebrow"), body);
    }

    /// <summary>
    /// The installed package's version, or the app assembly's for an
    /// unpackaged build (<c>UpdateController.currentVersion</c>).
    /// </summary>
    private static string CurrentVersion(bool packaged)
    {
        if (packaged)
        {
            try
            {
                Windows.ApplicationModel.PackageVersion v = Windows.ApplicationModel.Package.Current.Id.Version;
                return $"{v.Major}.{v.Minor}.{v.Build}";
            }
            catch (InvalidOperationException)
            {
            }
        }

        Version? assembly = typeof(UpdatesSection).Assembly.GetName().Version;
        return assembly is null ? string.Empty : $"{assembly.Major}.{assembly.Minor}.{assembly.Build}";
    }
}
