using System;
using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The digest could once say only one thing: how many sessions are waiting
/// for review. That was complete while every upload passed through review,
/// and stopped being complete when a project could be armed to contribute
/// without asking -- an armed project queues nothing, so the waiting count is
/// permanently zero and the contributor who most wanted to stop supervising
/// was told nothing at all.
///
/// These pin the contribution half. The daemon composes the same sentence in
/// <c>daemon::notify::contribution_text</c>, the Linux shell in
/// <c>notify::contribution_body</c>, and macOS in
/// <c>DigestCopy.contributionLine</c>; all four follow the same rules.
/// </summary>
public class DigestContributionTests
{
    [Fact]
    public void NothingContributedProducesNoLine()
    {
        Assert.Null(DigestText.ContributionLine(0, Array.Empty<string>(), 0));
    }

    [Fact]
    public void SingularReadsAsOneSession()
    {
        Assert.Equal(
            "1 session contributed from api.",
            DigestText.ContributionLine(1, new[] { "api" }, 0));
    }

    [Fact]
    public void PluralNamesTheProjects()
    {
        Assert.Equal(
            "4 sessions contributed from api and web.",
            DigestText.ContributionLine(4, new[] { "api", "web" }, 0));
    }

    /// <summary>
    /// A notification may be persisted in the Windows notification centre.
    /// Labels only, never a path -- the same rule the waiting half follows.
    /// </summary>
    [Fact]
    public void NeverContainsAPath()
    {
        string? line = DigestText.ContributionLine(2, new[] { "api", "web" }, 3.5);
        Assert.NotNull(line);
        Assert.DoesNotContain("/", line, StringComparison.Ordinal);
    }

    [Fact]
    public void BlankLabelsAreDroppedRatherThanTrailingAFrom()
    {
        Assert.Equal(
            "2 sessions contributed.",
            DigestText.ContributionLine(2, new[] { "", "  " }, 0));
    }

    /// <summary>
    /// Credit is the other half of the value exchange and the reason this
    /// line exists -- but only when there is some. "0 credit pending" reads
    /// as a failure rather than as a fresh start.
    /// </summary>
    [Fact]
    public void CreditIsStatedOnlyWhenThereIsSome()
    {
        Assert.Equal(
            "2 sessions contributed from api. 4.2 credit pending.",
            DigestText.ContributionLine(2, new[] { "api" }, 4.25));
        Assert.DoesNotContain(
            "credit",
            DigestText.ContributionLine(2, new[] { "api" }, 0)!,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// Settlement is off on every deployment shipped so far, so a bare figure
    /// would be read as money that exists. The word is always "pending".
    /// </summary>
    [Theory]
    [InlineData("earned")]
    [InlineData("paid")]
    [InlineData("settled")]
    [InlineData("worth")]
    public void PendingCreditIsNeverCalledEarned(string forbidden)
    {
        string? line = DigestText.ContributionLine(2, new[] { "api" }, 4.25);
        Assert.NotNull(line);
        Assert.Contains("pending", line, StringComparison.Ordinal);
        Assert.DoesNotContain(forbidden, line, StringComparison.Ordinal);
    }

    /// <summary>
    /// The hole this closes: an armed contributor has nothing pending, ever.
    /// </summary>
    [Fact]
    public void ContributionsAloneClaimADigest()
    {
        var cadence = new DigestCadence();
        Assert.True(cadence.TryClaim(0, 7, DateTimeOffset.UtcNow));
    }

    [Fact]
    public void NothingAtAllStillClaimsNothing()
    {
        var cadence = new DigestCadence();
        Assert.False(cadence.TryClaim(0, 0, DateTimeOffset.UtcNow));
        Assert.Null(cadence.LastClaimedAt);
    }

    /// <summary>
    /// Contributions do not get their own faster clock. One interruption per
    /// period, whatever the period held.
    /// </summary>
    [Fact]
    public void ContributionsDoNotShortenTheInterval()
    {
        var start = DateTimeOffset.UtcNow;
        var cadence = new DigestCadence();
        Assert.True(cadence.TryClaim(0, 3, start));
        Assert.False(cadence.TryClaim(0, 99, start.AddHours(1)));
    }

    /// <summary>
    /// A daemon predating these fields sends neither. Zero and an empty list
    /// degrade the digest to the waiting-only one that shipped before, rather
    /// than to a wrong number.
    /// </summary>
    [Fact]
    public void AnOlderDaemonsFrameDecodesToZero()
    {
        DaemonEvent? evt = DaemonEvent.Parse(
            "{\"event\":\"digest_due\",\"data\":{\"pending\":3,\"text\":\"x\"}}");
        Assert.NotNull(evt);
        Assert.Equal(3, evt!.PendingCount);
        Assert.Equal(0, evt.ContributedCount);
        Assert.Empty(evt.ContributedProjects);
        Assert.Equal(0, evt.CreditPending);
    }

    [Fact]
    public void AFullFrameDecodesEveryField()
    {
        DaemonEvent? evt = DaemonEvent.Parse(
            "{\"event\":\"digest_due\",\"data\":{\"pending\":0,\"contributed\":5,"
            + "\"contributed_projects\":[\"api\",\"web\"],\"credit_pending\":2.5,\"text\":\"x\"}}");
        Assert.NotNull(evt);
        Assert.Equal(0, evt!.PendingCount);
        Assert.Equal(5, evt.ContributedCount);
        Assert.Equal(new[] { "api", "web" }, evt.ContributedProjects);
        Assert.Equal(2.5, evt.CreditPending);
    }
}
