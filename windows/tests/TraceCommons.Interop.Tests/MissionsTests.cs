using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W23: Missions under #1174's rules, pinned to the Mac's
/// macos/Tests/TraceCommonsAppTests/MissionsTests.swift and its sample
/// catalogue (SampleDaemonData.swift:402-403).
/// </summary>
public class MissionsTests
{
    private static string Word(string key) => MonitorShellCopy.Top(key);

    /// <summary>The Mac's provisional sample catalogue, SampleDaemonData.swift:402-403.</summary>
    [Fact]
    public void TheMacsSampleCatalogueDecodes()
    {
        MissionCatalogue catalogue = MissionCatalogue.Parse(MissionSamples.ProvisionalCatalogue)!;
        Assert.Equal(new[] { "Debug a failing test", "Review a pull request" }, System.Linq.Enumerable.Select(catalogue.Missions, m => m.Title));
        Assert.Equal("Sample settlement explanation, supplied by the commons", MissionFormat.Condition(catalogue));
        Assert.Equal("2", MissionFormat.Count(catalogue));
        Assert.Equal("—", MissionFormat.Count(null));
    }

    /// <summary>MissionsTests.swift:30-49: projected credit carries its condition; no range is a dash.</summary>
    [Fact]
    public void CreditIsProjectedAndCarriesItsCondition()
    {
        MissionCatalogue catalogue = MissionCatalogue.Parse(
            "{\"fetched_at\":null,\"posture\":{\"settlement\":\"disabled\",\"graded\":false,\"explanation\":\"Waits on settlement\"},"
            + "\"missions\":[{\"id\":\"a\",\"title\":\"A\",\"summary\":null,\"credit_range\":{\"min\":5,\"max\":20,\"unit\":\"points\"}},"
            + "{\"id\":\"b\",\"title\":\"B\",\"summary\":null,\"credit_range\":{\"min\":3,\"max\":3,\"unit\":\"points\"}},"
            + "{\"id\":\"c\",\"title\":\"C\",\"summary\":null,\"credit_range\":null}]}")!;
        Assert.Equal("Waits on settlement", MissionFormat.Condition(catalogue));
        Assert.Equal(Word("mission_credit_points").Replace("{min}", "5").Replace("{max}", "20"), MissionFormat.Credit(catalogue.Missions[0], catalogue, Word));
        Assert.Equal(Word("mission_credit_points_one").Replace("{min}", "3"), MissionFormat.Credit(catalogue.Missions[1], catalogue, Word));
        Assert.Equal("—", MissionFormat.Credit(catalogue.Missions[2], catalogue, Word));
        Assert.True(MissionFormat.ShowsProjected(catalogue, Word));
        Assert.Equal("5–20 points", MissionFormat.Credit(catalogue.Missions[0], catalogue, Word));
    }

    /// <summary>MissionsTests.swift:53-62: a unit this build cannot word is a dash, never the wire label.</summary>
    [Fact]
    public void AnUnwordedUnitIsADash()
    {
        MissionCatalogue catalogue = MissionCatalogue.Parse(
            "{\"fetched_at\":null,\"posture\":{\"settlement\":\"disabled\",\"graded\":false,\"explanation\":\"Waits on settlement\"},"
            + "\"missions\":[{\"id\":\"a\",\"title\":\"A\",\"summary\":null,\"credit_range\":{\"min\":5,\"max\":20,\"unit\":\"points_per_accepted_trace\"}},"
            + "{\"id\":\"b\",\"title\":\"B\",\"summary\":null,\"credit_range\":{\"min\":1,\"max\":2,\"unit\":\"USD\"}}]}")!;
        Assert.Equal("—", MissionFormat.Credit(catalogue.Missions[0], catalogue, Word));
        Assert.Equal("—", MissionFormat.Credit(catalogue.Missions[1], catalogue, Word));
        Assert.False(MissionFormat.ShowsProjected(catalogue, Word));
    }

    /// <summary>MissionsTests.swift:67-74: projected, never pending, with the core's note.</summary>
    [Fact]
    public void MissionCreditIsProjectedNeverPending()
    {
        Assert.NotEmpty(Word("projected"));
        Assert.NotEqual(Word("pending"), Word("projected"));
        Assert.NotEmpty(Word("projected_note"));
    }

