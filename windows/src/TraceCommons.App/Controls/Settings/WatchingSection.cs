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

    /// <summary>
    /// What the card was last drawn from. A status event that changes none
    /// of it draws nothing, so the controls, and a keyboard user's focus on
    /// them, stay (W17 review F2).
    /// </summary>
    private object? _drawn;

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
            return SettingsCards.TertiaryCaption(SettingsLegacyWords.UndecidedDropped(days));
        }

        return SettingsCards.ReadNotice(_flow.SettingsReadFailed, async (_, _) =>
        {
            string? at = SettingsCards.FocusedKey(this);
            await _flow.ReadSettingsAsync();
            Render(at);
        }, "watcher/retry");
    }

    private void Render(string? focus = null)
    {
        WatcherCard card = _flow.Card(running: _host.IsRunning);
        if (_flow.StatusAnswered)
        {
            PausedChanged?.Invoke(_flow.Paused);
        }

        object drawn = (card, _flow.QueueTtlDays, _flow.SettingsReadFailed, _flow.Busy);
        if (Content is not null && Equals(drawn, _drawn))
        {
            return;
        }

        _drawn = drawn;
        if (!card.Drawn)
        {
            SettingsCards.Replace(this, SettingsCards.Card(string.Empty, QueueLimit()), focus);
            return;
        }

        var body = new StackPanel { Spacing = 12 };
        TextBlock caption = SettingsCards.TertiaryCaption(MonitorShellCopy.Shell("watcher_caption"));
        body.Children.Add(caption);
        body.Children.Add(QueueLimit());

        Button pause = SettingsCards.Key(SettingsCards.Button(MonitorShellCopy.PauseWatcher, async (_, _) =>
        {
            string? at = SettingsCards.FocusedKey(this);
            await _flow.PauseAsync();
            Render(at);
        }), "watcher/pause");
        pause.IsEnabled = card.PauseEnabled && !_flow.Busy;
        Button resume = SettingsCards.Key(SettingsCards.Button(MonitorShellCopy.ResumeWatcher, async (_, _) =>
        {
            string? at = SettingsCards.FocusedKey(this);
            await _flow.ResumeAsync();
            Render(at);
        }, primary: true), "watcher/resume");
        resume.IsEnabled = card.ResumeEnabled && !_flow.Busy;
        body.Children.Add(SettingsCards.Row(pause, resume));

        SettingsCards.Replace(this, SettingsCards.TitledCard(
            MonitorShellCopy.Shell("watcher_eyebrow"),
            MonitorShellCopy.Shell("watcher_title"),
            card.Chip.Length > 0 ? SettingsCards.GlassChip(card.Chip, card.ChipMuted) : null,
            body), focus);
    }
}
