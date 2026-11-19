using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop.Shell;

/// <summary>One of Home's stat tiles (<c>HomeStatTile</c>, HomeViews.swift:1841-1876): its label, figure and the line under it.</summary>
public sealed record HomeTile(string Label, string Value, string? Detail = null);

/// <summary>Where a tile or hero opens (HomeViews.swift:196-200, :500-504).</summary>
public enum HomeLink
{
    Traces,
    History,
    PrivateAI,
    MissionDrafts,
    Insights,
    Campaign,
    MissionCatalogue,
    Settings,
}

/// <summary>A hero card as Home draws it.</summary>
public sealed record HomeHero(HomeGround Ground, string Title, string? Subtitle, string? Action, string? Symbol, bool Closes, HomeLink Link);

/// <summary>A Home line with one action, as the no-credit reminder draws it (NoCreditNotice.swift).</summary>
public sealed record HomeNotice(string Line, string Action);

/// <summary>A Home card that is a way into a screen (<c>hostedCard</c>, HomeViews.swift:313-321): its title and the core's subtitle.</summary>
public sealed record HomeCard(string Title, string? Subtitle, HomeLink Link);

/// <summary>Which hero ground, mirrored from <see cref="HeroGround"/> so the record stays a plain value.</summary>
public enum HomeGround
{
    Campaign,
    Missions,
    Insights,
}

