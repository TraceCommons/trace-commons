using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The redactor leaves a typed placeholder where it removed a value, and
/// those tokens are already in the transcript the ABI hands us -- rendered,
/// until now, as ordinary text. Finding them is what lets the preview say
/// WHERE something was cut, which is more than a category count can.
/// </summary>
public class RedactionPlaceholdersTests
{
    [Fact]
    public void ABodyWithNoPlaceholdersScansToNothing()
    {
        Assert.Empty(RedactionPlaceholders.Scan("just some ordinary text"));
        Assert.Empty(RedactionPlaceholders.Scan(""));
    }

    [Fact]
    public void ASinglePlaceholderIsFound()
    {
        const string body = "ran the build in <PRIVATE_LOCAL_PATH_1> and stopped";

        IReadOnlyList<RedactionPlaceholder> found = RedactionPlaceholders.Scan(body);

        RedactionPlaceholder only = Assert.Single(found);
        Assert.Equal("LOCAL_PATH", only.Label);
        Assert.Equal(1, only.Ordinal);
        Assert.Equal("<PRIVATE_LOCAL_PATH_1>", body.Substring(only.Start, only.Length));
    }

    [Fact]
    public void TheDisplayNameIsHumanReadable()
        => Assert.Equal(
            "contextual entropy",
            RedactionPlaceholders.Scan("<PRIVATE_CONTEXTUAL_ENTROPY_2>")[0].Display);

    [Fact]
    public void MultiplePlaceholdersAreFoundInOrder()
    {
        IReadOnlyList<RedactionPlaceholder> found = RedactionPlaceholders.Scan(
            "<PRIVATE_SECRET_1> then <PRIVATE_LOCAL_PATH_3> then <PRIVATE_SECRET_1>");

        Assert.Equal(
            new[] { "SECRET", "LOCAL_PATH", "SECRET" },
            found.Select(p => p.Label).ToArray());
        Assert.Equal(new[] { 1, 3, 1 }, found.Select(p => p.Ordinal).ToArray());
    }

    /// <summary>
    /// The numbering is per DISTINCT VALUE, so one value referenced twice
    /// carries one ordinal twice. That property is what the summary's
    /// distinct counts rest on.
    /// </summary>
    [Fact]
    public void OneValueTwiceCarriesOneOrdinalTwice()
    {
        IReadOnlyList<RedactionPlaceholder> found =
            RedactionPlaceholders.Scan("<PRIVATE_LOCAL_PATH_1> and <PRIVATE_LOCAL_PATH_1>");

        Assert.Equal(2, found.Count);
        Assert.Single(found.Select(p => p.Ordinal).Distinct());
    }

    /// <summary>
    /// The ordinal is the last underscore-delimited run of digits, so a label
    /// that itself ends in a number must not steal it.
    /// </summary>
    [Fact]
    public void ALabelContainingDigitsIsParsedCorrectly()
    {
        IReadOnlyList<RedactionPlaceholder> found =
            RedactionPlaceholders.Scan("<PRIVATE_SHA256_KEY_7>");

        Assert.Equal("SHA256_KEY", found[0].Label);
        Assert.Equal(7, found[0].Ordinal);
    }

    [Fact]
    public void TextThatMerelyLooksLikeAPlaceholderIsIgnored()
    {
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE>"));
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE_LOCAL_PATH_>"));
        Assert.Empty(RedactionPlaceholders.Scan("<private_local_path_1>"));
        Assert.Empty(RedactionPlaceholders.Scan("PRIVATE_LOCAL_PATH_1"));
    }

    /// <summary>
    /// Offsets index a C# string, which is UTF-16. The ABI reports UTF-8 byte
    /// offsets elsewhere and <c>TcPreview.Search</c> converts them; this scan
    /// runs on the already-converted string, so its offsets are UTF-16 and
    /// must survive text outside the BMP.
    /// </summary>
    [Fact]
    public void OffsetsIndexTheManagedStringIncludingAstralText()
    {
        const string body = "h\U0001F600llo <PRIVATE_SECRET_1> world";

        IReadOnlyList<RedactionPlaceholder> found = RedactionPlaceholders.Scan(body);

        Assert.Equal("<PRIVATE_SECRET_1>", body.Substring(found[0].Start, found[0].Length));
    }

    /// <summary>
    /// Everything this finds, <c>TranscriptMarkers</c> also marks. The
    /// transcript is drawn from that one, which covers a second marker family
    /// and is what the chunker protects; if the two disagreed about a
    /// <c>&lt;PRIVATE_*&gt;</c> token, a mark would be described by a label
    /// nothing had drawn.
    /// </summary>
    [Fact]
    public void EveryPlaceholderIsAlsoAMarkerTheTranscriptDraws()
    {
        const string body = "a <PRIVATE_SECRET_1> b <PRIVATE_LOCAL_PATH_2> c";

        IReadOnlyList<(int Start, int Length)> marked = TranscriptMarkers.Split(body)
            .Where(run => run.IsMarker)
            .Select(run => (run.Start, run.Length))
            .ToList();

        Assert.Equal(
            marked,
            RedactionPlaceholders.Scan(body).Select(p => (p.Start, p.Length)).ToList());
    }
}
