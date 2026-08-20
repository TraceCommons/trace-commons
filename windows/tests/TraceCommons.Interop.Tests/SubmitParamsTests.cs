using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The two request shapes one-click submit ever builds: an entry, or a
/// project. Checked here rather than trusted at each call site, so the row
/// button and the project-group button cannot drift into two spellings of
/// the same request.
/// </summary>
public sealed class SubmitParamsTests
{
    [Fact]
    public void ForEntrySendsOnlyTheEntryId()
    {
        string json = SubmitParams.ForEntry("entry-123");

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.Equal("entry-123", doc.RootElement.GetProperty("entry_id").GetString());
        Assert.False(doc.RootElement.TryGetProperty("project_id", out _));
    }

    [Fact]
    public void ForProjectSendsOnlyTheProjectId()
    {
        string json = SubmitParams.ForProject("proj_abcdef");

        using JsonDocument doc = JsonDocument.Parse(json);
        Assert.Equal("proj_abcdef", doc.RootElement.GetProperty("project_id").GetString());
        Assert.False(doc.RootElement.TryGetProperty("entry_id", out _));
    }

    [Fact]
    public void AnEmptyEntryIdIsRejectedRatherThanSentToTheDaemon()
    {
        Assert.Throws<ArgumentException>(() => SubmitParams.ForEntry(string.Empty));
        Assert.Throws<ArgumentException>(() => SubmitParams.ForEntry("   "));
    }

    [Fact]
    public void AnEmptyProjectIdIsRejectedRatherThanSentToTheDaemon()
    {
        Assert.Throws<ArgumentException>(() => SubmitParams.ForProject(string.Empty));
        Assert.Throws<ArgumentException>(() => SubmitParams.ForProject("   "));
    }
}
