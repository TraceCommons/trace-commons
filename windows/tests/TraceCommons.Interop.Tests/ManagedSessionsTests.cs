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
        Assert.Equal(new[] { "api_key", "near_ai" }, ManagedSnapshot.Connections("opencode", signedIn: true));
        Assert.Equal(new[] { "subscription", "api_key" }, ManagedSnapshot.Connections("claude", signedIn: true));
        Assert.Equal(new[] { "subscription", "api_key", "near_ai" }, ManagedSnapshot.Connections("codex"));
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
        Assert.False(snapshot.NearAiSignedIn);
        Assert.Null(snapshot.SignInLaunchParams("claude", "C:\\p", "r"));
    }

    private const string SignedIn = """
    {"revision":1,"accounts":[],"sign_in_accounts":[{"id":"c","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"x","tool":"codex","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],"sessions":[],"generations":{"claude":4},"capabilities":{"managed_launch":true,"near_ai_signin":true,"terminal_launch":true},"copy":{}}
    """;

    [Fact]
    public void SignInEntriesLaunchWithNoSavedAccount()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn);
        Assert.NotNull(snapshot);
        Assert.True(snapshot.NearAiSignedIn);
        Assert.Equal("x", snapshot.SignInEntry("codex")?.Id);
        var request = snapshot.SignInLaunchParams("claude", "C:\\p", "r");
        Assert.NotNull(request);
        Assert.Equal("near_ai_signin", request["connection"]);
        Assert.Equal("c", request["account_id"]);
        Assert.Equal("claude", request["tool"]);
        Assert.Equal("C:\\p", request["cwd"]);
        Assert.Equal("r", request["request_id"]);
        Assert.Equal("coding", request["purpose"]);
        Assert.Equal(4UL, request["expected_generation"]);
        Assert.Equal(false, request["save_default"]);
        Assert.Null(snapshot.SignInLaunchParams("future_tool", "C:\\p", "r"));
    }

    [Fact]
    public void SignInEntriesAreIgnoredWithoutTheCapability()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn.Replace("\"near_ai_signin\":true", "\"near_ai_signin\":false"));
        Assert.NotNull(snapshot);
        Assert.False(snapshot.NearAiSignedIn);
        Assert.Null(snapshot.SignInEntry("claude"));
    }

    [Fact]
    public void SignInSurfacePicksCoreKeys()
    {
        const string line = "Uses your NEAR AI sign-in. Runs in {destination}, in the folder you choose.";
        Assert.Equal("Uses your NEAR AI sign-in. Runs in Windows Terminal, in the folder you choose.",
            ManagedSignIn.Scope(line, "Windows Terminal", "your terminal"));
        Assert.Equal("Uses your NEAR AI sign-in. Runs in your terminal, in the folder you choose.",
            ManagedSignIn.Scope(line, null, "your terminal"));
        Assert.Equal("start_claude", ManagedSignIn.StartKey("claude"));
        Assert.Equal("start_codex", ManagedSignIn.StartKey("codex"));
        Assert.Null(ManagedSignIn.StartKey("future_tool"));
        Assert.Equal(["subscription", "api_key"], ManagedSignIn.ConnectionOptions(true));
        Assert.Equal(["subscription", "api_key", "near_ai"], ManagedSignIn.ConnectionOptions(false));
        Assert.Equal("sign_in_needed", ManagedSignIn.ErrorKey("managed-near-ai-sign-in-required"));
        Assert.Equal("action_failed", ManagedSignIn.ErrorKey("managed-conflict"));
        Assert.Equal("action_failed", ManagedSignIn.ErrorKey(null));
    }
}
