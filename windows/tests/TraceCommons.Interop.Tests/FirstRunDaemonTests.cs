using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first run's daemon calls carry the Mac's method names and
/// parameters (DaemonClient.swift, DaemonClient+FirstRun.swift,
/// AppModel.swift:4774-5160), driven through the runner so a call that is
/// not made, or made wrong, fails here.
/// </summary>
public sealed class FirstRunDaemonTests
{
    private sealed class Wire
    {
        public readonly List<(string Method, JsonNode? Params)> Sent = new();
        public Func<string, string> Reply = _ => "{\"result\":{}}";

        public Task<DaemonResponse> Call(string method, string json)
        {
            Sent.Add((method, JsonNode.Parse(json)));
            return Task.FromResult(DaemonResponse.Parse(Reply(method)));
        }
    }

    private static (FirstRunDaemon Daemon, Wire Wire, List<string> Local) Make(bool running = true)
    {
        var wire = new Wire();
        var local = new List<string>();
        var daemon = new FirstRunDaemon(wire.Call, json => { local.Add("start " + json); return Task.FromResult(true); }, () => running,
            () => { local.Add("mark"); return Task.FromResult(true); }, () => { local.Add("mark-watch"); return Task.FromResult(true); }, _ => local.Add("finished"));
        return (daemon, wire, local);
    }

    [Fact]
    public async Task StartSendsTheMacsCallsInOrder()
    {
        var (daemon, wire, local) = Make();
        wire.Reply = m => m == "grant_automatic_including_existing" ? "{\"result\":{\"granted\":true,\"include_existing\":true}}" : "{\"result\":{}}";
        var state = new FirstRunState { Step = FirstRunStep.Join, Account = AccountAnswer.NearAI, EnrolledInvite = "I", DaemonStarted = true, SignedIn = true };
        state.ToolAnswers["claude-code"] = ToolChoice.Off;
        state.ToolAnswers["codex"] = ToolChoice.Off;
        state.Scopes.Add("research");
        state.Rules["p1"] = ProjectRule.Ignore;
        state.Sharing = SharingPath.Automatic;
        state.GrantReady = true;
        var runner = new FirstRunRunner(state, daemon) { AcceptedTerms = new AcceptedTerms("2026-11-01", "ab", new DateTimeOffset(2026, 11, 19, 1, 2, 3, TimeSpan.FromHours(-8))) };
        await runner.Commit(CommitPoint.Start);
        Assert.Null(runner.Failure);
        Assert.Equal(new[] { "terms_accept", "set_consent_scopes", "set_project_mode", "grant_automatic_including_existing" }, wire.Sent.ConvertAll(s => s.Method));
        Assert.Equal("2026-11-19T09:02:03Z", (string)wire.Sent[0].Params!["accepted_at"]!);
        Assert.Equal("ab", (string)wire.Sent[0].Params!["content_sha256"]!);
        Assert.Equal("[\"research\"]", wire.Sent[1].Params!["scopes"]!.ToJsonString());
        Assert.Equal("ignore", (string)wire.Sent[2].Params!["mode"]!);
        JsonNode grant = wire.Sent[3].Params!;
        Assert.True((bool)grant["confirmed"]!);
        Assert.True((bool)grant["include_existing"]!);
        Assert.True((bool)grant["keep_folder_asks"]!);
        Assert.True(grant.AsObject().ContainsKey("witness_signing_address"), "the witness is sent as null, never left out");
        Assert.Equal(new[] { "mark", "finished" }, local);
    }

