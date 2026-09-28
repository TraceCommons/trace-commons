using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The notice after a legacy invite identity moved to a NEAR AI account:
/// <c>status.legacy_invite_migration.notice</c> handed back to the ABI
/// verbatim, and the words it returns.
/// </summary>
public sealed class LegacyMigrationNoticeTests
{
    private static readonly Dictionary<string, string> Complete = new()
    {
        ["title"] = "T",
        ["body"] = "B",
        ["folders"] = "F",
        ["acknowledge"] = "K",
    };

    private static JsonElement Status(string json) => JsonDocument.Parse(json).RootElement.Clone();

    [Fact]
    public void TheNoticeObjectGoesToTheAbiVerbatim()
    {
        var seen = new List<string>();
        LegacyMigrationNotice? notice = LegacyMigrationNotices.Notice(
            Status("{\"offered\":false,\"notice\":{\"folders_kept\":2,\"automatic_grant_kept\":false}}"),
            wire =>
            {
                seen.Add(wire);
                return JsonSerializer.Serialize(Complete);
            });

        Assert.NotNull(notice);
        Assert.Equal("T", notice!.Title);
        Assert.Equal("F", notice.Folders);
        string wire = Assert.Single(seen);
        using JsonDocument passed = JsonDocument.Parse(wire);
        Assert.Equal(2, passed.RootElement.GetProperty("folders_kept").GetInt32());
    }

    [Fact]
    public void NoNoticeAndAnOlderDaemonShowNothing()
    {
        var calls = 0;
        string? Word(string _)
        {
            calls++;
            return JsonSerializer.Serialize(Complete);
        }

        Assert.Null(LegacyMigrationNotices.Notice(null, Word));
        Assert.Null(LegacyMigrationNotices.Notice(Status("{\"offered\":true,\"notice\":null}"), Word));
        Assert.Equal(0, calls);
    }

    [Fact]
    public void ANoticeIsShownWholeOrNotAtAll()
    {
        foreach (string field in LegacyMigrationNotice.ConsumedFields)
        {
            var partial = new Dictionary<string, string>(Complete) { [field] = string.Empty };
            Assert.Null(LegacyMigrationNotices.Notice(
                Status("{\"notice\":{\"folders_kept\":0}}"),
                _ => JsonSerializer.Serialize(partial)));
        }

        Assert.Null(LegacyMigrationNotices.Notice(Status("{\"notice\":{}}"), _ => null));
        Assert.Null(LegacyMigrationNotices.Notice(Status("{\"notice\":{}}"), _ => "not json"));
    }
}
