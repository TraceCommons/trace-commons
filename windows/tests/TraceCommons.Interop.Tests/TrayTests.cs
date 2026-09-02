using System;
using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public class PauseRequestTests
{
    private static readonly DateTimeOffset Now =
        new(2026, 8, 18, 10, 30, 0, TimeSpan.Zero);

    [Fact]
    public void OneHourIsAnAbsoluteDaemonDeadline()
    {
        using JsonDocument json = JsonDocument.Parse(
            PauseRequest.Serialize(PauseDuration.OneHour, Now));

        Assert.Equal(
            Now.AddHours(1),
            DateTimeOffset.Parse(json.RootElement.GetProperty("until").GetString()!));
    }

    [Fact]
    public void TomorrowMorningMeansNineInTheRequestedLocalZone()
    {
        TimeZoneInfo zone = TimeZoneInfo.CreateCustomTimeZone(
            "preview-test-zone",
            TimeSpan.FromHours(2),
            "preview-test-zone",
            "preview-test-zone");

        using JsonDocument json = JsonDocument.Parse(
            PauseRequest.Serialize(PauseDuration.TomorrowMorning, Now, zone));

        Assert.Equal(
            new DateTimeOffset(2026, 8, 19, 9, 0, 0, TimeSpan.FromHours(2)),
            DateTimeOffset.Parse(json.RootElement.GetProperty("until").GetString()!));
    }

    [Fact]
    public void UntilResumedHasNoInventedDeadline()
    {
        using JsonDocument json = JsonDocument.Parse(
            PauseRequest.Serialize(PauseDuration.UntilResumed, Now));

        Assert.False(json.RootElement.TryGetProperty("until", out _));
    }
}

/// <summary>
/// The tray's decisions, the digest's cadence, its wording, and the mark it
/// draws -- all on a machine that cannot render a Windows tray icon.
/// </summary>
/// <remarks>
/// This is the same bargain the rest of this assembly makes. What can only be
/// confirmed on Windows is that Shell_NotifyIcon accepts the struct and that
/// the shell draws the bitmap; what can be confirmed here is everything that
/// would still be wrong if it did.
/// </remarks>
public class DigestCadenceTests
{
    [Fact]
    public void NothingWaitingMeansNoNotificationAtAll()
    {
        var cadence = new DigestCadence();
        Assert.False(cadence.TryClaim(0, 0, Now(0)));
        Assert.Null(cadence.LastClaimedAt);
    }

    [Fact]
    public void AnEmptyClaimDoesNotConsumeTheWindow()
    {
        // The promise is "none at all if there's nothing waiting", not "the
        // empty one counts as your notification for the next four hours".
        var cadence = new DigestCadence();
        Assert.False(cadence.TryClaim(0, 0, Now(0)));
        Assert.True(cadence.TryClaim(3, 0, Now(1)));
    }

    [Fact]
    public void TheFirstDigestWithPendingWorkIsAllowed()
    {
        var cadence = new DigestCadence();
        Assert.True(cadence.TryClaim(1, 0, Now(0)));
    }

    [Fact]
    public void AtMostOneNotificationEveryFourHours()
    {
        var cadence = new DigestCadence();
        Assert.True(cadence.TryClaim(3, 0, Now(0)));

        // Every minute for just under four hours: not one of them may pass.
        for (int minutes = 1; minutes < 240; minutes++)
        {
            Assert.False(cadence.TryClaim(3, 0, Now(minutes)));
        }

        Assert.True(cadence.TryClaim(3, 0, Now(240)));
    }

    [Fact]
    public void TheIntervalIsExactlyTheFourHoursTheOnboardingScreenPromises()
    {
        Assert.Equal(TimeSpan.FromHours(4), DigestCadence.MinimumInterval);
    }

    [Fact]
    public void AClockThatWentBackwardsCanOnlySuppress()
    {
        var cadence = new DigestCadence();
        Assert.True(cadence.TryClaim(2, 0, Now(600)));

        // A manual clock change or a bad RTC after a resume. The wrong
        // answer here is a burst of notifications.
        Assert.False(cadence.TryClaim(2, 0, Now(0)));
    }

    [Fact]
    public void ClaimingStampsSoAForgottenRecordCannotHappen()
    {
        var cadence = new DigestCadence();
        cadence.TryClaim(1, 0, Now(0));
        Assert.Equal(Now(0), cadence.LastClaimedAt);
    }

