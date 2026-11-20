using System;
using System.Linq;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first run's Terms step draws the Mac's reader (TermsReader.swift:54-58,
/// :93-122, :215-240): a 420-wide column; a 260-high region holding the
/// document with its title, or the loading or failed state with Retry; then,
/// with the document loaded, the not-in-effect line, the checkbox and, while
/// it is locked, the scroll hint; then the Terms and Privacy links. Before,
/// Windows wrapped a 600-wide card round a title, the document and an
/// always-shown hint, with no links and no Retry, and drew the loading and
/// failed states as one line, so the step changed height as it loaded.
/// </summary>
public class TermsReaderLayoutTests
{
    [Fact]
    public void TheReadersMeasuresAreTheMacs()
    {
        Assert.Equal(260, TermsReaderLayout.Viewport);
        Assert.Equal(420, TermsReaderLayout.Width);
        // GlassTokens.Space.s4, s2, s8 and s6.
        Assert.Equal(8, TermsReaderLayout.Spacing);
        Assert.Equal(4, TermsReaderLayout.HintSpacing);
        Assert.Equal(16, TermsReaderLayout.DocumentPadding);
        Assert.Equal(12, TermsReaderLayout.BlockSpacing);
    }

    [Fact]
    public void LoadingAndFailedKeepTheRegionAndTheLinks()
    {
        Assert.Equal(new[] { TermsReaderPart.Region, TermsReaderPart.Links }, TermsReaderLayout.Parts(TermsRegion.Loading, inEffect: true, unlocked: false));
        Assert.Equal(new[] { TermsReaderPart.Region, TermsReaderPart.Links }, TermsReaderLayout.Parts(TermsRegion.Failed, inEffect: false, unlocked: false));
    }

    [Fact]
    public void ALoadedDocumentAddsTheNoteTheCheckboxAndTheHintWhileLocked()
    {
        Assert.Equal(
            new[] { TermsReaderPart.Region, TermsReaderPart.NotInEffect, TermsReaderPart.Accept, TermsReaderPart.ScrollHint, TermsReaderPart.Links },
            TermsReaderLayout.Parts(TermsRegion.Loaded, inEffect: false, unlocked: false));
        Assert.Equal(
            new[] { TermsReaderPart.Region, TermsReaderPart.Accept, TermsReaderPart.Links },
            TermsReaderLayout.Parts(TermsRegion.Loaded, inEffect: true, unlocked: true));
    }

    [Fact]
    public void TheRegionFollowsTheFetch()
    {
        Assert.Equal(TermsRegion.Loading, TermsReaderLayout.Region(loaded: false, failureLabel: null));
        Assert.Equal(TermsRegion.Failed, TermsReaderLayout.Region(loaded: false, failureLabel: "terms-unavailable"));
        Assert.Equal(TermsRegion.Loaded, TermsReaderLayout.Region(loaded: true, failureLabel: null));
    }

    [Fact]
    public void TheStepDrawsTheReaderFromTheseParts()
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/FirstRunWindow.xaml.cs.txt");
        string source = System.IO.File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = source.IndexOf("private UIElement TermsReader()", StringComparison.Ordinal);
        Assert.True(at >= 0, "TermsReader is gone");
        string body = source[at..source.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.Contains("TermsReaderLayout.Parts(", body, StringComparison.Ordinal);
        Assert.Contains("Height = TermsReaderLayout.Viewport", body, StringComparison.Ordinal);
        Assert.Contains("MaxWidth = TermsReaderLayout.Width", body, StringComparison.Ordinal);
        Assert.Contains("legal.Retry", body, StringComparison.Ordinal);
        Assert.Contains("legal.PrivacyUrl", body, StringComparison.Ordinal);
        Assert.DoesNotContain("Card(panel", body, StringComparison.Ordinal);
    }
}
