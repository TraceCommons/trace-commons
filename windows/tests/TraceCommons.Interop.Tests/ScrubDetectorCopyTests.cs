using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The removed-list dialog's words and slug prettification.
/// </summary>
public class ScrubDetectorCopyTests
{
    /// <summary>
    /// The guard. A detector arriving without a label is a real defect: the
    /// dialog is what a contributor reads to decide whether to trust the
    /// scrubber, and a raw slug in it reads as a leak of internals.
    ///
    /// Weaker than GTK's equivalent, which iterates the scrubber's live table.
    /// This iterates the set read out of that table when the screen was built,
    /// because the export carrying the live one is not reachable from here yet.
    /// </summary>
    [Fact]
    public void EveryKnownDetectorHasAHumanLabel()
    {
        foreach (string slug in ScrubDetectorCopy.KnownDetectorsAtTimeOfWriting)
        {
            string label = ScrubDetectorCopy.LabelFor(slug);

            // Compared against the DE-SLUGGED form, not against the slug. An
            // earlier version of this test asserted only that the label
            // differed from the slug and contained no underscore -- which the
            // fallback satisfies, so deleting a label left the test passing.
            // The name a detector appears under must be a decision, not a
            // de-slugged accident.
            Assert.NotEqual(slug.Replace('_', ' '), label);
        }
    }

    /// <summary>
    /// An unrecognised detector still appears, de-slugged. Vanishing would be
    /// worse than looking unpolished: it would understate what the scrubber
    /// catches, on the one screen making a claim about coverage.
    /// </summary>
    [Fact]
    public void AnUnknownDetectorIsDesluggedRatherThanDropped()
    {
        Assert.Equal("azure storage key", ScrubDetectorCopy.LabelFor("azure_storage_key"));
    }

    /// <summary>
    /// The list renders in the order the scrubber reports, and blank entries
    /// are skipped rather than becoming empty rows.
    /// </summary>
    [Fact]
    public void LabelsPreserveOrderAndSkipBlanks()
    {
        IReadOnlyList<string> labels = ScrubDetectorCopy.LabelsFor(
            new[] { "github_token", "", "jwt" });

        Assert.Equal(new[] { "GitHub tokens", "JSON Web Tokens" }, labels);
    }

    /// <summary>
    /// The concession must travel with the list. A list of what is caught,
    /// shown alone, reads as a guarantee.
    /// </summary>
    [Fact]
    public void TheConcessionConcedesImperfection()
    {
        Assert.Contains("misses", ScrubDetectorCopy.ResidualRisk, StringComparison.Ordinal);
    }
}