    private static DateTimeOffset Now(int minutes) =>
        new DateTimeOffset(2026, 8, 18, 9, 0, 0, TimeSpan.Zero).AddMinutes(minutes);
}

public class DigestTextTests
{
    [Fact]
    public void TheDigestReadsAsTheSharedSpecWritesIt()
    {
        // Verbatim from docs/superpowers/specs/
        // 2026-08-08-contributor-shell-shared-design.md, "### The digest",
        // and identical to the Linux shell's notify::digest_body test.
        Assert.Equal(
            "3 sessions ready from trace-commons-server and dotfiles.\n"
            + "Nothing is sent until you review them.",
            DigestText.Body(3, new[] { "trace-commons-server", "dotfiles" }));
    }

    [Fact]
    public void OneSessionIsSingular()
    {
        Assert.StartsWith("1 session ready from a.", DigestText.Body(1, new[] { "a" }), StringComparison.Ordinal);
    }

    [Fact]
    public void ThreeOrMoreProjectsUseTheSpecsListForm()
    {
        Assert.StartsWith(
            "5 sessions ready from a, b and c.",
            DigestText.Body(5, new[] { "a", "b", "c" }),
            StringComparison.Ordinal);
    }

    [Fact]
    public void NoProjectsLeavesNoDanglingFrom()
    {
        Assert.Equal(
            "2 sessions ready.\nNothing is sent until you review them.",
            DigestText.Body(2, Array.Empty<string>()));
    }

    [Fact]
    public void BlankAndDuplicateLabelsAreDropped()
    {
        Assert.StartsWith(
            "2 sessions ready from a and b.",
            DigestText.Body(2, new[] { "a", "", "a", "  ", "b" }),
            StringComparison.Ordinal);
    }

    [Fact]
    public void TheReassuranceIsTheSameSentenceEveryShellUses()
    {
        Assert.Equal("Nothing is sent until you review them.", DigestText.NothingSent);
    }
}

public class TrayModelTests
{
    [Fact]
    public void AttentionOutranksEverything()
    {
        Assert.Equal(
            TrayIconState.Attention,
            TrayModel.Compute(3, isPaused: true, isHealthy: false).State);
    }

    [Fact]
    public void UnhealthyOutranksPaused()
    {
        Assert.Equal(
            TrayIconState.Unhealthy,
            TrayModel.Compute(0, isPaused: true, isHealthy: false).State);
    }

    [Fact]
    public void PausedOutranksIdle()
    {
        Assert.Equal(
            TrayIconState.Paused,
            TrayModel.Compute(0, isPaused: true, isHealthy: true).State);
    }

    [Fact]
    public void NothingOwedAndAllWellIsIdle()
    {
        TrayModel model = TrayModel.Compute(0, isPaused: false, isHealthy: true);
        Assert.Equal(TrayIconState.Idle, model.State);
        Assert.Equal("Trace Commons — Watching. Nothing waiting.", model.Tooltip);
    }

    [Fact]
    public void TheCountIsDecisionsOwedAndIsSaidInWords()
    {
        TrayModel model = TrayModel.Compute(3, isPaused: false, isHealthy: true);
        Assert.Equal(3, model.DecisionsOwed);
        Assert.Equal("Trace Commons — 3 sessions waiting for review.", model.Tooltip);
    }

    [Fact]
    public void OneDecisionIsSingular()
    {
        Assert.Equal(
            "Trace Commons — 1 session waiting for review.",
            TrayModel.Compute(1, isPaused: false, isHealthy: true).Tooltip);
    }

    [Fact]
    public void AttentionStillSaysWhenItIsAlsoPaused()
    {
        // The icon can only show one state. The tooltip must not let a
        // contributor approve three sessions believing the watcher is
        // running.
        Assert.Equal(
            "Trace Commons — 3 sessions waiting for review. Paused.",
            TrayModel.Compute(3, isPaused: true, isHealthy: true).Tooltip);
    }

    [Fact]
    public void AttentionStillSaysWhenItIsAlsoUnhealthy()
    {
        Assert.Equal(
            "Trace Commons — 2 sessions waiting for review. Needs attention.",
            TrayModel.Compute(2, isPaused: false, isHealthy: false).Tooltip);
    }

