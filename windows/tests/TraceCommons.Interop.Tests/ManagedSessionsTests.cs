using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class ManagedSessionsTests
{
    [Theory]
    [InlineData("claude")]
    [InlineData("codex")]
    public void VersionRecoveryTargetsTheAttemptedTool(string attempted)
    {
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":1,"accounts":[],"sessions":[],"generations":{},"capabilities":{},"copy":{},"tools":[{"tool":"claude","installed":true,"install_url":"claude-docs"},{"tool":"codex","installed":true,"install_url":"codex-docs"},{"tool":"opencode","installed":false,"install_url":"opencode-docs"}]}
        """)!;
        Assert.Equal(attempted, Assert.Single(snapshot.VersionRecoveryTools(attempted)).Tool);
        Assert.Equal(2, snapshot.VersionRecoveryTools(null).Length);
    }

    [Fact]
    public void DiscoveryHidesAbsentToolsButPreservesLegacyUnknown()
    {
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":1,"accounts":[],"sessions":[],"generations":{},"capabilities":{},"copy":{},"tools":[{"tool":"claude","installed":false,"install_url":"https://code.claude.com/docs/en/setup"},{"tool":"opencode","installed":true,"install_url":"https://opencode.ai/docs/"}]}
        """)!;
        Assert.False(snapshot.CanLaunch("claude"));
        Assert.Equal(new[] { "opencode" }, snapshot.SetupTools);
        Assert.True((snapshot with { Tools = null }).CanLaunch("claude"));
        Assert.Equal(3, (snapshot with { Tools = null }).SetupTools.Length);
        Assert.Empty((snapshot with { Tools = new[] { new ManagedTool("claude", false, "") } }).SetupTools);
    }

    [Fact]
    public void OpenCodeAcceptsOnlyKeyConnectionsAndPreservesCollectionState()
    {
        Assert.Equal(new[] { "api_key", "near_ai" }, ManagedSnapshot.Connections("opencode"));
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":1,"accounts":[],"sessions":[{"id":"s","tool":"opencode","connection":"near_ai","account_id":"a","account_label":"Work","project_label":"Project","state":"exited","purpose":"coding","model":"provider/model","collect_traces":false,"collection":"off"}],"generations":{},"capabilities":{},"copy":{}}
        """)!;
        Assert.Equal("provider/model", snapshot.Sessions[0].Model);
        Assert.False(snapshot.Sessions[0].CollectTraces);
        Assert.Equal("off", snapshot.Sessions[0].Collection);
    }

    [Fact]
    public void FutureSessionStateKeepsItsAccountLocked()
    {
        var session = new ManagedSession("s", "claude", "subscription", "a", "Personal", "Project", "future", "coding");
        Assert.True(session.HoldsAccount);
        Assert.False((session with { State = "exited" }).HoldsAccount);
    }

    [Fact]
    public void SnapshotKeepsCliSessionIdentityAndTerminalCapability()
    {
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":4,"accounts":[],"sessions":[{"id":"s","tool":"codex","connection":"subscription","account_id":"a","account_label":"Work","project_label":"Project","state":"running","purpose":"coding"}],"generations":{"codex":2},"capabilities":{"managed_launch":true,"terminal_launch":false},"copy":{"title":"Managed sessions"}}
        """);
        Assert.NotNull(snapshot);
        Assert.Equal("Work", snapshot.Sessions[0].AccountLabel);
        Assert.False(snapshot.Capabilities.TerminalLaunch);
        Assert.Equal(2UL, snapshot.Generations["codex"]);
    }
}
