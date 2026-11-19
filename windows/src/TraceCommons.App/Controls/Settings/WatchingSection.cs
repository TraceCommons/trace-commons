using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Watching, the watcher card (<c>WatchingSection</c>,
/// WatchingSection.swift): once the status has answered, the core's
/// Watcher title with its Watching or Paused chip, what pausing does, the
/// queue limit, then Pause and Resume (<see cref="WatcherCard"/>). Before
/// the status answers, only the queue limit, in a plain card. The queue
/// limit waits for the daemon's settings, with the read notice meanwhile.
/// Every word is the core's.
/// </summary>
public sealed class WatchingSection : UserControl
{
    private readonly DaemonHost _host;
    private readonly WatcherFlow _flow;

    public WatchingSection(DaemonHost host)
    {
        IsTabStop = false;
        _host = host;
        _flow = new WatcherFlow((method, json) => host.CallAsync(method, json));
        Loaded += OnLoaded;
        Unloaded += (_, _) => host.StatusChanged -= OnStatusChanged;
    }

    /// <summary>Raised after every status read, with the paused state the header chip shows.</summary>
    public event Action<bool>? PausedChanged;

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        _host.StatusChanged -= OnStatusChanged;
        _host.StatusChanged += OnStatusChanged;
        Render();
        await _flow.ReadSettingsAsync();
        await _flow.ReadStatusAsync();
        Render();
    }

    private async void OnStatusChanged()
    {
        if (!IsLoaded)
        {
            return;
        }

        await _flow.ReadStatusAsync();
        Render();
    }

    private UIElement QueueLimit()
    {
        if (_flow.QueueTtlDays is { } days)
        {
            TextBlock line = SettingsCards.Caption(SettingsLegacyWords.UndecidedDropped(days));
            line.Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcInkTertiaryBrush"];
            return line;
        }

        return SettingsCards.ReadNotice(_flow.SettingsReadFailed, async (_, _) =>
        {
            await _flow.ReadSettingsAsync();
            Render();
        });
    }

    private void Render()
    {
        WatcherCard card = _flow.Card(running: _host.IsRunning);
        if (_flow.StatusAnswered)
        {
            PausedChanged?.Invoke(_flow.Paused);
        }

        if (!card.Drawn)
        {
            Content = SettingsCards.Card(string.Empty, QueueLimit());
            return;
        }

        var body = new StackPanel { Spacing = 12 };
        TextBlock caption = SettingsCards.Caption(MonitorShellCopy.Shell("watcher_caption"));
        caption.Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcInkTertiaryBrush"];
        body.Children.Add(caption);
        body.Children.Add(QueueLimit());

        Button pause = SettingsCards.Button(MonitorShellCopy.PauseWatcher, async (_, _) =>
        {
            await _flow.PauseAsync();
            Render();
        });
        pause.IsEnabled = card.PauseEnabled && !_flow.Busy;
        Button resume = SettingsCards.Button(MonitorShellCopy.ResumeWatcher, async (_, _) =>
        {
            await _flow.ResumeAsync();
            Render();
        }, primary: true);
        resume.IsEnabled = card.ResumeEnabled && !_flow.Busy;
        body.Children.Add(SettingsCards.Row(pause, resume));

        Content = SettingsCards.TitledCard(
            MonitorShellCopy.Shell("watcher_eyebrow"),
            MonitorShellCopy.Shell("watcher_title"),
            card.Chip.Length > 0 ? SettingsCards.GlassChip(card.Chip, card.ChipMuted) : null,
            body);
    }
}
