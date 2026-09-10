using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A contribution the commons refused must not read as one that never left.
/// </summary>
/// <remarks>
/// This shell had no outcome table: it showed the raw label with separators
/// swapped, which was machine-shaped but not misleading. macOS said "Held"
/// and Linux said "Nothing was sent.", which on this path is false. See #810.
///
/// <para>
/// On the spelling: these five labels are the server's own, with underscores,
/// while <c>daemon::health</c>'s constants for the same events are hyphenated.
/// Production never spells them -- it passes through whatever the daemon sent
/// -- so a wrong spelling could only hide in this file, and it hides in the
/// safe direction: a label the ABI does not recognise answers null and fails
/// these assertions rather than passing them.
/// </para>
/// </remarks>
public sealed class OutcomeRefusalSurfaceTests
{
    private static readonly string[] RefusalLabels =
    {
        "admission_refused",
        "admission_limit_reached",
        "admission_in_progress",
        "admission_identity_conflict",
        "admission_evidence_refused",
    };

    [Fact]
    public void ARefusedContributionSaysSoInWordsAndNotAsARawLabel()
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (string label in RefusalLabels)
        {
            string? line = NearAiEnrollSurfaceProbe(label);
            Assert.False(
                string.IsNullOrEmpty(line),
                $"{label} answered nothing, so the row would still show a raw label");
            Assert.DoesNotContain("nothing was sent", line!, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("_", line!);
            Assert.True(seen.Add(line!), $"{label} shares a sentence with another refusal");
        }
    }

    /// <summary>A lease another attempt holds is not a refusal.</summary>
    [Fact]
    public void ALeaseAnotherAttemptHoldsDoesNotReadAsARefusal()
    {
        string line = NearAiEnrollSurfaceProbe("admission_in_progress")!;
        foreach (string refused in new[] { "declined", "refused", "turned away", "rejected" })
        {
            Assert.DoesNotContain(refused, line, StringComparison.OrdinalIgnoreCase);
        }
    }

    /// <summary>
    /// Everything else is left alone, so the lookup is additive rather than a
    /// takeover of a surface that has not been moved into the crate.
    /// </summary>
    [Fact]
    public void ALabelThatIsNotARefusalIsNotClaimed()
    {
        foreach (string other in new[] { "dismissed-by-contributor", "queue-full", "", "nonsense" })
        {
            Assert.Null(NearAiEnrollSurfaceProbe(other));
        }
        Assert.Null(NearAiEnrollSurfaceProbe(null));
    }

    private static string? NearAiEnrollSurfaceProbe(string? label) =>
        OutcomeRefusalSurface.Line(label);

    /// <summary>
    /// The outcome row actually consults the shared table.
    /// </summary>
    /// <remarks>
    /// Without this, every assertion above passes while the row still renders
    /// a raw label: they exercise the interop surface, and the view model is
    /// what decides whether that surface is ever asked. TraceCommons.App is
    /// WinUI and cannot be referenced from a test assembly, so the view model
    /// is read as the text the csproj copies beside us. Comment lines are
    /// stripped first.
    /// </remarks>
    [Fact]
    public void TheOutcomeRowAsksTheSharedTableBeforeShowingARawLabel()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "HistoryViewModel.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        string source = string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        Assert.Contains("OutcomeRefusalSurface.Line(label)", source, StringComparison.Ordinal);
        // Still additive: the raw-label rendering remains for everything else.
        Assert.Contains("label.Replace('-', ' ').Replace('_', ' ')", source, StringComparison.Ordinal);
    }
}
