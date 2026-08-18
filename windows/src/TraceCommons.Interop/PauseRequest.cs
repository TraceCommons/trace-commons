using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>The three pause choices shared by every contributor shell.</summary>
public enum PauseDuration
{
    OneHour,
    TomorrowMorning,
    UntilResumed,
}

/// <summary>
/// Builds the daemon's <c>pause</c> parameters from a user-facing duration.
/// </summary>
/// <remarks>
/// Timed pauses are instants, not app timers. The daemon persists the instant
/// and resumes itself even if the window is closed; an app-side timer would
/// silently turn "one hour" into "until I reopen the app".
/// </remarks>
public static class PauseRequest
{
    public static string Serialize(
        PauseDuration duration,
        DateTimeOffset now,
        TimeZoneInfo? localZone = null)
    {
        DateTimeOffset? until = duration switch
        {
            PauseDuration.OneHour => now.AddHours(1),
            PauseDuration.TomorrowMorning => TomorrowAtNine(now, localZone ?? TimeZoneInfo.Local),
            PauseDuration.UntilResumed => null,
            _ => throw new ArgumentOutOfRangeException(nameof(duration)),
        };

        return until is null
            ? "{}"
            : JsonSerializer.Serialize(
                new Dictionary<string, string>
                {
                    ["until"] = until.Value.ToUniversalTime().ToString("O"),
                });
    }

    /// <summary>9:00 in the machine's local zone on the following date.</summary>
    private static DateTimeOffset TomorrowAtNine(DateTimeOffset now, TimeZoneInfo zone)
    {
        DateTimeOffset localNow = TimeZoneInfo.ConvertTime(now, zone);
        DateTime localNine = localNow.Date.AddDays(1).AddHours(9);
        TimeSpan offset = zone.GetUtcOffset(localNine);
        return new DateTimeOffset(localNine, offset);
    }
}
