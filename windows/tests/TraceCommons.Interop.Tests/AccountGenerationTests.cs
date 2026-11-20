using System;
using System.IO;
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

    /// <summary>
    /// Every new generation is announced, from a status naming another
    /// account as from a sign-out, so a Settings section showing the old
    /// account's devices, payouts or referrals reads them again (#2219
    /// review, defect 7). The same account read again announces nothing.
    /// </summary>
    [Fact]
    public void EveryNewAccountIsAnnouncedAndTheSameOneIsNot()
    {
        var owner = new AccountGeneration();
        int told = 0;
        owner.Changed += () => told++;
        owner.Observe("sha256:a");
        owner.Observe("sha256:a");
        Assert.Equal(0, told);
        owner.Observe("sha256:b");
        Assert.Equal(1, told);
        owner.Boundary();
        Assert.Equal(2, told);
    }

    /// <summary>Settings reads the account's lists again on each new generation.</summary>
    [Fact]
    public void SettingsReadsTheAccountsListsAgainOnANewAccount()
    {
        string sections = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "Controls", "SettingsView.Sections.cs.txt"));
        Assert.Contains("host.Account.Changed += OnAccountChanged;", sections);
        Assert.Contains("_devices?.RefreshAsync()", sections);
        Assert.Contains("_payouts?.RefreshAsync()", sections);
        Assert.Contains("_referrals?.RefreshAsync()", sections);
    }
}
