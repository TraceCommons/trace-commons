using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// K11: what leaves this machine, to whom, and what this client checked.
/// <c>Facts</c> is the daemon's <c>route_disclosure</c> answer; <c>Copy</c> is
/// the Rust's words for exactly those facts, both handed back by
/// <c>tc_route_disclosure_copy</c>. Nothing here is written in C#.
/// </summary>
public sealed record RouteDisclosure
{
    [JsonPropertyName("facts")] public RouteFacts Facts { get; init; } = new();
    [JsonPropertyName("copy")] public RouteDisclosureCopy Copy { get; init; } = new();

    /// <summary>The <c>copy</c> fields this shell decodes, compared against the export.</summary>
    public static IReadOnlyList<string> ConsumedCopyFields { get; } = new[]
    {
        "title", "route", "witness", "local_filter", "receipts", "attested_bodies", "session",
    };

    /// <summary>Whether a session leaves this machine unredacted.</summary>
    public bool SendsToWitness => Facts.Route == "witness";
}

public sealed record RouteFacts
{
    [JsonPropertyName("route")] public string Route { get; init; } = string.Empty;
    [JsonPropertyName("witness")] public WitnessFacts? Witness { get; init; }
    [JsonPropertyName("local_filter")] public string? LocalFilter { get; init; }
    [JsonPropertyName("attested_bodies")] public bool AttestedBodies { get; init; }
}

public sealed record WitnessFacts
{
    [JsonPropertyName("url")] public string Url { get; init; } = string.Empty;
    [JsonPropertyName("signing_address")] public string SigningAddress { get; init; } = string.Empty;
    [JsonPropertyName("pinned_measurements")] public List<string> PinnedMeasurements { get; init; } = new();
    [JsonPropertyName("origin")] public string Origin { get; init; } = string.Empty;
}

public sealed record RouteDisclosureCopy
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;
    [JsonPropertyName("route")] public string Route { get; init; } = string.Empty;
    [JsonPropertyName("witness")] public WitnessDisclosureCopy? Witness { get; init; }
    [JsonPropertyName("local_filter")] public string? LocalFilter { get; init; }
    [JsonPropertyName("receipts")] public string? Receipts { get; init; }
    [JsonPropertyName("attested_bodies")] public string? AttestedBodies { get; init; }
    [JsonPropertyName("session")] public SessionSendCopy Session { get; init; } = new();
}

public sealed record WitnessDisclosureCopy
{
    [JsonPropertyName("heading")] public string Heading { get; init; } = string.Empty;
    [JsonPropertyName("address_label")] public string AddressLabel { get; init; } = string.Empty;
    [JsonPropertyName("signing_label")] public string SigningLabel { get; init; } = string.Empty;
    [JsonPropertyName("measurements_label")] public string MeasurementsLabel { get; init; } = string.Empty;
    [JsonPropertyName("check")] public string Check { get; init; } = string.Empty;
    /// <summary>The second enclave. Only on the witness route.</summary>
    [JsonPropertyName("classifier")] public string? Classifier { get; init; }
    [JsonPropertyName("origin")] public string Origin { get; init; } = string.Empty;

    /// <summary>
    /// The nested <c>copy.witness</c> fields this shell decodes, compared
    /// against the export: the both-enclaves and origin sentences live here.
    /// </summary>
    public static IReadOnlyList<string> ConsumedFields { get; } = new[]
    {
        "heading", "address_label", "signing_label", "measurements_label", "check",
        "classifier", "origin",
    };
}

public sealed record SessionSendCopy
{
    [JsonPropertyName("heading")] public string Heading { get; init; } = string.Empty;
    [JsonPropertyName("before_label")] public string BeforeLabel { get; init; } = string.Empty;
    [JsonPropertyName("before_line")] public string BeforeLine { get; init; } = string.Empty;
    [JsonPropertyName("after_label")] public string AfterLabel { get; init; } = string.Empty;
    [JsonPropertyName("after_line")] public string AfterLine { get; init; } = string.Empty;

    /// <summary>The nested <c>copy.session</c> fields this shell decodes, compared against the export.</summary>
    public static IReadOnlyList<string> ConsumedFields { get; } = new[]
    {
        "heading", "before_label", "before_line", "after_label", "after_line",
    };
}

/// <summary>What a surface says when the disclosure cannot be read, under its title.</summary>
public sealed record RouteDisclosureUnreadable
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;
    [JsonPropertyName("panel")] public string Panel { get; init; } = string.Empty;
    [JsonPropertyName("session")] public string Session { get; init; } = string.Empty;

    public static IReadOnlyList<string> ConsumedFields { get; } = new[] { "title", "panel", "session" };
}

