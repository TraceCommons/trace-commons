using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The removed-list dialog's words and slug prettification.
/// </summary>
public class ScrubDetectorCopyTests
{
    /// <summary>
    /// The words themselves, against the copy the other two shells read.
    ///
    /// <c>EveryDetectorTheScrubberReportsHasAHumanLabel</c> proves a label
    /// EXISTS for every detector the native scrubber reports. It cannot see
    /// what the label says, and each shell hardcodes its own nine strings, so
    /// all three could satisfy their coverage guards while telling
    /// contributors three different things about the same detector.
    ///
    /// Iterates the fixture rather than this shell's table, so a detector
    /// this shell forgot fails here too, not only upstream.
    /// </summary>
    [Fact]
    public void ScrubDetectorLabelsMatchTheSharedFixture()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "scrub-detector-labels.json");
        Assert.True(
            File.Exists(path),
            $"the shared scrub-label fixture was not copied to {path}; without it this test " +
            "would pass over nothing");

        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(path));
        JsonElement labels = doc.RootElement.GetProperty("labels");

        int checked_ = 0;
        foreach (JsonProperty entry in labels.EnumerateObject())
        {
            Assert.Equal(entry.Value.GetString(), ScrubDetectorCopy.LabelFor(entry.Name));
            checked_++;
        }

        Assert.True(
            checked_ >= 9,
            $"the shared fixture lists only {checked_} detectors; a short list silently " +
            "weakens this test");
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
    /// Cursor keys are named in their own words, not folded into the
    /// provider-token line.
    ///
    /// <c>EveryDetectorTheScrubberReportsHasAHumanLabel</c> already fails if
    /// this detector reaches the dialog unlabelled, so what this adds is the
    /// wording: the whole reason <c>cursor_api_key</c> is a separate detector
    /// rather than a fifth arm of <c>provider_token</c> is that a Cursor user
    /// has to be able to find their own key in this list. A label reading
    /// "cursor api key", or one quietly merged into the Stripe/GitLab/Slack
    /// line, satisfies the coverage guard and loses the point. macOS pins the
    /// same two strings in <c>ScrubDetectorsTests</c>; this is the Windows
    /// half of that pair.
    /// </summary>
    [Fact]
    public void CursorKeysAreNamedInTheirOwnWords()
    {
        Assert.Equal("Cursor API keys", ScrubDetectorCopy.LabelFor("cursor_api_key"));
        Assert.Equal(
            "Stripe, GitLab and Slack tokens",
            ScrubDetectorCopy.LabelFor("provider_token"));
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
