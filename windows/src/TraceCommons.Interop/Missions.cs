using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>A mission's credit range (<c>DaemonData.CreditRange</c>).</summary>
public sealed record MissionCreditRange(long Min, long Max, string Unit);

/// <summary>One mission in the commons' catalogue (<c>DaemonData.Mission</c>).</summary>
public sealed record Mission(string Id, string Title, string? Summary, MissionCreditRange? CreditRange);

/// <summary>
/// The catalogue the Missions page reads: the PROVISIONAL shape the Mac's
/// <c>MissionsPage</c> draws (<c>DaemonData.MissionCatalogue</c>, read through
/// <c>missionCatalogue()</c>). The Mac's live client answers it
/// <c>notAvailableYet</c> without asking the daemon
/// (LiveDaemonClient.swift:439-441), so only sample data reaches the page;
/// the daemon's own <c>mission_catalogue</c> reply is a different shape and is
/// never decoded into this one.
/// </summary>
public sealed record MissionCatalogue(string? Explanation, IReadOnlyList<Mission> Missions)
{
    /// <summary>The catalogue, or null when it will not decode as the Mac decodes it.</summary>
    public static MissionCatalogue? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("missions", out JsonElement missions)
                || missions.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            string? explanation = null;
            if (root.TryGetProperty("posture", out JsonElement posture) && posture.ValueKind == JsonValueKind.Object)
            {
                explanation = SessionDetail.Str(posture, "explanation");
            }

            var list = new List<Mission>();
            foreach (JsonElement item in missions.EnumerateArray())
            {
                if (SessionDetail.Str(item, "id") is not { } id || SessionDetail.Str(item, "title") is not { } title)
                {
                    return null;
                }

                MissionCreditRange? range = null;
                if (item.TryGetProperty("credit_range", out JsonElement credit) && credit.ValueKind == JsonValueKind.Object)
                {
                    if (!credit.TryGetProperty("min", out JsonElement min) || !min.TryGetInt64(out long low)
                        || !credit.TryGetProperty("max", out JsonElement max) || !max.TryGetInt64(out long high)
                        || SessionDetail.Str(credit, "unit") is not { } unit)
                    {
                        return null;
                    }

                    range = new MissionCreditRange(low, high, unit);
                }

                list.Add(new Mission(id, title, SessionDetail.Str(item, "summary"), range));
            }

            return new MissionCatalogue(explanation, list);
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>
/// How the Missions page's read of the catalogue ended, as the Mac's
/// <c>HomeStore.read</c> settles it (HomeStore.swift:205-220).
/// </summary>
public enum MissionRead
{
    /// <summary>
    /// The daemon has no catalogue to give: the Mac's live client answers
    /// <c>notAvailableYet</c>, which is <c>isNotServed</c>
    /// (LiveDaemonClient.swift:439-440, DaemonDataModels.swift:153-158), so
    /// the store keeps no failure and the page draws a dash.
    /// </summary>
    NotServed,

    /// <summary>The catalogue's JSON came back (sample launches only, today).</summary>
    Served,

    /// <summary>The core was not there (<c>coreDown</c>: <c>DaemonDataError.unreachable</c>).</summary>
    Unreachable,
}

/// <summary>
/// What the Missions page draws: the catalogue, or the core's failure line,
/// or the dash named with the core's word for unknown. At most one is set.
/// </summary>
public sealed record MissionPage(MissionCatalogue? Catalogue, string? FailureLine, string? UnknownName)
{
    /// <summary>
    /// Settles a read as the Mac's <c>MissionsPage</c> draws it
    /// (MissionsViews.swift:33-64): a failure is the core's line; a catalogue
    /// that came back but will not decode is a failed request; not served is
    /// a dash, never a failure and never a 0 that reads as "none".
    /// </summary>
    public static MissionPage Settle(MissionRead read, string? json, Func<string, string> word)
    {
        if (read == MissionRead.Served && MissionCatalogue.Parse(json) is { } catalogue)
        {
            return new MissionPage(catalogue, null, null);
        }

        if (read == MissionRead.NotServed)
        {
            return new MissionPage(null, null, word("unknown"));
        }

        return new MissionPage(null, MissionFormat.FailureLine(read == MissionRead.Unreachable, word), null);
    }
}

/// <summary>
/// Formatting for Missions, under #1174's rules: the Mac's <c>MissionFormat</c>
/// (MissionsViews.swift:105-147). Every word is the core's monitor table.
/// </summary>
public static class MissionFormat
{
    /// <summary>The one unit the data contract names for a mission's range.</summary>
    public const string MissionCreditUnit = "points";

    /// <summary>The figure for something unread or unknown, never 0.</summary>
    public const string Dash = "—";

    /// <summary>What mission credit waits on, in the commons' own words (M3), or null.</summary>
    public static string? Condition(MissionCatalogue catalogue) =>
        string.IsNullOrEmpty(catalogue.Explanation) ? null : catalogue.Explanation;

    /// <summary>Whether any mission shows a projected figure, so the page needs the core's note.</summary>
    public static bool ShowsProjected(MissionCatalogue catalogue, Func<string, string> word) =>
        catalogue.Missions.Any(mission => Credit(mission, catalogue, word) != Dash);

    /// <summary>
    /// A mission's credit range, projected: a dash when it has no range, a
    /// unit this build cannot word, no condition stated, or no core words.
    /// </summary>
    public static string Credit(Mission mission, MissionCatalogue catalogue, Func<string, string> word)
    {
        if (Condition(catalogue) is null || mission.CreditRange is not { } range || range.Unit != MissionCreditUnit)
        {
            return Dash;
        }

        string min = range.Min.ToString(CultureInfo.InvariantCulture);
        if (range.Min == range.Max)
        {
            string one = word("mission_credit_points_one");
            return one.Length == 0 ? Dash : one.Replace("{min}", min, StringComparison.Ordinal);
        }

        string many = word("mission_credit_points");
        return many.Length == 0
            ? Dash
            : many.Replace("{min}", min, StringComparison.Ordinal)
                .Replace("{max}", range.Max.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);
    }

    /// <summary>Home's count: the catalogue's size, or a dash when it was not read.</summary>
    public static string Count(MissionCatalogue? catalogue) =>
        catalogue is null ? Dash : catalogue.Missions.Count.ToString(CultureInfo.InvariantCulture);

    /// <summary>
    /// The page's line when the catalogue could not be read: the core's
    /// unreachable line for a daemon that is not there, its request-failed
    /// line otherwise (<c>MonitorScreensCopy.line(for:)</c>, MonitorScreensCopy.swift:371-374).
    /// </summary>
    public static string FailureLine(bool unreachable, Func<string, string> word) =>
        word(unreachable ? "core_unreachable" : "request_failed");
}
