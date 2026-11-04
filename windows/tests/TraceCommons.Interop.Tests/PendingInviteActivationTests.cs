using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class PendingInviteActivationTests
{
    [Fact]
    public void StartupAndRootsWaitRetainOnlyLatestUntilBothAreReady()
    {
        var pending = new PendingInviteActivation();
        pending.Receive("old");
        Assert.Null(pending.Take(false, false, false, false).Invite);
        pending.Receive("latest");
        pending.Receive(null);
        var roots = pending.Take(true, true, false, false);
        Assert.Null(roots.Invite);
        Assert.NotNull(roots.Notice);
        Assert.Equal("latest", pending.Take(true, false, true, false).Invite);
        Assert.Null(pending.Take(true, false, true, false).Invite);
    }

    [Fact]
    public void FailedStartupRefusesVisiblyAndClearsCredential()
    {
        var pending = new PendingInviteActivation();
        pending.Receive("invite");
        var refused = pending.Take(true, false, false, false);
        Assert.Null(refused.Invite);
        Assert.Contains("cannot reach the running app", refused.Notice);
        Assert.DoesNotContain("daemon", refused.Notice, System.StringComparison.OrdinalIgnoreCase);
        Assert.Null(pending.Take(true, false, true, false).Invite);
    }

    [Fact]
    public void ExistingFlowWaitsThenOffersLatestAndCloseClearsIt()
    {
        var pending = new PendingInviteActivation();
        pending.Receive("old");
        Assert.NotNull(pending.Take(true, false, true, true).Notice);
        pending.Receive("latest");
        Assert.Equal("latest", pending.Take(true, false, true, false).Invite);
        pending.Receive("discard");
        pending.Clear();
        Assert.Null(pending.Take(true, false, true, false).Invite);
    }
}
