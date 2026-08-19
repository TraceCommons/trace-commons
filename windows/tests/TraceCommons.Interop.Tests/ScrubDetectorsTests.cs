using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The scrubber's detector names, read across the real ABI.
///
/// <c>tc_scrub_detector_names</c> now exists in the cdylib, so these drive the
/// actual export the way the rest of this suite drives the daemon, rather than
/// parsing a fixture. The dialog's list is the one thing on onboarding screen 1
/// that makes a factual claim about what the scrubber catches, and it is now
/// checked against the scrubber itself.
/// </summary>
public class ScrubDetectorsTests
{
    /// <summary>
    /// The export answers, and answers with something. An empty list here
    /// would mean the dialog silently claims nothing is removed.
    /// </summary>
    [Fact]
    public void TheRealExportReturnsTheScrubbersDetectors()
    {
        IReadOnlyList<string> names = ScrubDetectors.Names();

        Assert.NotEmpty(names);
        Assert.Contains("github_token", names);
        Assert.Contains("pem_header_orphan", names);
    }

    /// <summary>
    /// The guard, now over the LIVE table rather than a set copied out of the
    /// Rust when this screen was written. A detector added upstream appears in
    /// the dialog whether or not anyone taught this shell to say its name, so
    /// this fails the day that happens and the name it appears under stays a
    /// decision rather than a de-slugged accident.
    ///
    /// Compared against the DE-SLUGGED form, not against the slug: an earlier
    /// version asserted only that the label differed from the slug and carried
    /// no underscore, which the fallback satisfies, so deleting a label left
    /// the test passing.
    /// </summary>
    [Fact]
    public void EveryDetectorTheScrubberReportsHasAHumanLabel()
    {
        foreach (string slug in ScrubDetectors.Names())
        {
            Assert.NotEqual(slug.Replace('_', ' '), ScrubDetectorCopy.LabelFor(slug));
        }
    }

    /// <summary>
    /// Names only. The C# side must not become the place a pattern leaks, which
    /// is what the ABI test <c>the_detector_export_never_carries_a_pattern</c>
    /// asserts on the other side of the boundary. Checked per decoded name, not
    /// on the raw envelope, because JSON brings its own brackets.
    /// </summary>
    [Fact]
    public void NoDetectorNameCarriesAPattern()
    {
        foreach (string slug in ScrubDetectors.Names())
        {
            Assert.DoesNotContain('\\', slug);
            Assert.DoesNotContain('[', slug);
            Assert.DoesNotContain('(', slug);
            Assert.DoesNotContain('+', slug);
            Assert.DoesNotContain('*', slug);
        }
    }

    /// <summary>
    /// What the dialog actually renders: every detector, in the scrubber's
    /// order, as a display label.
    /// </summary>
    [Fact]
    public void LabelsCoverEveryDetectorInOrder()
    {
        IReadOnlyList<string> names = ScrubDetectors.Names();
        IReadOnlyList<string> labels = ScrubDetectors.Labels();

        Assert.Equal(names.Count, labels.Count);
        Assert.Equal(ScrubDetectorCopy.LabelFor(names[0]), labels[0]);
    }

    [Fact]
    public void AWellFormedPayloadKeepsItsOrder()
    {
        IReadOnlyList<string> names =
            ScrubDetectors.ParseNames("[\"github_token\",\"jwt\",\"npm_token\"]");

        Assert.Equal(new[] { "github_token", "jwt", "npm_token" }, names);
    }

    /// <summary>
    /// The dialog is reference material opened during onboarding. A transient
    /// fault should leave a contributor reading the concession over an empty
    /// list, not staring at a crash on the first screen.
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("not json")]
    [InlineData("{\"unexpected\":\"shape\"}")]
    public void AnUnusablePayloadIsEmptyRatherThanFatal(string? payload)
    {
        Assert.Empty(ScrubDetectors.ParseNames(payload));
    }

    /// <summary>
    /// A blank entry would render as an empty bullet, which reads as a detector
    /// nobody bothered to name.
    /// </summary>
    [Fact]
    public void BlankEntriesAreDropped()
    {
        IReadOnlyList<string> names = ScrubDetectors.ParseNames("[\"jwt\",\"\",\"  \"]");

        Assert.Equal(new[] { "jwt" }, names);
    }
}
