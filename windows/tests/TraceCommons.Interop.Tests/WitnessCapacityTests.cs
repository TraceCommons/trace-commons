using System;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// status.witness_capacity: approved sessions held because the privacy
/// witness is busy, and the notice the Rust words for them.
/// </summary>
public class WitnessCapacityTests
{
    private const string Saturated = """
        {"schema_version":"trace_commons.daemon.v1_1","logged_in":true,
         "paused":false,"queue_depth":0,
         "health":{"last_error_label":"queue-full","since":"2030-01-01T00:00:00Z"},
         "witness_capacity":{"waiting_sessions":2,"next_retry_at":"2030-01-01T00:01:00Z"}}
        """;

    /// <summary>Read beside an unrelated health label, which is the point.</summary>
    [Fact]
    public void TheCapacityIsReadFromStatusAlongsideAnUnrelatedHealthLabel()
    {
        DaemonStatus? status = JsonSerializer.Deserialize<DaemonStatus>(Saturated);

        Assert.NotNull(status);
        Assert.Equal("queue-full", status!.Health?.LastErrorLabel);
        Assert.True(status.WitnessCapacity!.Waiting);
        Assert.Equal(2, status.WitnessCapacity.WaitingSessions);
        Assert.Equal(
            new DateTimeOffset(2030, 1, 1, 0, 1, 0, TimeSpan.Zero),
            status.WitnessCapacity.NextRetryAtUtc);
    }

    [Fact]
    public void ADaemonThatPredatesTheFieldHoldsNothingOnTheWitness()
    {
        DaemonStatus? status = JsonSerializer.Deserialize<DaemonStatus>(
            """{"logged_in":true,"health":{"last_error_label":null,"since":null}}""");

        Assert.Null(status!.WitnessCapacity);
        Assert.Null(HealthCopy.ForWitnessCapacity(status.WitnessCapacity, null));
    }

    /// <summary>Only the count reaches the Rust; it is all the words depend on.</summary>
    [Fact]
    public void TheWireHandedToTheCoreCarriesTheCount()
    {
        var capacity = new WitnessCapacity { WaitingSessions = 3, NextRetryAt = "2030-01-01T00:01:00Z" };
        using JsonDocument document = JsonDocument.Parse(capacity.WireJson);
        Assert.Equal(3, document.RootElement.GetProperty("waiting_sessions").GetInt64());
    }

    private const string NoticeJson = """
        {"title":"Waiting for the privacy witness","body":"2 approved sessions are waiting.","next_check":"Next try"}
        """;

    [Fact]
    public void TheNoticeIsTakenWholeOrNotAtAll()
    {
        WitnessCapacityNotice? notice = WitnessCapacitySurface.Parse(NoticeJson);
        Assert.NotNull(notice);
        Assert.Equal("Next try", notice!.NextCheck);

        foreach (string field in WitnessCapacityNotice.ConsumedFields)
        {
            string blanked = NoticeJson.Replace(
                $"\"{field}\":\"",
                $"\"{field}\":\"\",\"ignored\":\"",
                StringComparison.Ordinal);
            Assert.Null(WitnessCapacitySurface.Parse(blanked));
        }

        Assert.Null(WitnessCapacitySurface.Parse(null));
        Assert.Null(WitnessCapacitySurface.Parse("not json"));
    }

    /// <summary>
    /// The banner is the Rust's title and body, with the next try when the
    /// daemon gave one, and no action.
    /// </summary>
    [Fact]
    public void TheBannerIsTheRustsNoticeWithTheNextTry()
    {
        WitnessCapacityNotice notice = WitnessCapacitySurface.Parse(NoticeJson)!;
        var timed = new WitnessCapacity { WaitingSessions = 2, NextRetryAt = "2030-01-01T00:01:00Z" };
        var untimed = new WitnessCapacity { WaitingSessions = 2 };

        HealthCopy? banner = HealthCopy.ForWitnessCapacity(timed, notice);
        Assert.NotNull(banner);
        Assert.Equal(notice.Title, banner!.Title);
        Assert.StartsWith(notice.Body, banner.Detail, StringComparison.Ordinal);
        Assert.Contains(notice.NextCheck, banner.Detail, StringComparison.Ordinal);
        Assert.Null(banner.ActionLabel);

        Assert.Equal(notice.Body, HealthCopy.ForWitnessCapacity(untimed, notice)!.Detail);
        Assert.Null(HealthCopy.ForWitnessCapacity(new WitnessCapacity(), notice));
    }

    /// <summary>
    /// Through the real cdylib: the exported fields are exactly the ones this
    /// shell decodes, and nothing waiting gets no notice.
    /// </summary>
    [Fact]
    public void TheExportedFieldsAreExactlyTheOnesThisShellConsumes()
    {
        var capacity = new WitnessCapacity { WaitingSessions = 1 };
        string json = NativeMethods.TakeOwnedString(
                NativeMethods.tc_witness_capacity_notice(capacity.WireJson))
            ?? throw new InvalidOperationException("tc_witness_capacity_notice returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        var exported = document.RootElement.EnumerateObject()
            .Select(property => property.Name)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToList();

        Assert.Equal(
            WitnessCapacityNotice.ConsumedFields.OrderBy(name => name, StringComparer.Ordinal).ToList(),
            exported);
        Assert.Null(WitnessCapacitySurface.Notice(new WitnessCapacity()));
    }
}