    /// <summary>MissionsTests.swift:78-86: no condition, no figure.</summary>
    [Fact]
    public void NoConditionMeansNoFigure()
    {
        MissionCatalogue catalogue = MissionCatalogue.Parse(
            "{\"fetched_at\":null,\"posture\":null,\"missions\":[{\"id\":\"a\",\"title\":\"A\",\"summary\":null,\"credit_range\":{\"min\":5,\"max\":20,\"unit\":\"points\"}}]}")!;
        Assert.Null(MissionFormat.Condition(catalogue));
        Assert.Equal("—", MissionFormat.Credit(catalogue.Missions[0], catalogue, Word));
        Assert.False(MissionFormat.ShowsProjected(catalogue, Word));
    }

    /// <summary>
    /// A failed read is the core's line, never 0 (MissionsViews.swift:33-35,
    /// 57-64; MonitorScreensCopy.swift:371-374); the daemon's own
    /// <c>mission_catalogue</c> reply, a different shape, is not a catalogue.
    /// </summary>
    [Fact]
    public void AFailedReadIsTheCoresLineAndTheWireShapeIsNotACatalogue()
    {
        Assert.Equal(Word("core_unreachable"), MissionFormat.FailureLine(unreachable: true, Word));
        Assert.Equal(Word("request_failed"), MissionFormat.FailureLine(unreachable: false, Word));
        Assert.NotEmpty(Word("request_failed"));
        Assert.Null(MissionCatalogue.Parse(
            "{\"kind\":\"skill_evaluation\",\"disclosure\":\"d\",\"catalogue\":{\"schema_version\":1,\"entries\":[],\"next_cursor\":null}}"));
        Assert.Null(MissionCatalogue.Parse("not json"));
    }

    /// <summary>
    /// A live launch: the daemon has no catalogue to give, which the Mac
    /// reads as not served and draws as a dash named with the core's word
    /// for unknown (HomeStore.swift:215-217, MissionsViews.swift:58-64),
    /// not the request-failed line.
    /// </summary>
    [Fact]
    public void NotServedIsADashNamedUnknownNotAFailure()
    {
        MissionPage page = MissionPage.Settle(MissionRead.NotServed, null, Word);
        Assert.Null(page.Catalogue);
        Assert.Null(page.FailureLine);
        Assert.Equal(Word("unknown"), page.UnknownName);
        Assert.NotEmpty(Word("unknown"));
    }

    /// <summary>
    /// A core that is not there is its line; a reply that came back but will
    /// not decode as the Mac decodes it is a failed request; neither draws
    /// the dash beside its line (MissionsViews.swift:33-35, 57).
    /// </summary>
    [Fact]
    public void AFailedReadIsALineAndNoDash()
    {
        MissionPage down = MissionPage.Settle(MissionRead.Unreachable, null, Word);
        Assert.Equal(Word("core_unreachable"), down.FailureLine);
        Assert.Null(down.UnknownName);
        Assert.Null(down.Catalogue);

        MissionPage garbled = MissionPage.Settle(MissionRead.Served, "not json", Word);
        Assert.Equal(Word("request_failed"), garbled.FailureLine);
        Assert.Null(garbled.UnknownName);
        Assert.Null(garbled.Catalogue);

        MissionPage served = MissionPage.Settle(MissionRead.Served, MissionSamples.ProvisionalCatalogue, Word);
        Assert.NotNull(served.Catalogue);
        Assert.Null(served.FailureLine);
        Assert.Null(served.UnknownName);
    }

    /// <summary>
    /// Every sample set serves the Mac's catalogue except <c>coreDown</c>,
    /// whose read is unreachable and draws the core's line
    /// (SampleDaemonClient.swift:576-581).
    /// </summary>
    [Fact]
    public void EverySampleSetButCoreDownServesTheCatalogue()
    {
        Assert.Contains(SampleDaemon.CoreDown, SampleDaemon.Sets);
        foreach (string set in SampleDaemon.Sets)
        {
            (MissionRead read, string? json) = MissionSamples.Read(set);
            MissionPage page = MissionPage.Settle(read, json, Word);
            if (set == SampleDaemon.CoreDown)
            {
                Assert.Equal(MissionRead.Unreachable, read);
                Assert.Null(page.Catalogue);
                Assert.Equal(Word("core_unreachable"), page.FailureLine);
            }
            else
            {
                Assert.Equal(MissionRead.Served, read);
                Assert.Equal(2, page.Catalogue!.Missions.Count);
                Assert.Null(page.FailureLine);
            }
        }
    }
}