/// <summary>The labels for <c>certificate_detail</c>.</summary>
public sealed record CertificateDetailCopy
{
    [JsonPropertyName("heading")] public string Heading { get; init; } = string.Empty;
    [JsonPropertyName("measurement_label")] public string MeasurementLabel { get; init; } = string.Empty;
    [JsonPropertyName("signer_label")] public string SignerLabel { get; init; } = string.Empty;
    [JsonPropertyName("verified_at_review")] public string VerifiedAtReview { get; init; } = string.Empty;

    public static IReadOnlyList<string> ConsumedFields { get; } = new[]
    {
        "heading", "measurement_label", "signer_label", "verified_at_review",
    };
}

/// <summary>One line of a disclosure, as the XAML draws it.</summary>
public sealed record DisclosureRow(string Kind, string Text, string Value)
{
    public bool IsHeading => Kind == "heading";
    public bool IsText => Kind == "text";
    public bool IsValue => Kind == "value";

    public static DisclosureRow Heading(string text) => new("heading", text, string.Empty);
    public static DisclosureRow Line(string text) => new("text", text, string.Empty);
    public static DisclosureRow Pair(string label, string value) => new("value", label, value);
}

/// <summary>
/// K11 across the C ABI, flattened into rows the XAML lays out.
/// </summary>
/// <remarks>
/// Nothing in this file is a word. A payload whose words do not match its
/// facts is refused -- the ABI sends only the blocks true of the route, so a
/// mismatch is a defect, and the safe rendering of a defect in a privacy
/// disclosure is none at all.
/// </remarks>
public static class RouteDisclosureSurface
{
    /// <summary>The Rust's words for the daemon's facts, or null if unreadable.</summary>
    public static RouteDisclosure? ForFacts(string? factsJson)
    {
        if (string.IsNullOrWhiteSpace(factsJson))
        {
            return null;
        }

        return Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_route_disclosure_copy(factsJson)));
    }

    /// <summary>The lines shown when the disclosure cannot be read.</summary>
    public static RouteDisclosureUnreadable? Unreadable() =>
        ParseUnreadable(NativeMethods.TakeOwnedString(NativeMethods.tc_route_disclosure_unreadable_copy()));

    /// <summary>The labels for <c>certificate_detail</c>.</summary>
    public static CertificateDetailCopy? CertificateLabels() =>
        ParseCertificateLabels(NativeMethods.TakeOwnedString(NativeMethods.tc_certificate_detail_copy()));

    /// <summary>The payload half of <see cref="Unreadable"/>, testable without the cdylib.</summary>
    internal static RouteDisclosureUnreadable? ParseUnreadable(string? json)
    {
        RouteDisclosureUnreadable? value = Deserialize<RouteDisclosureUnreadable>(json);
        return value is null || AnyEmpty(value.Title, value.Panel, value.Session) ? null : value;
    }

    /// <summary>
    /// The payload half of <see cref="CertificateLabels"/>, testable without
    /// the cdylib. Every label is required: an empty one would draw a value
    /// with nothing saying what it is.
    /// </summary>
    internal static CertificateDetailCopy? ParseCertificateLabels(string? json)
    {
        CertificateDetailCopy? value = Deserialize<CertificateDetailCopy>(json);
        return value is null
            || AnyEmpty(value.Heading, value.MeasurementLabel, value.SignerLabel, value.VerifiedAtReview)
            ? null
            : value;
    }

    /// <summary>
    /// The Settings section's title: the disclosure's, or the core's title
    /// for an unreadable one, so the unreadable line is never under a blank
    /// heading.
    /// </summary>
    public static string PanelTitle(RouteDisclosure? disclosure, RouteDisclosureUnreadable? unreadable) =>
        disclosure?.Copy.Title ?? unreadable?.Title ?? string.Empty;

    /// <summary>The payload half of <see cref="ForFacts"/>, testable without the cdylib.</summary>
    internal static RouteDisclosure? Parse(string? json)
    {
        RouteDisclosure? value = Deserialize<RouteDisclosure>(json);
        return value is not null && IsConsistent(value) ? value : null;
    }

    /// <summary>The Settings section's rows.</summary>
    public static IReadOnlyList<DisclosureRow> PanelRows(RouteDisclosure? disclosure, RouteDisclosureUnreadable? unreadable)
    {
        var rows = new List<DisclosureRow>();
        if (disclosure is null)
        {
            if (unreadable is not null)
            {
                rows.Add(DisclosureRow.Line(unreadable.Panel));
            }

            return rows;
        }

        RouteDisclosureCopy copy = disclosure.Copy;
        rows.Add(DisclosureRow.Line(copy.Route));
        AddLine(rows, copy.LocalFilter);
        if (disclosure.Facts.Witness is { } facts && copy.Witness is { } witness)
        {
            rows.Add(DisclosureRow.Heading(witness.Heading));
            rows.Add(DisclosureRow.Pair(witness.AddressLabel, facts.Url));
            rows.Add(DisclosureRow.Pair(witness.SigningLabel, facts.SigningAddress));
            foreach (string pin in facts.PinnedMeasurements)
            {
                rows.Add(DisclosureRow.Pair(witness.MeasurementsLabel, pin));
            }

            rows.Add(DisclosureRow.Line(witness.Check));
            AddLine(rows, witness.Classifier);
            rows.Add(DisclosureRow.Line(witness.Origin));
        }

        AddLine(rows, copy.AttestedBodies);
        AddLine(rows, copy.Receipts);
        return rows;
    }

    /// <summary>
    /// One session's rows for the preview sheet: both sizes, where each goes,
    /// and the certificate a witnessed review left, if the daemon reported one.
    /// </summary>
    public static IReadOnlyList<DisclosureRow> SessionRows(
        RouteDisclosure? disclosure,
        RouteDisclosureUnreadable? unreadable,
        string raw,
        string wouldSend,
        JsonElement? certificate,
        CertificateDetailCopy? labels)
    {
        var rows = new List<DisclosureRow>();
        if (disclosure is null)
        {
            if (unreadable is not null)
            {
                rows.Add(DisclosureRow.Line(unreadable.Session));
            }

            return rows;
        }

        SessionSendCopy session = disclosure.Copy.Session;
        rows.Add(DisclosureRow.Heading(session.Heading));
        rows.Add(DisclosureRow.Pair(session.BeforeLabel, raw));
        rows.Add(DisclosureRow.Line(session.BeforeLine));
        AddLine(rows, disclosure.Copy.LocalFilter);
        rows.Add(DisclosureRow.Pair(session.AfterLabel, wouldSend));
        rows.Add(DisclosureRow.Line(session.AfterLine));
        if (disclosure.SendsToWitness)
        {
            rows.Add(DisclosureRow.Line(disclosure.Copy.Route));
        }

        if (labels is not null && certificate is { ValueKind: JsonValueKind.Object } detail
            && Text(detail, "verification") == "verified_at_review"
            && Text(detail, "witness_measurement") is { Length: > 0 } measurement
            && Text(detail, "signer") is { Length: > 0 } signer)
        {
            rows.Add(DisclosureRow.Heading(labels.Heading));
            rows.Add(DisclosureRow.Pair(labels.MeasurementLabel, measurement));
            rows.Add(DisclosureRow.Pair(labels.SignerLabel, signer));
            rows.Add(DisclosureRow.Line(labels.VerifiedAtReview));
        }

        return rows;
    }

    private static bool IsConsistent(RouteDisclosure value)
    {
        RouteDisclosureCopy c = value.Copy;
        bool witnessRoute = value.SendsToWitness;
        var required = new List<string?>
        {
            c.Title, c.Route, c.Session.Heading, c.Session.BeforeLabel, c.Session.BeforeLine,
            c.Session.AfterLabel, c.Session.AfterLine,
        };
        if (c.Witness is { } w)
        {
            required.AddRange(new[] { w.Heading, w.AddressLabel, w.SigningLabel, w.MeasurementsLabel, w.Check, w.Origin });
        }

        // The witness's address and signing key are drawn as values; missing,
        // they would render as empty rows under their labels.
        if (value.Facts.Witness is { } facts)
        {
            required.AddRange(new[] { facts.Url, facts.SigningAddress });
        }

        if (required.Exists(string.IsNullOrEmpty))
        {
            return false;
        }

        // A block that is present must have words. Absent is "not true of
        // this route"; present and empty is a defect, refused rather than
        // dropped by AddLine, as the macOS and Tauri parsers do.
        if (new[] { c.LocalFilter, c.Receipts, c.AttestedBodies, c.Witness?.Classifier }
            .Any(line => line is { Length: 0 }))
        {
            return false;
        }

        return (value.Facts.Witness is null) == (c.Witness is null)
            && (c.Witness is null || (c.Witness.Classifier is not null) == witnessRoute)
            && (c.LocalFilter is not null) == (value.Facts.Route == "local")
            && (c.Receipts is not null) == witnessRoute
            && (c.AttestedBodies is not null) == (witnessRoute && value.Facts.AttestedBodies);
    }

    private static bool AnyEmpty(params string?[] lines) => lines.Any(string.IsNullOrEmpty);

    private static void AddLine(List<DisclosureRow> rows, string? line)
    {
        if (!string.IsNullOrEmpty(line))
        {
            rows.Add(DisclosureRow.Line(line));
        }
    }

    private static string? Text(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static T? Deserialize<T>(string? json)
        where T : class
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            return JsonSerializer.Deserialize<T>(json);
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
