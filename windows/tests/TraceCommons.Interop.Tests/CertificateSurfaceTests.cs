using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
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

    /// <summary>
    /// The queue asks the shared surface and writes no sentence of its own.
    /// </summary>
    /// <remarks>
    /// TraceCommons.App is WinUI and cannot be referenced from a test
    /// assembly, so the view model and the markup are read as the text the
    /// csproj copies beside us. Comment lines are stripped first: a guard
    /// that fails source for naming a sentence in prose teaches the next
    /// reader to delete the prose, and these comments are what say why the
    /// section is drawn when empty.
    /// </remarks>
    [Fact]
    public void TheQueueAsksTheSharedSurfaceAndAuthorsNoSentence()
    {
        string source = Uncommented("MainViewModel.cs.txt");

        Assert.Contains("CertificateSurface.ListTitle(_evidenceAdmitted)", source, StringComparison.Ordinal);
        Assert.Contains("CertificateSurface.RowLine(_evidenceAdmitted)", source, StringComparison.Ordinal);

        // The flag reaches the surface unnegated. It is true for a
        // contributor with NO invite, so a negation here would tell them
        // their session carries cryptographic proof when nothing has
        // attested it.
        Assert.Contains("_evidenceAdmitted = settings.AdmissionEvidenceRequired == true;", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ListTitle(!_evidenceAdmitted)", source, StringComparison.Ordinal);
        Assert.DoesNotContain("RowLine(!_evidenceAdmitted)", source, StringComparison.Ordinal);

        // Membership is the row's own answer and takes no reading.
        Assert.Contains("row.HoldsCertificate", source, StringComparison.Ordinal);

        foreach (string authored in new[] { "put forward", "signed proof", "witness certificate" })
        {
            Assert.DoesNotContain(authored, source, StringComparison.OrdinalIgnoreCase);
        }
    }

    /// <summary>
    /// The section is drawn even with nothing in it.
    /// </summary>
    /// <remarks>
    /// An empty filtered section that renders as nothing is indistinguishable
    /// from one that failed to load, and a missing <c>holds_certificate</c>
    /// defaults to false on every row, producing exactly that.
    /// </remarks>
    [Fact]
    public void TheEmptySectionStillSaysSomething()
    {
        string markup = Regex.Replace(
            Regex.Replace(Uncommented("MainWindow.xaml.txt"), "<!--.*?-->", " ", RegexOptions.Singleline),
            @"\s+",
            " ");

        Assert.Contains("ViewModel.CertificateHeldTitle", markup, StringComparison.Ordinal);
        Assert.Contains("ViewModel.CertificateHeldEmptyText", markup, StringComparison.Ordinal);
        Assert.Contains("ViewModel.CertificateHeldIsEmpty", markup, StringComparison.Ordinal);
        Assert.Contains("ViewModel.CertificateHeld,", markup, StringComparison.Ordinal);
    }

    private static string Uncommented(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, file);
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        return string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
    }
}
