using System.Collections.Generic;
using System.Threading.Tasks;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > Watching's watcher card and the Settings header chip against
/// the Mac: <c>WatchingSection</c> (WatchingSection.swift:10, 45-46, 57,
/// 60, 70-72) and <c>SettingsModal.chip</c> (SettingsModal.swift:245-248),
/// with the words from the core's tables
/// (preview_copy.rs:1579-1580, 1702-1704, 1729-1733; shell_words_copy.rs:498).
/// </summary>
public class WatcherCardTests
{
    [Theory]
    // statusAnswered, lastReadFailed, running -> armed (WatchingSection.swift:71).
    [InlineData(true, false, true, true)]
    [InlineData(false, false, true, false)]
    [InlineData(true, true, true, false)]
    [InlineData(true, false, false, false)]
    public void ControlsAreArmedOnlyWhileTheShownStateIsCurrent(bool answered, bool failed, bool running, bool armed)
    {
        Assert.Equal(armed, WatcherCard.ControlsArmed(answered, failed, running));
    }

    [Fact]
    public void BeforeTheStatusAnswersNoChipOrButtonsAreDrawn()
    {
        WatcherCard card = WatcherCard.Of(false, false, false, true, "Watching", "Paused");
        Assert.False(card.Drawn);
        Assert.False(card.PauseEnabled);
        Assert.False(card.ResumeEnabled);
    }

    [Fact]
    public void WatchingOffersPauseAndPausedOffersResume()
    {
        WatcherCard watching = WatcherCard.Of(true, false, false, true, "Watching", "Paused");
        Assert.Equal(("Watching", false, true, false), (watching.Chip, watching.ChipMuted, watching.PauseEnabled, watching.ResumeEnabled));

        WatcherCard paused = WatcherCard.Of(true, true, false, true, "Watching", "Paused");
        Assert.Equal(("Paused", true, false, true), (paused.Chip, paused.ChipMuted, paused.PauseEnabled, paused.ResumeEnabled));
    }

    [Fact]
    public void AStaleAnswerStillDrawsTheChipButNeitherButtonActs()
    {
        WatcherCard stale = WatcherCard.Of(true, true, true, true, "Watching", "Paused");
        Assert.True(stale.Drawn);
        Assert.Equal("Paused", stale.Chip);
        Assert.False(stale.PauseEnabled);
        Assert.False(stale.ResumeEnabled);
    }

    [Fact]
    public void TheHeaderChipIsWatchingOnOrPausedOff()
    {
        Assert.Equal(new SettingsHeaderChip("Watching", true), SettingsHeaderChip.For(false, "Watching", "Paused"));
        Assert.Equal(new SettingsHeaderChip("Paused", false), SettingsHeaderChip.For(true, "Watching", "Paused"));
        Assert.Null(SettingsHeaderChip.For(false, string.Empty, "Paused"));
    }

    [Fact]
    public void TheWordsAreTheCores()
    {
        Assert.Equal(new SettingsHeaderChip("Watching", true), SettingsHeaderChip.Live(false));
        Assert.Equal(new SettingsHeaderChip("Paused", false), SettingsHeaderChip.Live(true));
        Assert.Equal("Watcher", MonitorShellCopy.Shell("watcher_title"));
        Assert.Equal("Watching", MonitorShellCopy.Shell("watcher_watching"));
        Assert.Equal("Paused", MonitorShellCopy.Shell("watcher_paused"));
        Assert.Equal("Pausing stops contribution. Queued traces and consent are unchanged.", MonitorShellCopy.Shell("watcher_caption"));
        Assert.Equal("Pause", MonitorShellCopy.PauseWatcher);
        Assert.Equal("Resume", MonitorShellCopy.ResumeWatcher);
        Assert.Equal("Undecided traces are dropped after 30 days and never sent.", SettingsLegacyWords.UndecidedDropped(30));
    }

    private sealed class Daemon
    {
        public readonly List<(string Method, string Params)> Sent = new();
        public bool Paused;
        public bool FailStatus;

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Sent.Add((method, parameters));
            string reply = method switch
            {
                "status" when FailStatus => """{"error":{"code":"unavailable","message":"down"}}""",
                "status" => "{\"result\":{\"paused\":" + (Paused ? "true" : "false") + "}}",
                "get_settings" => """{"result":{"queue_ttl_days":14}}""",
                "pause" => Reply(() => Paused = true),
                "resume" => Reply(() => Paused = false),
                _ => """{"error":{"code":"unknown","message":"unknown"}}""",
            };
            return Task.FromResult(DaemonResponse.Parse(reply));
        }

        private static string Reply(System.Action act)
        {
            act();
            return """{"result":{}}""";
        }
    }

    [Fact]
    public async Task PauseSendsAnOpenEndedPauseThenReadsTheStatus()
    {
        var daemon = new Daemon();
        var flow = new WatcherFlow(daemon.Call);
        await flow.ReadSettingsAsync();
        await flow.ReadStatusAsync();
        Assert.Equal(14, flow.QueueTtlDays);
        Assert.True(flow.Card(running: true).PauseEnabled);

        daemon.Sent.Clear();
        await flow.PauseAsync();
        Assert.Equal(("pause", "{}"), daemon.Sent[0]);
        Assert.Equal("status", daemon.Sent[1].Method);
        WatcherCard card = flow.Card(running: true);
        Assert.True(card.ResumeEnabled);
        Assert.False(card.PauseEnabled);

        daemon.Sent.Clear();
        await flow.ResumeAsync();
        Assert.Equal(("resume", "{}"), daemon.Sent[0]);
        Assert.True(flow.Card(running: true).PauseEnabled);
    }

    [Fact]
    public async Task AFailedReadAfterAnAnswerKeepsTheChipAndDisarmsBoth()
    {
        var daemon = new Daemon { Paused = true };
        var flow = new WatcherFlow(daemon.Call);
        await flow.ReadStatusAsync();
        daemon.FailStatus = true;
        await flow.ReadStatusAsync();
        WatcherCard card = flow.Card(running: true);
        Assert.True(card.Drawn);
        Assert.Equal("Paused", card.Chip);
        Assert.False(card.ResumeEnabled);
    }
}
