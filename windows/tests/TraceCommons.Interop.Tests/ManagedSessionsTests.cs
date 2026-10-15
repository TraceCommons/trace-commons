using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class ManagedSessionsTests
{
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

    [Fact]
    public void AvailabilityFiltersSetupAndSignInWithoutLosingSavedAccounts()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn)! with {
            Tools = [new("claude", false, "https://claude.ai/download"), new("codex", true, "https://developers.openai.com/codex/cli/"), new("opencode", true, "https://opencode.ai")],
            Accounts = [new("saved", "claude", "subscription", "Personal", "ready")]
        };
        Assert.Equal(["codex"], snapshot.SetupTools);
        Assert.Null(snapshot.SignInEntry("claude"));
        Assert.NotNull(snapshot.SignInEntry("codex"));
        Assert.False(snapshot.CanLaunch("opencode"));
        Assert.Single(snapshot.Accounts);
        // A tool is installed: no install block (Ron, 2026-10-15).
        Assert.Empty(snapshot.InstallTools);
        // The missing tool's account: Reconnect hidden, the tool named.
        Assert.False(snapshot.OffersReconnect(snapshot.Accounts[0]));
        Assert.True(snapshot.OffersReconnect(snapshot.Accounts[0] with { Tool = "codex" }));
        Assert.False(snapshot.OffersReconnect(snapshot.Accounts[0] with { Tool = "codex", Connection = "api_key" }));
        var worded = snapshot with { Copy = new() { ["tool_unavailable"] = "{tool} wasn't found on this computer.", ["claude"] = "Claude Code" } };
        Assert.Equal("Claude Code wasn't found on this computer.", worded.ToolUnavailable("claude"));
    }

    [Fact]
    public void EmptyInventoryAndOlderSnapshotsHaveDistinctBehavior()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn)!;
        Assert.Equal(["claude", "codex"], snapshot.SetupTools);
        Assert.Empty((snapshot with { Tools = [] }).SetupTools);
        var decoded = ManagedSnapshot.Parse(SignedIn.Replace("\"revision\":1", "\"tools\":[{\"tool\":\"codex\",\"installed\":true,\"install_url\":\"https://example.test\"}],\"revision\":1"))!;
        Assert.Equal(["codex"], decoded.SetupTools);
        Assert.Equal("https://example.test", Assert.Single(decoded.Tools!).InstallUrl);
        var empty = snapshot with { Tools = [new("claude", false, "a"), new("codex", false, "b"), new("opencode", false, "c")] };
        // Windows stays on Claude Code and Codex until it launches OpenCode.
        Assert.Equal(["claude", "codex"], empty.InstallTools.Select(t => t.Tool));
        Assert.Empty(empty.SetupTools);
        Assert.Empty(snapshot.InstallTools);
        Assert.Empty((snapshot with { Tools = [new("opencode", true, "c")] }).SetupTools);
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
