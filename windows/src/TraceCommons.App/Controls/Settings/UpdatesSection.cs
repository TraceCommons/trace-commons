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
/// <see cref="UpdatesCard"/> says for this copy. Check Now asks the window
/// to check, which raises the update banner when the feed offers one.
/// </summary>
public sealed class UpdatesSection : UserControl
{
    public UpdatesSection(Action checkNow)
    {
        IsTabStop = false;
        bool packaged = PackageIdentity.IsPackaged();
        UpdatesCard card = UpdatesCard.Live(packaged, CurrentVersion(packaged));

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

        if (card.Caption is { } caption)
        {
            body.Children.Add(SettingsCards.Caption(caption));
        }

        if (card.CheckNow is { } word)
        {
            Button check = SettingsCards.Button(word, (_, _) => checkNow());
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
