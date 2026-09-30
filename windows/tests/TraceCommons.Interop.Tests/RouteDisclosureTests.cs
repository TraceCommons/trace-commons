using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
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

    /// <summary>Applies <paramref name="edit"/> to a payload, as a newer or broken core might send it.</summary>
    private static string Edited(Action<JsonObject> edit, string? json = null)
    {
        JsonObject root = JsonNode.Parse(json ?? Payload())!.AsObject();
        edit(root);
        return root.ToJsonString();
    }

    /// <summary>
    /// A block present with no words is refused, not silently dropped: a
    /// witness route with <c>receipts: ""</c> would otherwise render without
    /// its receipt disclosure. The macOS and Tauri parsers refuse these too.
    /// </summary>
    [Fact]
    public void APresentButEmptyOptionalLineIsRefusedNotDropped()
    {
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["copy"]!["receipts"] = "")));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["copy"]!["witness"]!["classifier"] = "")));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r =>
        {
            r["facts"]!["attested_bodies"] = true;
            r["copy"]!["attested_bodies"] = "";
        })));
        string local = Payload(route: "local", witness: false, localFilter: "FILTER", receipts: false);
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["copy"]!["local_filter"] = "", local)));

        // The same payloads with words are read, so each refusal above is the empty line's.
        Assert.NotNull(RouteDisclosureSurface.Parse(Edited(r =>
        {
            r["facts"]!["attested_bodies"] = true;
            r["copy"]!["attested_bodies"] = "BODIES";
        })));
        Assert.NotNull(RouteDisclosureSurface.Parse(local));
    }

    /// <summary>A witness with no address or signing key is refused, not drawn as empty rows.</summary>
    [Fact]
    public void AWitnessWithoutAnAddressOrSigningKeyIsRefused()
    {
        Assert.NotNull(RouteDisclosureSurface.Parse(Payload()));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["facts"]!["witness"]!.AsObject().Remove("url"))));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["facts"]!["witness"]!.AsObject().Remove("signing_address"))));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["facts"]!["witness"]!["url"] = "")));
        Assert.Null(RouteDisclosureSurface.Parse(Edited(r => r["facts"]!["witness"]!["signing_address"] = null)));
    }

    /// <summary>Each certificate label is required; an empty or missing one refuses the set.</summary>
    [Fact]
    public void CertificateLabelsWithAnEmptyLabelAreRefused()
    {
        const string whole =
            """{"heading":"CH","measurement_label":"CM","signer_label":"CS","verified_at_review":"CV"}""";
        Assert.NotNull(RouteDisclosureSurface.ParseCertificateLabels(whole));
        foreach (string field in CertificateDetailCopy.ConsumedFields)
        {
            Assert.Null(RouteDisclosureSurface.ParseCertificateLabels(Edited(r => r[field] = "", whole)));
            Assert.Null(RouteDisclosureSurface.ParseCertificateLabels(Edited(r => r.Remove(field), whole)));
        }
    }

    /// <summary>The unreadable lines, title included, are each required.</summary>
    [Fact]
    public void UnreadableLinesWithAnEmptyLineAreRefused()
    {
        const string whole = """{"title":"T","panel":"P","session":"S"}""";
        Assert.NotNull(RouteDisclosureSurface.ParseUnreadable(whole));
        foreach (string field in RouteDisclosureUnreadable.ConsumedFields)
        {
            Assert.Null(RouteDisclosureSurface.ParseUnreadable(Edited(r => r[field] = "", whole)));
        }
    }

    /// <summary>
    /// The section keeps its title when the route cannot be read, so the
    /// unreadable line sits under a named heading rather than an empty one.
    /// </summary>
    [Fact]
    public void AnUnreadableDisclosureIsStillTitled()
    {
        var unreadable = new RouteDisclosureUnreadable { Title = "UNREADABLE TITLE", Panel = "PANEL", Session = "SESSION" };
        Assert.Equal("UNREADABLE TITLE", RouteDisclosureSurface.PanelTitle(null, unreadable));
        Assert.Equal("TITLE", RouteDisclosureSurface.PanelTitle(RouteDisclosureSurface.Parse(Payload()), unreadable));
        Assert.Equal(string.Empty, RouteDisclosureSurface.PanelTitle(null, null));
    }

    [Fact]
    public void AnUnreadableDisclosureSaysSoAndNothingElse()
    {
        var unreadable = new RouteDisclosureUnreadable { Title = "TITLE", Panel = "PANEL", Session = "SESSION" };
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
        // Every other field as the readable payload above has it, so the
        // route alone is what the cdylib refuses.
        string unknownRoute = WitnessFacts.Replace(
            "\"route\":\"witness\"", "\"route\":\"somewhere_new\"", StringComparison.Ordinal);
        Assert.NotEqual(WitnessFacts, unknownRoute);
        Assert.Null(RouteDisclosureSurface.ForFacts(unknownRoute));

        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_route_disclosure_copy(WitnessFacts))
            ?? throw new InvalidOperationException("tc_route_disclosure_copy returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        JsonElement copy = document.RootElement.GetProperty("copy");
        Assert.Equal(
            RouteDisclosure.ConsumedCopyFields.OrderBy(n => n, StringComparer.Ordinal),
            copy.EnumerateObject().Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal));
        // The nested blocks too, as macOS (RouteDisclosureBridgeTests): a
        // sentence added to `witness` or `session` in Rust must not be dropped
        // silently here. The witness route sends every witness key.
        Assert.Equal(
            WitnessDisclosureCopy.ConsumedFields.OrderBy(n => n, StringComparer.Ordinal),
            copy.GetProperty("witness").EnumerateObject()
                .Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal));
        Assert.Equal(
            SessionSendCopy.ConsumedFields.OrderBy(n => n, StringComparer.Ordinal),
            copy.GetProperty("session").EnumerateObject()
                .Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal));

        Assert.False(string.IsNullOrEmpty(RouteDisclosureSurface.Unreadable()?.Title));
        Assert.False(string.IsNullOrEmpty(RouteDisclosureSurface.CertificateLabels()?.VerifiedAtReview));
    }

    /// <summary>
    /// The unreadable lines and the certificate labels export exactly the
    /// fields this shell decodes, so a label added in Rust cannot be dropped
    /// silently here. macOS asserts the same (RouteDisclosureBridgeTests).
    /// </summary>
    [Fact]
    public void TheCdylibExportsExactlyTheConsumedUnreadableAndCertificateFields()
    {
        AssertExportsExactly(
            RouteDisclosureUnreadable.ConsumedFields,
            NativeMethods.TakeOwnedString(NativeMethods.tc_route_disclosure_unreadable_copy()));
        AssertExportsExactly(
            CertificateDetailCopy.ConsumedFields,
            NativeMethods.TakeOwnedString(NativeMethods.tc_certificate_detail_copy()));
    }

    private static void AssertExportsExactly(IReadOnlyList<string> consumed, string? json)
    {
        Assert.NotNull(json);
        using JsonDocument document = JsonDocument.Parse(json!);
        Assert.Equal(
            consumed.OrderBy(n => n, StringComparer.Ordinal),
            document.RootElement.EnumerateObject().Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal));
    }

    /// <summary>
    /// The preview sheet is shown before the disclosure's two daemon calls
    /// finish, as the macOS and GTK sheets are: <c>IsLoading</c> is cleared
    /// first and the block fills in after.
    /// </summary>
    [Fact]
    public void ThePreviewSheetDoesNotWaitOnTheDisclosure()
    {
        string load = MethodBody(AppSource("PreviewSheetViewModel.cs.txt"), "public async Task LoadAsync()");
        int shown = load.LastIndexOf("IsLoading = false;", StringComparison.Ordinal);
        int filled = load.IndexOf("FillSessionDisclosureAsync(", StringComparison.Ordinal);
        Assert.True(shown >= 0, "LoadAsync never clears IsLoading");
        Assert.True(filled >= 0, "LoadAsync never fills the session disclosure");
        Assert.True(shown < filled, "the sheet waits on the disclosure before it is shown");
    }

    /// <summary>
    /// Settings repaints the disclosure on every daemon status event, not
    /// only on load, and titles it from the core when it is unreadable.
    /// </summary>
    [Fact]
    public void SettingsRefreshesTheDisclosureOnStatusAndTitlesItWhenUnreadable()
    {
        string source = AppSource("ContributorSettingsViewModel.cs.txt");
        Assert.Contains(
            "RefreshDisclosureAsync()",
            MethodBody(source, "private async void OnDaemonStatusChanged()"),
            StringComparison.Ordinal);
        Assert.Contains(
            "RouteDisclosureSurface.PanelTitle(",
            MethodBody(source, "public async Task RefreshDisclosureAsync()"),
            StringComparison.Ordinal);
    }

    /// <summary>An app-layer source copied beside the test assembly, without its comment lines.</summary>
    private static string AppSource(string name)
    {
        string path = Path.Combine(AppContext.BaseDirectory, name);
        Assert.True(File.Exists(path), $"the app source was not copied to {path}");
        return string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
    }

    /// <summary>The body of the member declared by <paramref name="signature"/>, braces balanced.</summary>
    private static string MethodBody(string source, string signature)
    {
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"`{signature}` was not found");
        int open = source.IndexOf('{', start);
        int depth = 0;
        for (int i = open; i < source.Length; i++)
        {
            if (source[i] == '{')
            {
                depth++;
            }
            else if (source[i] == '}' && --depth == 0)
            {
                return source[open..(i + 1)];
            }
        }

        throw new InvalidOperationException($"`{signature}` has an unclosed body");
    }
}
