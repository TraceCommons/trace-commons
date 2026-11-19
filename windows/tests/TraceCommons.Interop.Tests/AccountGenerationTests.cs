using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The account boundary consent writes are settled against (checklist L2;
/// the Mac's <c>accountReadGeneration</c>, AppModel.swift:4698).
/// </summary>
public class AccountGenerationTests
{
    [Fact]
    public void TheFirstReadSettlesTheOwnerWithoutABoundary()
    {
        var owner = new AccountGeneration();
        long pressed = owner.Current;
        Assert.False(owner.Observe("sha256:a"));
        Assert.True(owner.IsCurrent(pressed));
    }

    [Fact]
    public void ASameAccountReadKeepsAPressCurrent()
    {
        var owner = new AccountGeneration();
        owner.Observe("sha256:a");
        long pressed = owner.Current;
        Assert.False(owner.Observe("sha256:a"));
        Assert.True(owner.IsCurrent(pressed));
    }

    [Fact]
    public void AReadNamingAnotherAccountRefusesAPressMadeBeforeIt()
    {
        var owner = new AccountGeneration();
        owner.Observe("sha256:a");
        long pressed = owner.Current;
        Assert.True(owner.Observe("sha256:b"));
        Assert.False(owner.IsCurrent(pressed));
    }

    [Fact]
    public void SigningOutIsABoundaryBeforeAnyReadSaysSo()
    {
        var owner = new AccountGeneration();
        owner.Observe("sha256:a");
        long pressed = owner.Current;
        owner.Boundary();
        Assert.False(owner.IsCurrent(pressed));
    }

    [Fact]
    public void AnAccountLosingItsScopeIsABoundary()
    {
        var owner = new AccountGeneration();
        owner.Observe("sha256:a");
        long pressed = owner.Current;
        Assert.True(owner.Observe(null));
        Assert.False(owner.IsCurrent(pressed));
    }

    [Fact]
    public void AFailedReadBetweenTwoAccountsStillCountsTheChange()
    {
        var owner = new AccountGeneration();
        owner.Observe("sha256:a");
        long pressed = owner.Current;
        // A failed read observes nothing; the next answered read is compared
        // with the last account a read named.
        Assert.True(owner.Observe("sha256:b"));
        Assert.False(owner.IsCurrent(pressed));
    }

    [Fact]
    public void AnUnenrollTellsEveryWindowBeforeAnyReadSaysSo()
    {
        var owner = new AccountGeneration();
        int told = 0;
        owner.BoundaryCrossed += () => told++;
        owner.Observe("sha256:a");
        owner.Boundary();
        Assert.Equal(1, told);
    }
}