    [Fact]
    public void ANegativeCountCannotProduceAnAttentionState()
    {
        // Defensive against a daemon frame this client failed to parse:
        // reading "-1 sessions waiting" as attention would be an interruption
        // caused by a bug.
        TrayModel model = TrayModel.Compute(-4, isPaused: false, isHealthy: true);
        Assert.Equal(TrayIconState.Idle, model.State);
        Assert.Equal(0, model.DecisionsOwed);
    }

    [Fact]
    public void TheTooltipFitsWhatSzTipCanHold()
    {
        string trimmed = TrayModel.Truncate(new string('x', 400));
        Assert.Equal(TrayModel.MaxTooltipLength, trimmed.Length);
        Assert.EndsWith("…", trimmed, StringComparison.Ordinal);
    }

    [Fact]
    public void ATooltipThatAlreadyFitsIsUntouched()
    {
        Assert.Equal("short", TrayModel.Truncate("short"));
    }

    [Fact]
    public void NoTooltipCarriesAPathOrAnIdentity()
    {
        // A structural guard, not a proof: Compute takes only an int and two
        // bools, so there is nothing available to it that could be a path.
        // The test exists so that a future signature change adding a string
        // has to come past it.
        foreach (int owed in new[] { 0, 1, 7 })
        {
            foreach (bool paused in new[] { true, false })
            {
                foreach (bool healthy in new[] { true, false })
                {
                    string tip = TrayModel.Compute(owed, paused, healthy).Tooltip;
                    Assert.DoesNotContain('\\', tip);
                    Assert.DoesNotContain('/', tip);
                    Assert.DoesNotContain("sha256", tip, StringComparison.OrdinalIgnoreCase);
                }
            }
        }
    }
}

public class TrayMenuModelTests
{
    [Fact]
    public void WaitingRowsAreGroupedAndSortedWithoutPaths()
    {
        var status = new DaemonStatus { QueueDepth = 3 };
        var pending = new[]
        {
            new QueueEntry { ProjectLabel = "zeta", SizeBytes = 1024 },
            new QueueEntry { ProjectLabel = "alpha", SizeBytes = 512 },
            new QueueEntry { ProjectLabel = "zeta", SizeBytes = 2048 },
        };

        TrayMenuModel menu = TrayMenuModel.Compute(
            status,
            pending,
            new HistoryRollup(),
            Array.Empty<ProjectSetting>());

        Assert.Equal(2, menu.Waiting.Count);
        Assert.Equal("alpha — 1 · 512 B", menu.Waiting[0].Text);
        Assert.Equal("zeta — 2 · 3 KB", menu.Waiting[1].Text);
    }

    [Fact]
    public void WeekAndArmedProjectsComeFromDaemonModels()
    {
        var status = new DaemonStatus { Paused = true, QueueDepth = 1 };
        var rollup = new HistoryRollup
        {
            Week = new HistoryCounts { Submitted = 4, Quarantined = 2 },
        };
        var projects = new[]
        {
            new ProjectSetting { ProjectLabel = "ask", Mode = "ask" },
            new ProjectSetting { ProjectLabel = "armed", Mode = "auto_upload" },
        };

        TrayMenuModel menu = TrayMenuModel.Compute(status, Array.Empty<QueueEntry>(), rollup, projects);

        Assert.True(menu.IsPaused);
        Assert.Equal("This week: 4 contributed, 2 held for privacy review", menu.WeekText);
        Assert.Equal(new[] { "armed" }, menu.ArmedProjects);
    }
}

public class AutostartCommandTests
{
    [Fact]
    public void TheCommandIsQuotedSoASpaceInThePathCannotSplitIt()
    {
        Assert.Equal(
            "\"C:\\Users\\Ada Lovelace\\Trace Commons\\TraceCommons.exe\"",
            AutostartCommand.For("C:\\Users\\Ada Lovelace\\Trace Commons\\TraceCommons.exe"));
    }

    [Fact]
    public void NoArgumentsAreAppended()
    {
        // A launch at login is an ordinary launch. An argument here would be
        // a code path that only ever runs on a contributor's machine.
        Assert.Equal("\"C:\\a\\b.exe\"", AutostartCommand.For("C:\\a\\b.exe"));
    }

    [Fact]
    public void AnEmptyPathIsRefused()
    {
        Assert.Throws<ArgumentException>(() => AutostartCommand.For(string.Empty));
    }

