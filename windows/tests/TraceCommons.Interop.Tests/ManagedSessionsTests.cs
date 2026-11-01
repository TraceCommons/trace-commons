using System.Linq;
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

    [Fact]
    public void AvailabilityFiltersSetupAndSignInWithoutLosingSavedAccounts()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn)! with {
            Tools = [new("claude", false, "https://claude.ai/download"), new("codex", true, "https://developers.openai.com/codex/cli/"), new("opencode", true, "https://opencode.ai")],
            Accounts = [new("saved", "claude", "subscription", "Personal", "ready")]
        };
        Assert.Equal(["codex", "opencode"], snapshot.SetupTools);
        Assert.Null(snapshot.SignInEntry("claude"));
        Assert.NotNull(snapshot.SignInEntry("codex"));
        // Windows launches OpenCode since #1418.
        Assert.True(snapshot.CanLaunch("opencode"));
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
    public void AnInstalledOpenCodeAccountIsNotReportedMissing()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn)! with {
            Tools = [new("claude", true, "a"), new("codex", false, "b"), new("opencode", true, "c")],
        };
        var account = new ManagedAccount("oc", "opencode", "subscription", "Zed", "ready");
        Assert.False(snapshot.ToolMissing("opencode"));
        Assert.True(snapshot.ToolMissing("codex"));
        Assert.False(snapshot.ToolMissing("claude"));
        Assert.True((snapshot with { Tools = [new("opencode", false, "c")] }).ToolMissing("opencode"));
        Assert.False(ManagedSnapshot.Parse(SignedIn)!.ToolMissing("opencode"));
        // OpenCode launches (#1418) but has no subscription to reconnect.
        Assert.True(snapshot.CanLaunch("opencode"));
        Assert.False(snapshot.OffersReconnect(account));
    }

    [Fact]
    public void EmptyInventoryAndOlderSnapshotsHaveDistinctBehavior()
    {
        var snapshot = ManagedSnapshot.Parse(SignedIn)!;
        Assert.Equal(["claude", "codex", "opencode"], snapshot.SetupTools);
        Assert.Empty((snapshot with { Tools = [] }).SetupTools);
        var decoded = ManagedSnapshot.Parse(SignedIn.Replace("\"revision\":1", "\"tools\":[{\"tool\":\"codex\",\"installed\":true,\"install_url\":\"https://example.test\"}],\"revision\":1"))!;
        Assert.Equal(["codex"], decoded.SetupTools);
        Assert.Equal("https://example.test", Assert.Single(decoded.Tools!).InstallUrl);
        var empty = snapshot with { Tools = [new("claude", false, "a"), new("codex", false, "b"), new("opencode", false, "c")] };
        // Windows launches OpenCode since #1418, so it is offered too.
        Assert.Equal(["claude", "codex", "opencode"], empty.InstallTools.Select(t => t.Tool));
        Assert.Empty(empty.SetupTools);
        Assert.Empty(snapshot.InstallTools);
        Assert.Equal(["opencode"], (snapshot with { Tools = [new("opencode", true, "c")] }).SetupTools);
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
        const string line = "Uses your NEAR AI sign-in.";
        Assert.Equal("Uses your NEAR AI sign-in.",
            ManagedSignIn.Scope(line, "Windows Terminal", "your terminal"));
        Assert.Equal("Uses your NEAR AI sign-in.",
            ManagedSignIn.Scope(line, null, "your terminal"));
        Assert.Equal("start_claude", ManagedSignIn.StartKey("claude"));
        Assert.Equal("start_codex", ManagedSignIn.StartKey("codex"));
        Assert.Null(ManagedSignIn.StartKey("future_tool"));
        Assert.Equal(["subscription", "api_key"], ManagedSignIn.ConnectionOptions(true));
        Assert.Equal(["subscription", "api_key", "near_ai"], ManagedSignIn.ConnectionOptions(false));
        Assert.Equal("sign_in_needed", ManagedSignIn.ErrorKey("managed-near-ai-sign-in-required"));
        Assert.Equal("balance_too_low", ManagedSignIn.ErrorKey("managed-near-ai-balance-too-low"));
        Assert.Equal("project_chooses_model", ManagedSignIn.ErrorKey("managed-project-chooses-model"));
        Assert.Equal("action_failed", ManagedSignIn.ErrorKey("managed-conflict"));
        Assert.Equal("action_failed", ManagedSignIn.ErrorKey(null));
        Assert.Equal("terminal_not_found", ManagedSignIn.ErrorKey("managed-terminal-not-found"));
    }

    [Fact]
    public void SnapshotCarriesTheTerminalChoiceAndTheFoundTerminals()
    {
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":2,"accounts":[],"sessions":[],"generations":{},"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_destination":"Windows Terminal","terminal_current":"windows_terminal","terminal_choices":[{"id":"windows_terminal","name":"Windows Terminal"}]},"copy":{"terminal_choice":"Open sessions in"}}
        """);
        Assert.NotNull(snapshot);
        Assert.Equal("windows_terminal", snapshot.Capabilities.TerminalCurrent);
        Assert.Equal(new[] { new ManagedTerminalChoice("windows_terminal", "Windows Terminal") }, snapshot.TerminalChoices);
        Assert.Equal(0, snapshot.TerminalSelectedIndex);
        Assert.Equal("Open sessions in", snapshot.Text("terminal_choice"));
    }

    [Fact]
    public void AnOlderSnapshotOffersNoTerminalChoice()
    {
        var snapshot = ManagedSnapshot.Parse("""
        {"revision":1,"accounts":[],"sessions":[],"generations":{},"capabilities":{"managed_launch":true,"terminal_launch":true,"terminal_destination":"Windows Terminal"},"copy":{}}
        """);
        Assert.NotNull(snapshot);
        Assert.Null(snapshot.Capabilities.TerminalCurrent);
        Assert.Empty(snapshot.TerminalChoices);
        Assert.Equal(-1, snapshot.TerminalSelectedIndex);
    }

    [Fact]
    public void AMissingTerminalHasItsOwnLine()
    {
        const string line = "{terminal} wasn't found on this computer. Choose another in Settings.";
        Assert.Equal("Windows Terminal wasn't found on this computer. Choose another in Settings.",
            ManagedSignIn.FillTerminal(line, "Windows Terminal", "your terminal"));
        Assert.Equal("your terminal wasn't found on this computer. Choose another in Settings.",
            ManagedSignIn.FillTerminal(line, null, "your terminal"));
    }
}
