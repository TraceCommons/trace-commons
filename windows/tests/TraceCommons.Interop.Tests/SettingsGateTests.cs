using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings' onboarding gate against the Mac's <c>MonitorGate</c>
/// (MonitorNavigation.swift:300-312) and <c>availableBeforeOnboarding</c>
/// (SettingsSections.swift:52-60): Payouts and Referrals, and every other
/// section first run asks about, are not drawn before onboarding.
/// </summary>
public class SettingsGateTests
{
    [Theory]
    [InlineData(false, false, SettingsGate.Awaiting)]
    [InlineData(false, true, SettingsGate.Awaiting)]
    [InlineData(true, true, SettingsGate.SignedOut)]
    [InlineData(true, false, SettingsGate.Open)]
    public void TheWindowWaitsThenGatesOnOnboarding(bool known, bool requires, SettingsGate want)
    {
        Assert.Equal(want, SettingsGates.Of(known, requires));
    }

    [Theory]
    [InlineData(SettingsSection.Payouts, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Referrals, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Consent, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Witness, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Account, SettingsGate.Open)]
    [InlineData(SettingsSection.General, SettingsGate.Open)]
    [InlineData(SettingsSection.Compute, SettingsGate.Open)]
    public void OnlyTheSectionsFirstRunDoesNotAskWaitForIt(SettingsSection section, SettingsGate want)
    {
        Assert.Equal(want, SettingsGates.For(section, SettingsGate.SignedOut));
        Assert.Equal(SettingsGate.Open, SettingsGates.For(section, SettingsGate.Open));
    }

    /// <summary>
    /// Two status changes overlap: the older read answers "set up" after the
    /// newer one answered "needs onboarding". The newer answer stands
    /// (#2219 review, defect 1).
    /// </summary>
    [Fact]
    public async Task AnOlderReplyLandingLateDoesNotReopenTheGate()
    {
        var replies = new Queue<TaskCompletionSource<bool?>>();
        var reader = new SettingsGateReader(() =>
        {
            var reply = new TaskCompletionSource<bool?>();
            replies.Enqueue(reply);
            return reply.Task;
        });

        Task<bool> older = reader.ReadAsync();
        Task<bool> newer = reader.ReadAsync();
        TaskCompletionSource<bool?> olderReply = replies.Dequeue();
        TaskCompletionSource<bool?> newerReply = replies.Dequeue();

        newerReply.SetResult(true);
        Assert.True(await newer);
        Assert.Equal(SettingsGate.SignedOut, reader.Gate);

        olderReply.SetResult(false);
        Assert.False(await older);
        Assert.Equal(SettingsGate.SignedOut, reader.Gate);
    }

    /// <summary>
    /// The Mac counts a failed status read as known, over the placeholder
    /// status that requires onboarding (MonitorNavigation.swift:275-283),
    /// so a first failed read closes the gated sections; once a status has
    /// answered, a failed read keeps it.
    /// </summary>
    [Fact]
    public async Task AFirstFailedReadClosesTheGateALaterOneKeepsTheAnswer()
    {
        var answers = new Queue<bool?>(new bool?[] { null, false, null });
        var reader = new SettingsGateReader(() => Task.FromResult(answers.Dequeue()));

        Assert.Equal(SettingsGate.Awaiting, reader.Gate);
        Assert.True(await reader.ReadAsync());
        Assert.Equal(SettingsGate.SignedOut, reader.Gate);
        Assert.True(await reader.ReadAsync());
        Assert.Equal(SettingsGate.Open, reader.Gate);
        Assert.False(await reader.ReadAsync());
        Assert.Equal(SettingsGate.Open, reader.Gate);
    }

    [Fact]
    public async Task AReadThatThrowsCountsAsFailed()
    {
        var reader = new SettingsGateReader(() => throw new System.InvalidOperationException("down"));
        Assert.True(await reader.ReadAsync());
        Assert.Equal(SettingsGate.SignedOut, reader.Gate);
    }

    /// <summary>
    /// A daemon that refused to start is down for every section first run
    /// asks about, onboarded or not, and that wins over whatever the status
    /// reads said; a section available before onboarding stays open
    /// (<c>MonitorGate.of</c>, MonitorNavigation.swift:300-304; #2219
    /// review, defect 2). A failed status read over a running daemon is
    /// still signed out, as on the Mac.
    /// </summary>
    [Theory]
    [InlineData(true, SettingsGate.Awaiting, SettingsGate.Down)]
    [InlineData(true, SettingsGate.SignedOut, SettingsGate.Down)]
    [InlineData(true, SettingsGate.Open, SettingsGate.Down)]
    [InlineData(false, SettingsGate.SignedOut, SettingsGate.SignedOut)]
    [InlineData(false, SettingsGate.Open, SettingsGate.Open)]
    [InlineData(false, SettingsGate.Awaiting, SettingsGate.Awaiting)]
    public void ARefusedStartIsDownNotSignedOut(bool refused, SettingsGate read, SettingsGate want)
    {
        SettingsGate window = SettingsGates.Window(refused, read);
        Assert.Equal(want, window);
        Assert.Equal(want, SettingsGates.For(SettingsSection.Consent, window));
        Assert.Equal(SettingsGate.Open, SettingsGates.For(SettingsSection.General, window));
    }

    /// <summary>The down notice's title is the core's (health_copy.rs core_down_copy).</summary>
    [Fact]
    public void TheDownNoticeIsTheCoresTitle()
    {
        Assert.Equal("Background service isn't running.", HealthCopy.CoreDownTitle());
    }

    /// <summary>
    /// The window applies the refusal over the reads, reads the gate again
    /// when a start lands or is refused, and draws the core's down title.
    /// </summary>
    [Fact]
    public void TheWindowDrawsARefusedStartAsDown()
    {
        string sections = ShellSource("TraceCommons.App/Controls/SettingsView.Sections.cs");
        Assert.Contains("SettingsGates.Window(host.StartRefused, _gateReader.Gate)", sections);
        Assert.Contains("host.Started += OnSectionsHostStarted;", sections);
        Assert.Contains("host.Refused += OnGateStatusChanged;", sections);
        Assert.Contains("if (gate == SettingsGate.Down)", sections);
        Assert.Contains("HealthCopy.CoreDownTitle()", sections);

        string host = ShellSource("TraceCommons.App/DaemonHost.cs");
        Assert.Contains("catch (TcException exception) when (!exception.IsRootsNotDeclared)", host);
        Assert.Contains("StartRefused = true;", host);
        Assert.Contains("StartRefused = false;", host);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
