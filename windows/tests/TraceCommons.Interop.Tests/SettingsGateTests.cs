using System.Collections.Generic;
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
}