    [Fact]
    public async Task AskEachTimeWithdrawsTheGrantAndAGrantWithoutHistoryIsRefused()
    {
        var (daemon, wire, _) = Make();
        Assert.True(await daemon.SetAskEachTime());
        Assert.Equal("withdraw_automatic_grant", wire.Sent[0].Method);
        Assert.True((bool)wire.Sent[0].Params!["return_folders_to_ask"]!);
        wire.Reply = _ => "{\"result\":{\"granted\":true}}";
        Assert.Equal(new GrantAnswer(false, "automatic-existing-traces-unsupported"), await daemon.GrantAutomatic(null));
        wire.Reply = _ => "{\"error\":{\"code\":\"refused\",\"message\":\"automatic-grant-witness-changed\"}}";
        Assert.Equal(new GrantAnswer(false, "automatic-grant-witness-changed"), await daemon.GrantAutomatic("w"));
    }

    [Fact]
    public async Task EnrollSendsNoScopesAndLookupSortsItsFailures()
    {
        var (daemon, wire, _) = Make();
        await daemon.EnrollInvite("INVITE-1");
        Assert.Equal("enroll", wire.Sent[0].Method);
        Assert.False(wire.Sent[0].Params!.AsObject().ContainsKey("scopes"));
        wire.Reply = _ => "{\"result\":{\"valid\":true,\"issuer\":\"x\"}}";
        Assert.IsType<FirstRunLookup.Found>(await daemon.LookupInvite("INVITE-1"));
        Assert.Equal("INVITE-1", (string)wire.Sent[1].Params!["code"]!);
        wire.Reply = _ => "{\"result\":{\"valid\":false}}";
        Assert.Equal(new FirstRunLookup.Refused("invite-invalid"), await daemon.LookupInvite("x"));
        wire.Reply = _ => "{\"error\":{\"code\":\"unavailable\",\"message\":\"invite-lookup-unavailable\"}}";
        Assert.IsType<FirstRunLookup.Unavailable>(await daemon.LookupInvite("x"));
        wire.Reply = _ => "{\"error\":{\"code\":\"unavailable\",\"message\":\"invite-host-not-allowed\"}}";
        Assert.Equal(new FirstRunLookup.Refused("invite-host-not-allowed"), await daemon.LookupInvite("x"));
    }

    [Fact]
    public async Task ARunningDaemonTakesTheDeclarationThroughSetSettings()
    {
        var (daemon, wire, local) = Make(running: true);
        Assert.True(await daemon.StartDaemon("{\"claude_source\":{\"mode\":\"off\"}}"));
        Assert.Equal("set_settings", wire.Sent[0].Method);
        Assert.Empty(local);
        var (stopped, wire2, local2) = Make(running: false);
        Assert.True(await stopped.StartDaemon("{}"));
        Assert.Empty(wire2.Sent);
        Assert.Equal(new[] { "start {}" }, local2);
    }

    [Fact]
    public void TheEstimateRequestAndTheCoresLine()
    {
        Assert.Equal("{\"earns_credit\":true,\"traces\":12}", FirstRunEstimate.Request(12, true));
        Assert.Equal("{\"earns_credit\":false}", FirstRunEstimate.Request(null, false));
        // The core draws no line with nothing found.
        Assert.Null(FirstRunEstimate.Line(null, true));
        Assert.False(string.IsNullOrEmpty(FirstRunEstimate.Line(12, true)));
    }

    [Fact]
    public void TermsDecodeRefusesAnUnknownBlock()
    {
        const string ok = "{\"document\":{\"version\":\"v\",\"effective\":\"e\",\"in_effect\":true,\"title\":\"T\",\"blocks\":[{\"kind\":\"heading\",\"text\":\"H\"},{\"kind\":\"list\",\"items\":[\"a\"]},{\"kind\":\"table\",\"header\":[\"h\"],\"rows\":[[\"r\"]]}]},\"content_sha256\":\"00\"}";
        FetchedTerms terms = Assert.IsType<FetchedTerms>(FetchedTerms.Decode(ok));
        Assert.Equal(3, terms.Blocks.Count);
        Assert.Null(FetchedTerms.Decode(ok.Replace("\"heading\"", "\"poem\"")));
        Assert.Null(FetchedTerms.Decode(ok.Replace("\"content_sha256\":\"00\"", "\"x\":1")));
    }
}