/// <summary>
/// Everything Home's overview draws, from the reads it was given
/// (<c>HomeOverview</c>, macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift:150-554).
/// Pure: the view only lays this out.
/// </summary>
/// <remarks>
/// What is drawn: the Traces tiles (Waiting, Contributed, and Not
/// contributed while anything waits), the Account tiles (Credit earned with
/// the account total under it, Credit pending, the NEAR AI balance), then the
/// three hero cards on the Mac's solid navy grounds (the compiled campaign,
/// Missions while missions are on, Insights once its heading has loaded).
/// Also the no-credit line under the Traces heading and the mission
/// catalogue's card, ported with the Mac's conditions (cards new to Windows,
/// approved as a class 2026-11-19). The walkthrough card is left out
/// (W11-D2 (a)).
/// </remarks>
public sealed record HomeOverviewModel(
    string TracesHeading,
    string AccountHeading,
    IReadOnlyList<HomeTile> TraceTiles,
    IReadOnlyList<HomeTile> AccountTiles,
    IReadOnlyList<HomeHero> Heroes,
    IReadOnlyList<GlanceBar> InsightsBars,
    string? InsightsLine,
    HomeNotice? NoCredit = null,
    HomeCard? Catalogue = null)
{
    /// <summary>The figure rules, the core's (<c>tc_count_figure</c>, <c>tc_credit_figure</c>), handed in so tests need no library.</summary>
    public sealed record Figures(Func<ulong, string?> Count, Func<double, string?> Credit, Func<long?, byte?, string> Amount);

    /// <summary>The reads Home draws from; any of them null when it failed or has not come.</summary>
    public sealed record Reads(
        DaemonStatus? Status,
        HistoryRollup? Rollup,
        CommonsCredit? Credit,
        NearAiBalance? Balance,
        IReadOnlyList<QueueEntry>? Pending,
        JsonElement? Glance,
        Campaign? Campaign,
        bool CampaignShown,
        bool MissionsOn,
        int? MissionDrafts,
        string? InsightsHeading,
        string? InsightsIntro,
        bool CatalogueRead = false);

    /// <summary>The data use that earns credit (<c>SentTracesTrainingState.scope</c>).</summary>
    public const string ModelTrainingScope = "model_training";

    /// <summary>States that have left, or are leaving, this machine (HomeViews.swift:1889).</summary>
    private static readonly HashSet<string> Gone = new(StringComparer.Ordinal) { "uploading", "uploaded" };

    public static HomeOverviewModel From(Reads reads, Func<string, string> top, Func<string, string> home, string balanceTitle, Figures figures)
    {
        var traceTiles = new List<HomeTile>
        {
            new(top("waiting"), HomeFacts.Count(reads.Status?.DecisionsOwed, figures.Count)),
            new(top("contributed"), HomeFacts.Count(reads.Rollup?.AllTime.Contributed, figures.Count)),
        };

        // Not contributed: drawn only while something waits, with the core's
        // heading (HomeViews.swift:266-271).
        int waiting = reads.Pending?.Count(e => !Gone.Contains(e.State ?? string.Empty)) ?? 0;
        if (waiting > 0 && HomeFacts.CardHeading(home("uncontributed")) is string heading)
        {
            traceTiles.Add(new HomeTile(heading, HomeFacts.Count(waiting, figures.Count)));
        }

        (string received, string pending) = HomeFacts.CreditTiles(
            reads.Rollup?.CreditFinal, reads.Rollup?.CreditPending, reads.Credit, figures.Credit);
        string creditReceived = top("credit_received") is { Length: > 0 } r ? r : top("credit_pending");
        string creditPending = top("credit_pending_title") is { Length: > 0 } p ? p : top("pending");
        var accountTiles = new List<HomeTile>
        {
            new(creditReceived, received, HomeFacts.CreditAccountTotal(reads.Credit, home("credit_account_total"), figures.Credit)),
            new(creditPending, pending),
        };
        if (balanceTitle.Length > 0)
        {
            string amount = reads.Balance is NearAiBalance b ? figures.Amount(b.RemainingNanos, b.Scale) : string.Empty;
            accountTiles.Add(new HomeTile(balanceTitle, string.IsNullOrEmpty(amount) ? HomeFacts.Dash : amount));
        }

        var heroes = new List<HomeHero>();
        if (reads.Campaign is Campaign campaign && reads.CampaignShown)
        {
            heroes.Add(new HomeHero(HomeGround.Campaign, campaign.Title, campaign.Body, campaign.ActionTitle, campaign.Symbol, Closes: true, HomeLink.Campaign));
        }

        if (reads.MissionsOn && HomeFacts.CardHeading(top("missions")) is string missions)
        {
            heroes.Add(new HomeHero(HomeGround.Missions, missions, NonEmpty(home("missions_subtitle")), NonEmpty(home("missions_action")), HeroCard.MissionsSymbol, Closes: false, HomeLink.MissionDrafts));
        }

        IReadOnlyList<GlanceBar> bars = HomeFacts.Bars(reads.Glance);
        string? insightsLine = null;
        if (HomeFacts.CardHeading(reads.InsightsHeading) is string insights)
        {
            // The Mac's line under this heading says "read on this Mac"; this
            // shell keeps Insights' own intro, which names no machine (the
            // Linux port's choice, #2187), until the core has a "this
            // computer" variant.
            heroes.Add(new HomeHero(HomeGround.Insights, insights, NonEmpty(reads.InsightsIntro), NonEmpty(home("insights_action")), bars.Count == 0 ? HeroCard.InsightsSymbol : null, Closes: false, HomeLink.Insights));
            insightsLine = bars.Count == 0
                ? NonEmpty(home("insights_teaser"))
                : NonEmpty(home("insights_today"))?.Replace("{total}", Compact(bars.Aggregate(0UL, (sum, b) => sum + b.Tokens)), StringComparison.Ordinal);
        }

        // Under the Traces heading while Train coding models is off, for a
        // signed-in account (NoCreditNotice.showsOnHome, HomeViews.swift:448-453).
        HomeNotice? noCredit = null;
        if (reads.Status is { LoggedIn: true } status
            && !status.ConsentScopes.Contains(ModelTrainingScope)
            && NonEmpty(home("no_credit_body")) is string line
            && NonEmpty(home("no_credit_open_settings")) is string openSettings)
        {
            noCredit = new HomeNotice(line, openSettings);
        }

        // After the heroes, the catalogue's way in, while missions are on and
        // the catalogue was read (HomeViews.swift:274-277).
        HomeCard? catalogue = reads.MissionsOn && reads.CatalogueRead && HomeFacts.CardHeading(home("mission_catalogue")) is string title
            ? new HomeCard(title, NonEmpty(home("catalogue_subtitle")), HomeLink.MissionCatalogue)
            : null;

        return new HomeOverviewModel(
            home("section_traces"),
            home("section_account"),
            traceTiles,
            accountTiles,
            heroes,
            bars,
            insightsLine,
            noCredit,
            catalogue);
    }

    /// <summary>
    /// Whether <paramref name="other"/> draws the same Home: every heading,
    /// tile, hero and bar equal. A record's own equality compares its lists
    /// by reference, so two reads of an unchanged daemon would look new and
    /// rebuild Home (losing focus and restarting the glyphs; checklist O2).
    /// </summary>
    public bool Same(HomeOverviewModel? other) =>
        other is not null
        && TracesHeading == other.TracesHeading
        && AccountHeading == other.AccountHeading
        && InsightsLine == other.InsightsLine
        && NoCredit == other.NoCredit
        && Catalogue == other.Catalogue
        && TraceTiles.SequenceEqual(other.TraceTiles)
        && AccountTiles.SequenceEqual(other.AccountTiles)
        && Heroes.SequenceEqual(other.Heroes)
        && InsightsBars.SequenceEqual(other.InsightsBars);

    private static string? NonEmpty(string? word) => string.IsNullOrEmpty(word) ? null : word;

    /// <summary>
    /// Today's total as the Mac's compact figure (<c>InsightsOverviewWords.figure</c>,
    /// <c>.notation(.compactName)</c> in English): whole below a thousand, one
    /// decimal under ten of a unit, rounded half to even, then K, M, B, T.
    /// </summary>
    public static string Compact(ulong value)
    {
        if (value < 1000)
        {
            return value.ToString(System.Globalization.CultureInfo.InvariantCulture);
        }

        (ulong Size, string Suffix)[] units = { (1_000, "K"), (1_000_000, "M"), (1_000_000_000, "B"), (1_000_000_000_000, "T") };
        int index = Array.FindLastIndex(units, u => value >= u.Size);
        while (true)
        {
            (ulong size, string suffix) = units[index];
            bool last = index + 1 == units.Length;
            if ((decimal)value < 10m * size)
            {
                decimal tenths = Math.Round(value * 10m / size, MidpointRounding.ToEven);
                if (tenths >= 10_000 && !last)
                {
                    index++;
                    continue;
                }

                return tenths % 10 == 0
                    ? (tenths / 10).ToString("0", System.Globalization.CultureInfo.InvariantCulture) + suffix
                    : Math.Floor(tenths / 10).ToString("0", System.Globalization.CultureInfo.InvariantCulture) + "." + (tenths % 10).ToString("0", System.Globalization.CultureInfo.InvariantCulture) + suffix;
            }

            decimal whole = Math.Round((decimal)value / size, MidpointRounding.ToEven);
            if (whole >= 1000 && !last)
            {
                index++;
                continue;
            }

            return whole.ToString("#,0", System.Globalization.CultureInfo.InvariantCulture) + suffix;
        }
    }
}
