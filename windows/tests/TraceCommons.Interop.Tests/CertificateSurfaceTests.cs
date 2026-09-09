using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The certificate readings, taken from the live ABI rather than a fixture.
/// </summary>
public sealed class CertificateSurfaceTests
{
    [Fact]
    public void EachReadingIsItsOwnSentence()
    {
        string? uninvited = CertificateSurface.RowLine(evidenceAdmitted: true);
        string? invited = CertificateSurface.RowLine(evidenceAdmitted: false);
        Assert.False(string.IsNullOrEmpty(uninvited));
        Assert.False(string.IsNullOrEmpty(invited));
        Assert.NotEqual(uninvited, invited);
        Assert.NotEqual(
            CertificateSurface.ListTitle(evidenceAdmitted: true),
            CertificateSurface.ListTitle(evidenceAdmitted: false));
    }

    /// <summary>
    /// The direction, as an outcome rather than a mapping.
    /// </summary>
    /// <remarks>
    /// <c>admission_evidence_required</c> is TRUE for a contributor who has NO
    /// invite. If this shell ever passed the flag negated, that contributor
    /// would be told their session carries cryptographic proof when nothing
    /// has attested it. An assertion that merely repeated the mapping would be
    /// equally happy with the negation in place.
    /// </remarks>
    [Fact]
    public void AContributorWithoutAnInviteIsNeverToldItIsAttested()
    {
        string uninvited = CertificateSurface.RowLine(evidenceAdmitted: true)!;
        Assert.DoesNotContain("signed proof", uninvited, System.StringComparison.OrdinalIgnoreCase);
        Assert.Contains("put forward", uninvited, System.StringComparison.OrdinalIgnoreCase);

        string invited = CertificateSurface.RowLine(evidenceAdmitted: false)!;
        Assert.Contains("signed proof", invited, System.StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// The wire key must actually decode.
    /// </summary>
    /// <remarks>
    /// System.Text.Json defaults a missing bool, so a key renamed on one side
    /// only yields false on every row and an empty section -- which is what a
    /// contributor with no certificates sees. Nothing else here would fail.
    /// </remarks>
    [Fact]
    public void ARowCarryingTheKeyDiffersFromOneWithoutIt()
    {
        var with = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"a\",\"holds_certificate\":true}")!;
        var without = JsonSerializer.Deserialize<QueueEntry>("{\"entry_id\":\"a\"}")!;
        Assert.True(with.HoldsCertificate, "the key decoded into nothing");
        Assert.False(without.HoldsCertificate);
    }
}
