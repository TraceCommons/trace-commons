using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The commons' credit summary, the parts Home reads
/// (<c>DaemonData.CommonsCreditSummary</c>; <c>commons_credit_summary</c>).
/// </summary>
public sealed record CommonsCredit(
    bool PostureKnown,
    string? SettlementExplanation,
    bool PointsKnown,
    double? PointsLifetimeEarned,
    bool AccountSessionRequired)
{
    /// <summary>
    /// Decodes the reply. Anything unreadable is null: Home then shows a dash,
    /// never an earlier reply as current.
    /// </summary>
    public static CommonsCredit? Parse(JsonElement? result)
    {
        if (result is not { ValueKind: JsonValueKind.Object } o)
        {
            return null;
        }

        static string? Str(JsonElement o, string name) =>
            o.TryGetProperty(name, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
        static double? Num(JsonElement o, string name) =>
            o.TryGetProperty(name, out JsonElement v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : null;

        return new CommonsCredit(
            PostureKnown: Str(o, "posture_state") == "known",
            SettlementExplanation: Str(o, "commons_settlement_explanation"),
            PointsKnown: Str(o, "points_state") == "known",
            PointsLifetimeEarned: Num(o, "commons_points_lifetime_earned"),
            AccountSessionRequired: Str(o, "points_unknown_reason") == "account-session-required");
    }
}

/// <summary>One tool's tokens today, from <c>insights_glance</c>.</summary>
public readonly record struct GlanceBar(string Tool, ulong Tokens);

/// <summary>
/// Home's formatting rules (<c>HomeFormat</c>, <c>HomeClosedCards</c> and
/// <c>HomeInsightsGraphic.bars</c>, macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift
/// and HomeInsightsVisual.swift). Pure, so the rules are tested; the figures
/// themselves are the core's (<c>tc_count_figure</c>, <c>tc_credit_figure</c>).
/// </summary>
public static class HomeFacts
{
    /// <summary>The unknown figure: a dash, never a zero (HomeViews.swift:2014-2018).</summary>
    public const string Dash = "—";

    /// <summary>HomeViews.swift:2014-2018: a count, or a dash when the core did not say. Zero is a number.</summary>
    public static string Count(int? value, Func<ulong, string?> figure)
    {
        if (value is not int n)
        {
            return Dash;
        }

        if (n < 0)
        {
            return n.ToString(System.Globalization.CultureInfo.InvariantCulture);
        }

        string? text = figure((ulong)n);
        return string.IsNullOrEmpty(text) ? n.ToString(System.Globalization.CultureInfo.InvariantCulture) : text;
    }

    /// <summary>HomeViews.swift:2041-2043: a credit figure in the core's one rule, a dash for none.</summary>
    public static string Points(double? value, Func<double, string?> figure)
    {
        if (value is not double v)
        {
            return Dash;
        }

        string? text = figure(v);
        return string.IsNullOrEmpty(text) ? Dash : text;
    }

    /// <summary>HomeViews.swift:2131-2136: what pending credit waits on, in the commons' words; nil when it has not said.</summary>
    public static string? PendingCondition(CommonsCredit? credit) =>
        credit is { PostureKnown: true, SettlementExplanation: { Length: > 0 } explanation } ? explanation : null;

    /// <summary>HomeViews.swift:2047-2049: pending credit only beside the commons' statement (D6).</summary>
    public static string PendingFigure(double? pending, CommonsCredit? credit, Func<double, string?> figure) =>
        PendingCondition(credit) is null ? Dash : Points(pending, figure);

    /// <summary>
    /// HomeViews.swift:2120-2124: Home's two credit tiles. Received is
    /// History's Final, 0 once History has answered with nothing decided;
    /// pending is a figure only with the commons' statement.
    /// </summary>
    public static (string Received, string Pending) CreditTiles(double? final, double? pending, CommonsCredit? credit, Func<double, string?> figure)
    {
        double? received = final ?? (pending is null ? null : 0);
        return (Points(received, figure), PendingFigure(pending, credit, figure));
    }

    /// <summary>HomeViews.swift:2031-2036: the account's lifetime points under this machine's figure, in the core's words.</summary>
    public static string? CreditAccountTotal(CommonsCredit? credit, string? template, Func<double, string?> figure)
    {
        if (credit is not { PointsKnown: true, PointsLifetimeEarned: double lifetime } || string.IsNullOrEmpty(template))
        {
            return null;
        }

        return template.Replace("{amount}", Points(lifetime, figure), StringComparison.Ordinal);
    }

    /// <summary>HomeViews.swift:2005-2008: a card heading from the core's copy, or null so no card is drawn on a word of this shell's own.</summary>
    public static string? CardHeading(string? word) => string.IsNullOrEmpty(word) ? null : word;

    /// <summary>
    /// The compiled campaign's fingerprint (HomeViews.swift:138-141): every
    /// field it shows, so a changed campaign is a different one. The image URL
    /// is part of it; this shell draws no image (owner decision W11-D3 (a)).
    /// </summary>
    public static string CampaignFingerprint(Campaign campaign) => string.Join(
        "\u001F",
        campaign.Id,
        campaign.Title,
        campaign.Body,
        campaign.ActionTitle,
        campaign.ActionUrl,
        campaign.Symbol,
        campaign.ImageUrl ?? string.Empty);

    /// <summary>HomeViews.swift:145-147: shown while promotions are on and it is not the one closed, as it was then.</summary>
    public static bool ShowsCampaign(string fingerprint, string closed, bool promotionsOn) => promotionsOn && fingerprint != closed;

    /// <summary>HomeInsightsVisual.swift:45-46: the most tools drawn.</summary>
    public const int ToolLimit = 4;

    /// <summary>HomeInsightsVisual.swift:47-49: bar width and tallest bar.</summary>
    public const double BarWidth = 30;
    public const double MaxBarHeight = 190;

    /// <summary>
    /// HomeInsightsVisual.swift:51-59: tools with known, positive tokens, the
    /// most first, at most four; none for a disabled or unreadable glance.
    /// </summary>
    public static IReadOnlyList<GlanceBar> Bars(JsonElement? glance)
    {
        if (glance is not { ValueKind: JsonValueKind.Object } g)
        {
            return Array.Empty<GlanceBar>();
        }

        bool enabled = g.TryGetProperty("enabled", out JsonElement e) && e.ValueKind == JsonValueKind.True;
        bool unreadable = g.TryGetProperty("readable", out JsonElement r) && r.ValueKind == JsonValueKind.False;
        if (!enabled || unreadable || !g.TryGetProperty("tools", out JsonElement tools) || tools.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<GlanceBar>();
        }

        var bars = new List<GlanceBar>();
        foreach (JsonElement tool in tools.EnumerateArray())
        {
            if (tool.ValueKind == JsonValueKind.Object
                && tool.TryGetProperty("tool", out JsonElement name) && name.ValueKind == JsonValueKind.String
                && tool.TryGetProperty("tokens", out JsonElement tokens) && tokens.ValueKind == JsonValueKind.Number
                && tokens.TryGetUInt64(out ulong count) && count > 0)
            {
                bars.Add(new GlanceBar(name.GetString() ?? string.Empty, count));
            }
        }

        // A stable sort, as Swift's `sorted` keeps equal tokens in input order.
        return bars.OrderByDescending(b => b.Tokens).Take(ToolLimit).ToArray();
    }

    /// <summary>HomeInsightsVisual.swift:75-76: a bar's height, its share of the longest, never under 8.</summary>
    public static double BarHeight(ulong tokens, ulong most) => most == 0 ? 8 : Math.Max(8, MaxBarHeight * tokens / most);
}

/// <summary>
/// The NEAR AI campaign Home promotes: the compiled one only (owner decision
/// W11-D3 (a), 2026-11-18), from <c>tc_private_ai_campaign_json</c>.
/// </summary>
public sealed record Campaign(string Id, string Title, string Body, string ActionTitle, string ActionUrl, string Symbol, string? ImageUrl)
{
    /// <summary>The compiled campaign, or null if the core gave none or one this shell cannot read.</summary>
    public static Campaign? Compiled() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_private_ai_campaign_json()));

    /// <summary>Decodes the core's campaign object; any missing required field is no campaign.</summary>
    public static Campaign? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement o = doc.RootElement;
            if (o.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            static string? Str(JsonElement o, string name) =>
                o.TryGetProperty(name, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

            // PrivateAICampaign.swift:28-35: schema 1, plain bounded words,
            // a symbol a hero can draw, an https action on near.ai, and an
            // image, if named, that is an https raster.
            if (!(o.TryGetProperty("schema_version", out JsonElement schema) && schema.ValueKind == JsonValueKind.Number
                    && schema.TryGetInt32(out int version) && version == 1)
                || Str(o, "id") is not { } id || !Plain(id, 100)
                || Str(o, "title") is not { } title || !Plain(title, 100)
                || Str(o, "body") is not { } body || !Plain(body, 300)
                || Str(o, "action_title") is not { } action || !Plain(action, 40)
                || Str(o, "symbol") is not { } symbol || Array.IndexOf(Symbols, symbol) < 0
                || Str(o, "action_url") is not { } url || !SecureUrl(url, out Uri? actionUri)
                || !string.Equals(actionUri!.Host, "near.ai", StringComparison.OrdinalIgnoreCase))
            {
                return null;
            }

            string? image = Str(o, "image_url");
            if (image is not null && !RasterUrl(image))
            {
                return null;
            }

            return new Campaign(id, title, body, action, url, symbol, image);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>PrivateAICampaign.swift:31: the symbols a campaign may name.</summary>
    public static readonly string[] Symbols = { "cpu", "sparkles", "bolt", "globe" };

    /// <summary>PrivateAICampaign.swift:44-48: non-blank, bounded, no control characters, no angle brackets.</summary>
    private static bool Plain(string value, int limit) =>
        value.Trim().Length > 0
        && new System.Globalization.StringInfo(value).LengthInTextElements <= limit
        && !value.Any(char.IsControl)
        && !value.Contains('<') && !value.Contains('>');

    /// <summary>PrivateAICampaign.swift:50-55: https, a host, no user, password or fragment, port 443 or none.</summary>
    private static bool SecureUrl(string text, out Uri? uri)
    {
        if (!Uri.TryCreate(text, UriKind.Absolute, out uri))
        {
            return false;
        }

        return uri.Scheme == Uri.UriSchemeHttps && uri.Host.Length > 0 && uri.UserInfo.Length == 0
            && uri.Port == 443 && uri.Fragment.Length == 0 && !text.Contains('#');
    }

    /// <summary>PrivateAICampaign.swift:57-59: a secure png or jpeg.</summary>
    private static bool RasterUrl(string text)
    {
        if (!SecureUrl(text, out Uri? uri))
        {
            return false;
        }

        string extension = System.IO.Path.GetExtension(uri!.AbsolutePath).ToLowerInvariant();
        return extension is ".png" or ".jpg" or ".jpeg" && !uri.AbsolutePath.Contains("..", StringComparison.Ordinal);
    }
}
