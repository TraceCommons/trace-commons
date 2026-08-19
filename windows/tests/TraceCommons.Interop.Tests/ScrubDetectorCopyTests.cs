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
