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