    [Fact]
    public void APathContainingAQuoteIsRefusedRatherThanQuotedAnyway()
    {
        Assert.Throws<ArgumentException>(() => AutostartCommand.For("C:\\a\"b.exe"));
    }

    [Fact]
    public void TheKeyIsThePerUserRunKey()
    {
        // HKCU-relative. Nothing here may need elevation, and HKLM would.
        Assert.Equal(@"Software\Microsoft\Windows\CurrentVersion\Run", AutostartCommand.RunKeyPath);
        Assert.DoesNotContain("HKEY_LOCAL_MACHINE", AutostartCommand.RunKeyPath, StringComparison.Ordinal);
    }

    [Fact]
    public void TheEntryIsNamedSomeoneAuditingStartupWouldRecognise()
    {
        Assert.Equal("Trace Commons", AutostartCommand.ValueName);
    }

    [Fact]
    public void AStoredValueIsRecognisedThroughItsQuoting()
    {
        Assert.True(AutostartCommand.PointsAt("\"C:\\a\\b.exe\"", "C:\\a\\b.exe"));
        Assert.True(AutostartCommand.PointsAt("\"C:\\A\\B.EXE\"", "C:\\a\\b.exe"));
    }

    [Fact]
    public void AnEntryLeftBehindByAMovedCopyIsNotOurs()
    {
        Assert.False(AutostartCommand.PointsAt("\"D:\\old\\b.exe\"", "C:\\a\\b.exe"));
        Assert.False(AutostartCommand.PointsAt(null, "C:\\a\\b.exe"));
        Assert.False(AutostartCommand.PointsAt(string.Empty, "C:\\a\\b.exe"));
    }
}

public class MarkRasterTests
{
    private const uint Ink = 0xFF20241F;
    private const uint Dot = 0xFF178F70;

    [Theory]
    [InlineData(16)]
    [InlineData(20)]
    [InlineData(24)]
    [InlineData(32)]
    public void EveryDpiSizeProducesAFullBgraBuffer(int size)
    {
        Assert.Equal(size * size * 4, MarkRaster.Render(size, Ink).Length);
    }

    [Fact]
    public void ASizeOfZeroIsRefusedRatherThanReturningNothing()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => MarkRaster.Render(0, Ink));
    }

    [Fact]
    public void BothBracketsAreDrawnInTheInkTheyWereGiven()
    {
        const int size = 64;
        byte[] pixels = MarkRaster.Render(size, Ink);

        // Mid-stroke on the user's bracket (top left) and on the agent's
        // answer (bottom right), in the 64-unit coordinates the mark is
        // described in.
        AssertOpaque(pixels, size, 11, 20, Ink);
        AssertOpaque(pixels, size, 53, 44, Ink);
    }

    [Fact]
    public void TheCentreOfTheMarkIsEmpty()
    {
        // The session is the space between the brackets. If this ever fills
        // in, the geometry has been transcribed wrong.
        const int size = 64;
        byte[] pixels = MarkRaster.Render(size, Ink);
        Assert.Equal(0, Alpha(pixels, size, 32, 32));
    }

    [Fact]
    public void TheMarkIsDrawnTopDownAndNotUpsideDown()
    {
        // The two brackets are 180-degree rotations of each other, so a
        // flipped buffer looks plausible. What is not symmetric is the
        // corner: the user's bracket's corner is at the TOP LEFT.
        const int size = 64;
        byte[] pixels = MarkRaster.Render(size, Ink);
        Assert.True(Alpha(pixels, size, 11, 11) > 0, "top-left corner should be ink");
        Assert.Equal(0, Alpha(pixels, size, 11, 53));
    }

    [Fact]
    public void NoDotIsDrawnUnlessOneIsAsked()
    {
        const int size = 64;
        byte[] pixels = MarkRaster.Render(size, Ink);

        // Paused and idle carry no dot; the top-right quadrant stays empty.
        Assert.Equal(0, Alpha(pixels, size, 52, 12));
    }

    [Fact]
    public void TheStateDotSitsWhereNeitherBracketDoes()
    {
        const int size = 64;
        byte[] withDot = MarkRaster.Render(size, Ink, Dot);
        byte[] without = MarkRaster.Render(size, Ink);

        AssertOpaque(withDot, size, 52, 12, Dot);

        // And it did not eat either bracket.
        Assert.Equal(Alpha(without, size, 11, 20), Alpha(withDot, size, 11, 20));
        Assert.Equal(Alpha(without, size, 53, 44), Alpha(withDot, size, 53, 44));
    }

    [Fact]
    public void TheEdgesAreAntiAliasedRatherThanBinary()
    {
        // A 16px tray icon drawn without coverage sampling crawls between DPI
        // scales. The outer edge of the user's bracket is at 7/64, which at
        // 16px lands mid-pixel.
        byte[] pixels = MarkRaster.Render(16, Ink);

        var partial = 0;
        for (int i = 3; i < pixels.Length; i += 4)
        {
            if (pixels[i] > 0 && pixels[i] < 255)
            {
                partial++;
            }
        }

        Assert.True(partial > 0, "expected some partially covered pixels");
    }

    /// <summary>Alpha of the pixel covering a point in 64-unit mark space.</summary>
    private static int Alpha(byte[] pixels, int size, int viewX, int viewY)
    {
        int x = viewX * size / 64;
        int y = viewY * size / 64;
        return pixels[(y * size + x) * 4 + 3];
    }

    private static void AssertOpaque(byte[] pixels, int size, int viewX, int viewY, uint colour)
    {
        int x = viewX * size / 64;
        int y = viewY * size / 64;
        int offset = (y * size + x) * 4;

        Assert.Equal(255, pixels[offset + 3]);
        Assert.Equal((byte)(colour & 0xFF), pixels[offset + 0]);
        Assert.Equal((byte)(colour >> 8 & 0xFF), pixels[offset + 1]);
        Assert.Equal((byte)(colour >> 16 & 0xFF), pixels[offset + 2]);
    }
}

