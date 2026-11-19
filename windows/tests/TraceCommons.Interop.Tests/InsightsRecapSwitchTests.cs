using System.Collections.Generic;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > Insights' weekly-summary switch against the Mac's
/// <c>InsightsSettingsCard</c> (InsightsSettingsCard.swift:30-45) and
/// <c>AppModel.setInsightsSwitch</c> (AppModel.swift:3159-3184), and its
/// words against the core (service.rs:59, analytics_copy.rs:838).
/// </summary>
public class InsightsRecapSwitchTests
{
    private sealed class Daemon
    {
        public readonly List<(string Method, string Params)> Sent = new();
        public string Get = """{"result":{"insights_recap_card_enabled":true}}""";
        public string Set = """{"result":{"insights_recap_card_enabled":false}}""";

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Sent.Add((method, parameters));
            return Task.FromResult(DaemonResponse.Parse(method == "set_settings" ? Set : Get));
        }
    }

    private static InsightsRecapSwitch Switch(Daemon daemon) =>
        new(daemon.Call) { RefusalLine = () => "failed" };

    [Fact]
    public async Task NoSwitchIsDrawnUntilTheDaemonReportsOne()
    {
        var daemon = new Daemon { Get = """{"result":{"queue_ttl_days":14}}""" };
        InsightsRecapSwitch s = Switch(daemon);
        Assert.False(s.SettingsAnswered);
        await s.ReadAsync();
        Assert.True(s.SettingsAnswered);
        Assert.Null(s.On);
    }

    [Fact]
    public async Task AConfirmedWriteStandsAndSendsOneKey()
    {
        var daemon = new Daemon();
        InsightsRecapSwitch s = Switch(daemon);
        await s.ReadAsync();
        Assert.True(s.On);
        daemon.Sent.Clear();
        await s.SetAsync(false);
        Assert.Equal(("set_settings", """{"insights_recap_card_enabled":false}"""), daemon.Sent[0]);
        Assert.False(s.On);
        Assert.Null(s.Refusal);
    }

    [Fact]
    public async Task AReplyWithTheOldValueKeepsItAndSaysTheRequestFailed()
    {
        var daemon = new Daemon { Set = """{"result":{"insights_recap_card_enabled":true}}""" };
        InsightsRecapSwitch s = Switch(daemon);
        await s.ReadAsync();
        await s.SetAsync(false);
        Assert.True(s.On);
        Assert.Equal("failed", s.Refusal);
    }

    [Fact]
    public async Task ARefusedCallReadsTheSettingsAgainAndALaterSuccessClearsTheLine()
    {
        var daemon = new Daemon { Set = """{"error":{"code":"refused","message":"no"}}""" };
        InsightsRecapSwitch s = Switch(daemon);
        await s.ReadAsync();
        daemon.Sent.Clear();
        await s.SetAsync(false);
        Assert.Equal(new[] { "set_settings", "get_settings" }, daemon.Sent.ConvertAll(x => x.Method));
        Assert.True(s.On);
        Assert.Equal("failed", s.Refusal);

        daemon.Set = """{"result":{"insights_recap_card_enabled":false}}""";
        await s.SetAsync(false);
        Assert.Null(s.Refusal);
        Assert.False(s.On);
    }

    [Fact]
    public void TheWordsAreTheCores()
    {
        Assert.Equal("Insights", InsightsSettingsWords.Text("title"));
        Assert.Equal("Weekly summary", InsightsSettingsWords.Text("analytics_setting_recap_card"));
    }
}
