using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Terms that arrive but do not decode are a failure with a label, not "Loading"
/// for good: one block kind this shell does not know, in published terms, must
/// not block every new person at first run.
/// </summary>
public sealed class FirstRunTermsResolveTests
{
    private const string Valid = """
        {"document":{"version":"1","effective":"2026-01-01","in_effect":true,"title":"Terms","blocks":[{"kind":"paragraph","text":"Hello"}]},"content_sha256":"abc"}
        """;

    [Fact]
    public void ADocumentThatDecodesIsTheTermsWithNoLabel()
    {
        (FetchedTerms? terms, string? label) = FetchedTerms.Resolve(Valid, null);
        Assert.NotNull(terms);
        Assert.Null(label);
    }

    [Theory]
    [InlineData("""{"document":{"version":"1","effective":"2026-01-01","in_effect":true,"title":"T","blocks":[{"kind":"video","text":"x"}]},"content_sha256":"abc"}""")]
    [InlineData("""{"document":{"version":"1"},"content_sha256":"abc"}""")]
    [InlineData("not json")]
    [InlineData("")]
    [InlineData("{}")]
    public void AnUndecodableDocumentIsAFailureWithItsOwnLabel(string json)
    {
        (FetchedTerms? terms, string? label) = FetchedTerms.Resolve(json, null);
        Assert.Null(terms);
        Assert.Equal("terms-unreadable", label);
    }

    [Theory]
    [InlineData(null, "terms-unavailable")]
    [InlineData("terms-fetch-failed", "terms-fetch-failed")]
    public void NothingReturnedKeepsTheCoresLabelOrTheDefault(string? coreLabel, string expected)
    {
        (FetchedTerms? terms, string? label) = FetchedTerms.Resolve(null, coreLabel);
        Assert.Null(terms);
        Assert.Equal(expected, label);
    }

    [Fact]
    public void AFailureLabelIsTheFailedRegionNotLoading()
    {
        (FetchedTerms? terms, string? label) = FetchedTerms.Resolve("{}", null);
        Assert.Equal(TermsRegion.Failed, TermsReaderLayout.Region(terms is not null, label));
    }
}