public class DaemonStatusTests
{
    [Fact]
    public void TheTraysWholeWorldParsesFromOneStatusFrame()
    {
        DaemonResponse response = DaemonResponse.Parse(
            """
            {"id":1,"result":{"logged_in":true,"paused":true,"queue_depth":4,
             "health":{"last_error_label":null,"since":null}}}
            """);

        DaemonStatus? status = response.ResultAs<DaemonStatus>();
        Assert.NotNull(status);
        Assert.True(status!.Paused);
        Assert.Equal(4, status.QueueDepth);
        Assert.True(status.IsHealthy);
    }

    [Fact]
    public void AnyErrorLabelAtAllCountsAsUnhealthy()
    {
        // Including one this client has never heard of: an unknown problem
        // must not render as fine.
        DaemonStatus? status = DaemonResponse
            .Parse("""{"id":1,"result":{"health":{"last_error_label":"some-future-label"}}}""")
            .ResultAs<DaemonStatus>();

        Assert.NotNull(status);
        Assert.False(status!.IsHealthy);
    }

    [Fact]
    public void AStatusFrameWithoutHealthIsHealthy()
    {
        DaemonStatus? status = DaemonResponse
            .Parse("""{"id":1,"result":{"queue_depth":0}}""")
            .ResultAs<DaemonStatus>();

        Assert.NotNull(status);
        Assert.True(status!.IsHealthy);
    }
}

public class DigestEventTests
{
    [Fact]
    public void ADigestDueFrameCarriesItsPendingCount()
    {
        DaemonEvent? evt = DaemonEvent.Parse(
            """{"event":"digest_due","data":{"pending":3,"text":"3 sessions ready to contribute"}}""");

        Assert.NotNull(evt);
        Assert.Equal(DaemonProtocol.Events.DigestDue, evt!.Event);
        Assert.Equal(3, evt.PendingCount);
    }

    [Fact]
    public void AFrameWithoutACountReadsAsNothingToSay()
    {
        // Zero suppresses the notification downstream, which is the safe
        // direction for a frame this client could not understand.
        DaemonEvent? evt = DaemonEvent.Parse("""{"event":"digest_due"}""");
        Assert.NotNull(evt);
        Assert.Equal(0, evt!.PendingCount);
    }

    [Fact]
    public void SkippedAndPendingDoNotReadEachOther()
    {
        DaemonEvent? lagged = DaemonEvent.Parse("""{"event":"lagged","data":{"skipped":9}}""");
        Assert.NotNull(lagged);
        Assert.Equal(9, lagged!.SkippedCount);
        Assert.Equal(0, lagged.PendingCount);
    }
}
