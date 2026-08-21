using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The condition these cover: a contributor approved traces, the day's byte
/// budget was spent, and nothing anywhere said so. The daemon did set a
/// daily-cap-reached health label, but that label is last in its precedence
/// order and the single health slot was held by queue-full, so from outside
/// a working app was indistinguishable from a broken one.
/// </summary>
public class DailyBudgetTests
{
    /// The state measured on the machine this was diagnosed from.
    private const string RealStatus = """
        {"schema_version":"trace_commons.daemon.v1_1","logged_in":true,
         "paused":false,"queue_depth":500,
         "health":{"last_error_label":"queue-full","since":"2026-08-21T09:17:35Z"},
         "daily_budget":{"bytes_today":204659969,"max_bytes_per_day":209715200,
          "bytes_remaining":5055231,"uploads_today":12,"max_uploads_per_day":50,
          "uploads_remaining":38,"resets_at":"2026-08-22T00:00:00Z",
          "blocked":true,"blocked_entries":14,"blocked_bytes":137283584}}
        """;

    [Fact]
    public void TheBudgetIsReadFromStatusAlongsideAnUnrelatedHealthLabel()
    {
        DaemonStatus? status = JsonSerializer.Deserialize<DaemonStatus>(RealStatus);

        Assert.NotNull(status);
        // The health slot is telling the truth about something else entirely.
        Assert.Equal("queue-full", status!.Health?.LastErrorLabel);
        // And the budget is still legible, which is the whole point.
        Assert.True(status.BudgetIsBlocking);
        Assert.Equal(14, status.DailyBudget!.BlockedEntries);
        Assert.Equal(204_659_969L, status.DailyBudget.BytesToday);
        Assert.Equal(209_715_200L, status.DailyBudget.MaxBytesPerDay);
        Assert.Equal(5_055_231L, status.DailyBudget.BytesRemaining);
        Assert.Equal(12, status.DailyBudget.UploadsToday);
        Assert.Equal(38, status.DailyBudget.UploadsRemaining);
        Assert.Equal(
            new DateTimeOffset(2026, 8, 22, 0, 0, 0, TimeSpan.Zero),
            status.DailyBudget.ResetsAtUtc);
    }

    [Fact]
    public void ADaemonThatPredatesTheFieldReportsNothingBlocking()
    {
        DaemonStatus? status = JsonSerializer.Deserialize<DaemonStatus>(
            """{"logged_in":true,"health":{"last_error_label":null,"since":null}}""");

        Assert.NotNull(status);
        Assert.Null(status!.DailyBudget);
        Assert.False(status.BudgetIsBlocking);
        Assert.Null(HealthCopy.ForBudget(status.DailyBudget));
    }

    [Fact]
    public void TheBannerStatesHowManyAreWaitingAndWhenTheLimitResets()
    {
        DaemonStatus status = JsonSerializer.Deserialize<DaemonStatus>(RealStatus)!;
        HealthCopy? copy = HealthCopy.ForBudget(status.DailyBudget);

        Assert.NotNull(copy);
        Assert.Equal("Today's upload limit is used up.", copy!.Title);
        Assert.StartsWith("14 approved traces are waiting.", copy.Detail, StringComparison.Ordinal);
        Assert.Contains("Nothing has been lost", copy.Detail, StringComparison.Ordinal);
        Assert.Contains(
            $"resets at {status.DailyBudget!.ResetsAtUtc!.Value.ToLocalTime():t}",
            copy.Detail,
            StringComparison.Ordinal);
        // Nothing to press: the caps are not settable from this window.
        Assert.Null(copy.ActionLabel);
    }

    [Fact]
    public void OneWaitingTraceIsNotDescribedInThePlural()
    {
        HealthCopy? copy = HealthCopy.ForBudget(
            new DailyBudget { Blocked = true, BlockedEntries = 1 });

        Assert.NotNull(copy);
        Assert.StartsWith("1 approved trace is waiting.", copy!.Detail, StringComparison.Ordinal);
    }

    [Fact]
    public void WithNoResetTimeTheBannerStopsRatherThanGuessing()
    {
        // Never "tomorrow". The daemon rolls its counters at UTC midnight,
        // which is not tomorrow for most of the world, and the old copy said
        // exactly that.
        HealthCopy? copy = HealthCopy.ForBudget(
            new DailyBudget { Blocked = true, BlockedEntries = 3, ResetsAt = null });

        Assert.NotNull(copy);
        Assert.Equal(
            "3 approved traces are waiting. Nothing has been lost -- they go out when the "
            + "limit resets.",
            copy!.Detail);
        Assert.DoesNotContain("tomorrow", copy.Detail, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void AnUnparseableResetTimeIsTreatedAsNoResetTime()
    {
        HealthCopy? copy = HealthCopy.ForBudget(
            new DailyBudget { Blocked = true, BlockedEntries = 2, ResetsAt = "not a timestamp" });

        Assert.NotNull(copy);
        Assert.DoesNotContain("resets at", copy!.Detail, StringComparison.Ordinal);
    }

    [Fact]
    public void ASpentBudgetHoldingNothingDrawsNoBanner()
    {
        // The budget really is gone, but there is nobody to tell about it.
        HealthCopy? copy = HealthCopy.ForBudget(
            new DailyBudget
            {
                Blocked = false,
                BlockedEntries = 0,
                BytesToday = 209_715_200,
                BytesRemaining = 0,
            });

        Assert.Null(copy);
    }

    [Fact]
    public void TheBannerNeverReadsAsAFailure()
    {
        HealthCopy copy = HealthCopy.ForBudget(
            new DailyBudget { Blocked = true, BlockedEntries = 14 })!;
        HealthCopy fallback = HealthCopy.ForLabel("daily-cap-reached")!;

        foreach (string text in new[] { copy.Title, copy.Detail, fallback.Title, fallback.Detail })
        {
            foreach (string word in new[] { "error", "failed", "problem", "wrong" })
            {
                Assert.DoesNotContain(word, text, StringComparison.OrdinalIgnoreCase);
            }
        }
    }

    [Fact]
    public void TheFallbackLabelLinePromisesNoParticularTime()
    {
        // Used only when a daemon reports the label without the budget
        // object, and it must not invent what the object would have said.
        HealthCopy fallback = HealthCopy.ForLabel("daily-cap-reached")!;

        Assert.DoesNotContain("tomorrow", fallback.Detail, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("resets", fallback.Detail, StringComparison.Ordinal);
    }

    [Fact]
    public void TheBudgetCarriesNoIdentifierOfAnyKind()
    {
        // Counts and one timestamp. No entry id, no hash, no path can reach
        // this surface, and the banner it feeds must not either.
        HealthCopy copy = HealthCopy.ForBudget(
            JsonSerializer.Deserialize<DaemonStatus>(RealStatus)!.DailyBudget)!;

        Assert.DoesNotContain("sha256", copy.Detail, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("/", copy.Detail, StringComparison.Ordinal);
        Assert.DoesNotContain("\\", copy.Detail, StringComparison.Ordinal);
    }
}
