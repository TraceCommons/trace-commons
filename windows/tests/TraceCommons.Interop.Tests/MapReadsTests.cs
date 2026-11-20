using System.Collections.Generic;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's decoders keep the Mac's optionality (DaemonDataModels.swift): a
/// failed or malformed read is unread (null), never an empty answer, and a
/// field the reply leaves out is null, never a default that reads as an
/// answer -- the gate is closed on a status with no <c>paused</c> field
/// (FlowMapScene.swift:350, probe "gates" pausedMissing).
/// </summary>
public class MapReadsTests
{
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("not json")]
    [InlineData("[]")]
    [InlineData("{\"other\":[]}")]
    public void AReadThatDidNotAnswerIsUnreadNotEmpty(string? json)
    {
        Assert.Null(MapReads.Pending(json));
        Assert.Null(MapReads.Projects(json));
        Assert.Null(MapReads.History(json));
        Assert.Null(MapReads.Destinations(json));
    }

    [Fact]
    public void AnEmptyListIsAnAnswer()
    {
        Assert.Empty(MapReads.Pending("{\"pending\":[]}")!);
        Assert.Empty(MapReads.Projects("{\"projects\":[]}")!);
    }

    [Fact]
    public void AStatusWithoutItsFieldsSaysNothingAndKeepsTheGateClosed()
    {
        MapStatus status = MapReads.Status("{}")!;
        Assert.Null(status.Paused);
        Assert.Null(status.LoggedIn);
        Assert.Null(status.HeldSessions);
        Assert.Null(status.GrantVoids);
        var routes = MapReads.Destinations("{\"sessions_route\":\"local\",\"tools\":[]}");
        Assert.False(new CommonsGate(true, status, routes).Open);
    }

    [Fact]
    public void AManagedStateThisShellDoesNotKnowIsSaidAsUnknown()
    {
        // ManagedSessionsSurface.swift:307-313.
        var copy = new Dictionary<string, string> { ["state_unknown"] = "U", ["state_running"] = "R", ["provider_x"] = "X" };
        Assert.Equal("X · m · U", new MapManagedSession("1", "codex", "x", "m", "c", "rebooting").Detail(copy));
        Assert.Equal("X · R", new MapManagedSession("1", "codex", "x", null, "c", "running").Detail(copy));
    }
}
