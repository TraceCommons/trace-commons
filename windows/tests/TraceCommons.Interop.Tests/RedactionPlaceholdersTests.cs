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
        Assert.Equal<int?>(1, only.Ordinal);
        Assert.True(only.HasLabel);
        Assert.Equal("<PRIVATE_LOCAL_PATH_1>", body.Substring(only.Start, only.Length));
    }

    [Fact]
    public void TheDisplayNameIsHumanReadable()
        => Assert.Equal(
            "private email",
            RedactionPlaceholders.Scan("<PRIVATE_PRIVATE_EMAIL_2>")[0].Display);

    [Fact]
    public void MultiplePlaceholdersAreFoundInOrder()
    {
        IReadOnlyList<RedactionPlaceholder> found = RedactionPlaceholders.Scan(
            "<PRIVATE_PRIVATE_EMAIL_1> then <PRIVATE_LOCAL_PATH_3> "
            + "then <PRIVATE_PRIVATE_EMAIL_1>");

        Assert.Equal(
            new[] { "PRIVATE_EMAIL", "LOCAL_PATH", "PRIVATE_EMAIL" },
            found.Select(p => p.Label).ToArray());
        Assert.Equal(new int?[] { 1, 3, 1 }, found.Select(p => p.Ordinal).ToArray());
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
    /// that itself ends in a number must not steal it. The label here is
    /// synthetic: this is about the grammar, not about a label the redactor
    /// actually mints.
    /// </summary>
    [Fact]
    public void ALabelContainingDigitsIsParsedCorrectly()
    {
        IReadOnlyList<RedactionPlaceholder> found =
            RedactionPlaceholders.Scan("<PRIVATE_SHA256_KEY_7>");

        Assert.Equal("SHA256_KEY", found[0].Label);
        Assert.Equal<int?>(7, found[0].Ordinal);
    }

    /// <summary>
    /// Only <c>apply_placeholder_regex</c> mints a number, and it is called
    /// for exactly two labels. A scan recognising only the numbered form would
    /// mark every path and NO SECRET, while the summary panel beside it
    /// reported those secrets as removed.
    /// </summary>
    [Fact]
    public void TheThreeFixedTokensAreFoundToo()
    {
        Assert.Single(RedactionPlaceholders.Scan("a [REDACTED] b"));
        Assert.Single(RedactionPlaceholders.Scan("a <REDACTED_PRIVATE_KEY> b"));
        Assert.Single(RedactionPlaceholders.Scan("a [REDACTED:person_name] b"));
    }

    /// <summary>
    /// Null, never zero. Faking a zero would put a value in the field that the
    /// redactor never assigned.
    /// </summary>
    [Fact]
    public void AFixedTokenCarriesNoOrdinal()
    {
        Assert.Null(RedactionPlaceholders.Scan("[REDACTED]")[0].Ordinal);
        Assert.Null(RedactionPlaceholders.Scan("<REDACTED_PRIVATE_KEY>")[0].Ordinal);
        Assert.Null(RedactionPlaceholders.Scan("[REDACTED:person_name]")[0].Ordinal);
    }

    /// <summary>
    /// Two of the four shapes can say that something left and not what, and a
    /// caller has to be able to tell which it is holding rather than printing
    /// an empty category name.
    /// </summary>
    [Fact]
    public void OnlyTheLabelledShapesNameTheirCategory()
    {
        Assert.False(RedactionPlaceholders.Scan("[REDACTED]")[0].HasLabel);
        Assert.False(RedactionPlaceholders.Scan("<REDACTED_PRIVATE_KEY>")[0].HasLabel);

        RedactionPlaceholder labelled = RedactionPlaceholders.Scan("[REDACTED:person_name]")[0];
        Assert.True(labelled.HasLabel);
        Assert.Equal("person_name", labelled.Label);
        Assert.Equal("person name", labelled.Display);
    }

    /// <summary>
    /// The whole token, not the bracket it starts with: a mark drawn over half
    /// a token leaves the other half reading as content that was never
    /// scrubbed.
    /// </summary>
    [Fact]
    public void AFixedTokenSpansItsWholeSelf()
    {
        const string body = "key was [REDACTED:aws_secret_key] here";

        RedactionPlaceholder found = RedactionPlaceholders.Scan(body)[0];

        Assert.Equal(
            "[REDACTED:aws_secret_key]",
            body.Substring(found.Start, found.Length));
    }

    [Fact]
    public void TextThatMerelyLooksLikeAPlaceholderIsIgnored()
    {
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE>"));
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE_LOCAL_PATH_>"));
        Assert.Empty(RedactionPlaceholders.Scan("<private_local_path_1>"));
        Assert.Empty(RedactionPlaceholders.Scan("PRIVATE_LOCAL_PATH_1"));
        Assert.Empty(RedactionPlaceholders.Scan("<REDACTED_PUBLIC_KEY>"));
    }

    /// <summary>
    /// An unclosed bracket must not let one "marker" run to the end of the
    /// body. It would mark the whole rest of the transcript as removed, and
    /// the chunker, which protects markers from being cut, would then refuse
    /// to cut anywhere inside it.
    /// </summary>
    [Fact]
    public void AnUnclosedBracketDoesNotSwallowTheRestOfTheBody()
        => Assert.Empty(RedactionPlaceholders.Scan("[REDACTED:oops\nand more text here"));

    /// <summary>
    /// Offsets index a C# string, which is UTF-16. The ABI reports UTF-8 byte
    /// offsets elsewhere and <c>TcPreview.Search</c> converts them; this scan
    /// runs on the already-converted string, so its offsets are UTF-16 and
    /// must survive text outside the BMP.
    /// </summary>
    [Fact]
    public void OffsetsIndexTheManagedStringIncludingAstralText()
    {
        const string body = "h\U0001F600llo <PRIVATE_LOCAL_PATH_1> world";

        IReadOnlyList<RedactionPlaceholder> found = RedactionPlaceholders.Scan(body);

        Assert.Equal("<PRIVATE_LOCAL_PATH_1>", body.Substring(found[0].Start, found[0].Length));
    }

    /// <summary>
    /// Everything this finds, <c>TranscriptMarkers</c> also marks. The
    /// transcript is drawn from that one, which is what the chunker protects;
    /// if the two disagreed about a token, a mark would be described by a
    /// label nothing had drawn, or a token the redactor emitted would be drawn
    /// as ordinary text. All four shapes, because it was
    /// <c>&lt;REDACTED_PRIVATE_KEY&gt;</c> that neither arm used to reach.
    /// </summary>
    [Fact]
    public void EveryPlaceholderIsAlsoAMarkerTheTranscriptDraws()
    {
        const string body =
            "a <PRIVATE_LOCAL_PATH_1> b [REDACTED] c <REDACTED_PRIVATE_KEY> d "
            + "[REDACTED:person_name] e";

        IReadOnlyList<(int Start, int Length)> marked = TranscriptMarkers.Split(body)
            .Where(run => run.IsMarker)
            .Select(run => (run.Start, run.Length))
            .ToList();

        Assert.Equal(
            marked,
            RedactionPlaceholders.Scan(body).Select(p => (p.Start, p.Length)).ToList());
    }
}
