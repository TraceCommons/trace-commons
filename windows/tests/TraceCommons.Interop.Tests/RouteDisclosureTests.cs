using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// K11: the raw send, both enclaves, and where the witness came from. The
/// words are stand-ins here except in the cdylib tests; which blocks are
/// present is what matters.
/// </summary>
public class RouteDisclosureTests
{
    private const string WitnessFacts = """
        {"route":"witness","witness":{"state":"pinned","url":"https://witness.example",
         "signing_address":"0xab","pinned_measurements":["mrtd=aa","mrtd=bb"],
         "origin":"published_at_join"},"local_filter":null,
         "receipts":{"endpoint_configured":true,"check_attestation":false},"attested_bodies":false}
        """;

    private static string Payload(
        string route = "witness",
        bool witness = true,
        bool classifier = true,
        string? localFilter = null,
        bool receipts = true)
    {
        var facts = new Dictionary<string, object?>
        {
            ["route"] = route,
            ["witness"] = witness
                ? new Dictionary<string, object?>
                {
                    ["state"] = "pinned", ["url"] = "https://witness.invalid", ["signing_address"] = "0xab",
                    ["pinned_measurements"] = new[] { "mrtd=aa" }, ["origin"] = "published_at_join",
                }
                : null,
            ["local_filter"] = route == "local" ? "near_ai" : null,
            ["receipts"] = new Dictionary<string, bool> { ["endpoint_configured"] = false, ["check_attestation"] = false },
            ["attested_bodies"] = false,
        };
        var copy = new Dictionary<string, object?>
        {
            ["title"] = "TITLE",
            ["route"] = "ROUTE",
            ["witness"] = witness
                ? new Dictionary<string, object?>
                {
                    ["heading"] = "W", ["address_label"] = "A", ["signing_label"] = "S",
                    ["measurements_label"] = "M", ["check"] = "CHECK",
                    ["classifier"] = classifier ? "CLASSIFIER" : null, ["origin"] = "ORIGIN",
                }
                : null,
            ["local_filter"] = localFilter,
            ["receipts"] = receipts ? "RECEIPTS" : null,
            ["attested_bodies"] = null,
            ["session"] = new Dictionary<string, string>
            {
                ["heading"] = "H", ["before_label"] = "B", ["before_line"] = "BL",
                ["after_label"] = "AF", ["after_line"] = "AL",
            },
        };
        return JsonSerializer.Serialize(new Dictionary<string, object?> { ["facts"] = facts, ["copy"] = copy });
    }

    [Fact]
    public void TheWitnessRouteShowsTheWitnessItsPinsAndOrigin()
    {
        RouteDisclosure? value = RouteDisclosureSurface.Parse(Payload());
        Assert.NotNull(value);
        IReadOnlyList<DisclosureRow> rows = RouteDisclosureSurface.PanelRows(value, null);
        Assert.Equal("ROUTE", rows[0].Text);
        Assert.Contains(DisclosureRow.Pair("M", "mrtd=aa"), rows);
        Assert.Contains(DisclosureRow.Line("CLASSIFIER"), rows);
        Assert.Contains(DisclosureRow.Line("ORIGIN"), rows);
        Assert.Contains(DisclosureRow.Line("RECEIPTS"), rows);
    }

    /// <summary>Words for a block the facts do not have are refused, not rendered.</summary>
    [Fact]
    public void MismatchedWordsAreRefused()
    {
        Assert.Null(RouteDisclosureSurface.Parse(
            Payload(route: "local", witness: false, localFilter: "FILTER", receipts: true)));
        Assert.Null(RouteDisclosureSurface.Parse(
            Payload(route: "witness_refusing", classifier: true, receipts: false)));
        Assert.NotNull(RouteDisclosureSurface.Parse(
            Payload(route: "witness_refusing", classifier: false, receipts: false)));
        Assert.Null(RouteDisclosureSurface.Parse(Payload(localFilter: "FILTER")));
        Assert.Null(RouteDisclosureSurface.Parse("not json"));
    }

    [Fact]
    public void AnUnreadableDisclosureSaysSoAndNothingElse()
    {
        var unreadable = new RouteDisclosureUnreadable { Panel = "PANEL", Session = "SESSION" };
        Assert.Equal(new[] { DisclosureRow.Line("PANEL") }, RouteDisclosureSurface.PanelRows(null, unreadable));
        Assert.Equal(
            new[] { DisclosureRow.Line("SESSION") },
            RouteDisclosureSurface.SessionRows(null, unreadable, "1 KB", "4 KB", null, null));
    }

    [Fact]
    public void ASessionShowsBothSizesAndOnlyAVerificationItCanWord()
    {
        RouteDisclosure? value = RouteDisclosureSurface.Parse(Payload());
        var labels = new CertificateDetailCopy
        {
            Heading = "CH", MeasurementLabel = "CM", SignerLabel = "CS", VerifiedAtReview = "CV",
        };
        using JsonDocument held = JsonDocument.Parse(
            """{"verification":"verified_at_review","witness_measurement":"mrtd=aa","signer":"0xab"}""");
        IReadOnlyList<DisclosureRow> rows = RouteDisclosureSurface.SessionRows(
            value, null, "1 KB", "4 KB", held.RootElement, labels);
        Assert.Contains(DisclosureRow.Pair("B", "1 KB"), rows);
        Assert.Contains(DisclosureRow.Pair("AF", "4 KB"), rows);
        Assert.Contains(DisclosureRow.Line("ROUTE"), rows);
        Assert.Contains(DisclosureRow.Line("CV"), rows);

        using JsonDocument other = JsonDocument.Parse(
            """{"verification":"verified_now","witness_measurement":"mrtd=aa","signer":"0xab"}""");
        Assert.DoesNotContain(
            DisclosureRow.Line("CV"),
            RouteDisclosureSurface.SessionRows(value, null, "1 KB", "4 KB", other.RootElement, labels));
    }

    /// <summary>Through the real cdylib: the Rust's words, and exactly the fields this shell decodes.</summary>
    [Fact]
    public void TheCdylibWordsTheFactsAndExportsExactlyTheConsumedFields()
    {
        RouteDisclosure? value = RouteDisclosureSurface.ForFacts(WitnessFacts);
        Assert.NotNull(value);
        Assert.Contains("without asking you", value!.Copy.Witness!.Origin, StringComparison.Ordinal);
        Assert.Equal(new[] { "mrtd=aa", "mrtd=bb" }, value.Facts.Witness!.PinnedMeasurements);
        Assert.Null(RouteDisclosureSurface.ForFacts("""{"route":"somewhere_new"}"""));

        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_route_disclosure_copy(WitnessFacts))
            ?? throw new InvalidOperationException("tc_route_disclosure_copy returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        Assert.Equal(
            RouteDisclosure.ConsumedCopyFields.OrderBy(n => n, StringComparer.Ordinal),
            document.RootElement.GetProperty("copy").EnumerateObject()
                .Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal));

        Assert.NotNull(RouteDisclosureSurface.Unreadable());
        Assert.False(string.IsNullOrEmpty(RouteDisclosureSurface.CertificateLabels()?.VerifiedAtReview));
    }
}
