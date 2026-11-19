using System;
using System.Collections.Generic;
using System.IO;
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
/// <remarks>
/// Every reply is one the daemon sent (checklist B4): the first run's
/// replies recorded by the contributor crate's
/// <c>daemon::first_run_fixtures_tests</c> under tests/fixtures/first-run
/// (its README says how each was made), and the status and
/// set_project_mode replies from the macOS recorded samples.
/// </remarks>
public sealed class FirstRunDaemonTests
{
    /// <summary>One recorded response frame from tests/fixtures/first-run.</summary>
    private static string Recorded(string name) =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "first-run", name + ".json"));

    /// <summary>A macOS recorded sample (a bare result), framed as the daemon frames it.</summary>
    private static JsonObject Sample(string set, string method) => new()
    {
        ["id"] = 1,
        ["result"] = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "RecordedSamples", set, method + ".json"))),
    };

    /// <summary>The sign-in attempt and page the recorded start reply names.</summary>
    private static readonly JsonNode Started = JsonNode.Parse(Recorded("near_ai_credential_start"))!["result"]!;
    private static readonly string AttemptId = (string)Started["attempt_id"]!;
    private static readonly string BrowserUrl = new Uri((string)Started["browser_url"]!).ToString();

    /// <summary>The account the recorded status sample is settled on.</summary>
    private static readonly string RecordedScope = (string)Sample("normalDay", "status")["result"]!["account_scope"]!;

    /// <summary>
    /// A second account. Every recorded status sample is settled on the same
    /// account, so this one is the recorded status with only its
    /// <c>account_scope</c> edited.
    /// </summary>
    private static readonly string OtherScope = "sha256:" + new string('1', 64);

    /// <summary>
    /// The recorded status (normalDay) answering <paramref name="scope"/>:
    /// only <c>account_scope</c> is edited, to another account or to null,
    /// which is what the daemon sends with no account settled.
    /// </summary>
    private static string Status(string? scope)
    {
        JsonObject frame = Sample("normalDay", "status");
        frame["result"]!["account_scope"] = scope;
        return frame.ToJsonString();
    }

    /// <summary>The recorded reply each method gets unless a test says otherwise.</summary>
    private static string DefaultReply(string method) => method switch
    {
        "terms_accept" => Recorded("terms_accept"),
        "set_consent_scopes" => Recorded("set_consent_scopes"),
        "set_project_mode" => Sample("shared", "set_project_mode").ToJsonString(),
        "set_settings" => Recorded("set_settings"),
        "enroll" => Recorded("enroll"),
        "invite_lookup" => Recorded("invite_lookup_valid"),
        "grant_automatic_including_existing" => Recorded("grant_automatic_including_existing"),
        "withdraw_automatic_grant" => Recorded("withdraw_automatic_grant"),
        "near_ai_credential_start" => Recorded("near_ai_credential_start"),
        "near_ai_credential_status" => Recorded("near_ai_credential_status_absent"),
        "near_ai_credential_cancel" => Recorded("near_ai_credential_cancel"),
        // Not an exception the daemon client swallows: a call with no
        // recorded reply fails the test rather than reading as a refusal.
        _ => throw new NotSupportedException("no recorded reply for " + method),
    };

    private sealed class Wire
    {
        public readonly List<(string Method, JsonNode? Params)> Sent = new();
        public Func<string, string> Reply = DefaultReply;

        /// <summary>
        /// The account each <c>status</c> read answers, in turn (the last
        /// repeats); null answers none. Status reads are counted apart from
        /// <see cref="Sent"/>, which keeps the calls the plan makes.
        /// </summary>
        public readonly List<string?> Scopes = new() { RecordedScope };
        public int StatusReads;

        public Task<DaemonResponse> Call(string method, string json)
        {
            if (method == "status")
            {
                string? scope = Scopes[Math.Min(StatusReads++, Scopes.Count - 1)];
                return Task.FromResult(DaemonResponse.Parse(Status(scope)));
            }

            Sent.Add((method, JsonNode.Parse(json)));
            return Task.FromResult(DaemonResponse.Parse(Reply(method)));
        }
    }

    private static bool OpensBrowser = true;

    private static (FirstRunDaemon Daemon, Wire Wire, List<string> Local) Make(bool running = true)
    {
        var wire = new Wire();
        var local = new List<string>();
        var daemon = new FirstRunDaemon(wire.Call, json => { local.Add("start " + json); return Task.FromResult(true); }, () => running,
            () => { local.Add("mark"); return Task.FromResult(true); }, () => { local.Add("mark-watch"); return Task.FromResult(true); }, _ => local.Add("finished"),
            uri => { local.Add("open " + uri); return Task.FromResult(OpensBrowser); });
        return (daemon, wire, local);
    }

    [Fact]
    public async Task StartSendsTheMacsCallsInOrder()
    {
        var (daemon, wire, local) = Make();
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
        // A granted answer that leaves the traces already on disk out: the
        // daemon's from-now grant (README: grant_automatic_from_now.json).
        wire.Reply = _ => Recorded("grant_automatic_from_now");
        Assert.Equal(new GrantAnswer(false, "automatic-existing-traces-unsupported"), await daemon.GrantAutomatic(null));
        wire.Reply = _ => Recorded("grant_automatic_including_existing_witness_changed");
        Assert.Equal(new GrantAnswer(false, "automatic-grant-witness-changed"), await daemon.GrantAutomatic("w"));
    }

    [Fact]
    public async Task EnrollSendsNoScopesAndLookupSortsItsFailures()
    {
        var (daemon, wire, _) = Make();
        await daemon.EnrollInvite("INVITE-1");
        Assert.Equal("enroll", wire.Sent[0].Method);
        Assert.False(wire.Sent[0].Params!.AsObject().ContainsKey("scopes"));
        wire.Reply = _ => Recorded("invite_lookup_valid");
        Assert.IsType<FirstRunLookup.Found>(await daemon.LookupInvite("INVITE-1"));
        Assert.Equal("INVITE-1", (string)wire.Sent[1].Params!["code"]!);
        // A code that is not an invite is refused with the daemon's label.
        wire.Reply = _ => Recorded("invite_lookup_malformed");
        Assert.Equal(new FirstRunLookup.Refused("invite-invalid"), await daemon.LookupInvite("x"));
        // An invite the issuer does not know is refused with the issuer's
        // reason; the daemon never answers valid: false without one.
        wire.Reply = _ => Recorded("invite_lookup_not_found");
        Assert.Equal(new FirstRunLookup.Refused("not_found"), await daemon.LookupInvite("x"));
        wire.Reply = _ => Recorded("invite_lookup_unavailable");
        Assert.IsType<FirstRunLookup.Unavailable>(await daemon.LookupInvite("x"));
        wire.Reply = _ => Recorded("invite_lookup_host_not_allowed");
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

    [Fact]
    public async Task TheBrowserSignInOpensTheDaemonsPageAndPollsItsAttempt()
    {
        // AppModel.swift:4863-4935: a kept session first; else start, open
        // browser_url, poll near_ai_credential_status by attempt_id until
        // session_state is present.
        OpensBrowser = true;
        var (daemon, wire, local) = Make();
        int polls = 0;
        wire.Reply = m => m == "near_ai_credential_status"
            ? ++polls < 3
                ? polls == 1 ? Recorded("near_ai_credential_status_absent") : Recorded("near_ai_credential_status_waiting")
                : Recorded("near_ai_credential_status_present")
            : DefaultReply(m);
        Assert.Equal(NearAILoginOutcome.SignedIn, await daemon.NearAILoginOutcome(default));
        Assert.Equal(new[] { "open " + BrowserUrl }, local);
        Assert.Equal("near_ai_credential_status", wire.Sent[0].Method);
        Assert.Equal("near_ai_credential_start", wire.Sent[1].Method);
        Assert.Equal(AttemptId, (string)wire.Sent[2].Params!["attempt_id"]!);
    }

    [Fact]
    public async Task APageThatDoesNotOpenCancelsTheAttempt()
    {
        OpensBrowser = false;
        try
        {
            var (daemon, wire, _) = Make();
            Assert.Equal(NearAILoginOutcome.Failed, await daemon.NearAILoginOutcome(default));
            Assert.Equal("near_ai_credential_cancel", wire.Sent[^1].Method);
            Assert.Equal(AttemptId, (string)wire.Sent[^1].Params!["attempt_id"]!);
        }
        finally
        {
            OpensBrowser = true;
        }
    }

    [Fact]
    public async Task AGrantChangeWaitsForASettledAccount()
    {
        // AppModel.swift:4701-4704, :4994-5000, :5062-5070: no settled
        // account sends nothing; an account that changed while the call ran
        // refuses its answer.
        var (daemon, wire, _) = Make();
        wire.Scopes[0] = null;
        Assert.False(await daemon.SetAskEachTime());
        Assert.Equal(new GrantAnswer(false, "automatic-grant-unavailable"), await daemon.GrantAutomatic(null));
        Assert.Empty(wire.Sent);

        (daemon, wire, _) = Make();
        wire.Scopes.Clear();
        wire.Scopes.AddRange(new[] { RecordedScope, OtherScope });
        Assert.Equal(new GrantAnswer(false, "automatic-grant-unavailable"), await daemon.GrantAutomatic(null));
        Assert.Equal("grant_automatic_including_existing", Assert.Single(wire.Sent).Method);

        (daemon, wire, _) = Make();
        Assert.Equal(new GrantAnswer(true, null), await daemon.GrantAutomatic(null));
        Assert.Equal(2, wire.StatusReads);
    }

    [Fact]
    public async Task CancelNamesTheAttemptItsSignInStarted()
    {
        // AppModel.swift: cancel names firstRunLoginAttemptID.
        OpensBrowser = true;
        var (daemon, wire, _) = Make();
        wire.Reply = m => m == "near_ai_credential_status" && wire.Sent.Exists(s => s.Method == "near_ai_credential_start")
            ? Recorded("near_ai_credential_status_waiting")
            : DefaultReply(m);
        using var stop = new System.Threading.CancellationTokenSource();
        Task<NearAILoginOutcome> login = daemon.NearAILoginOutcome(stop.Token);
        while (!wire.Sent.Exists(s => s.Method == "near_ai_credential_start"))
        {
            await Task.Yield();
        }

        Assert.True(await daemon.CancelNearAILogin());
        stop.Cancel();
        await login;
        (string method, JsonNode? args) = wire.Sent.Find(s => s.Method == "near_ai_credential_cancel");
        Assert.Equal("near_ai_credential_cancel", method);
        Assert.Equal(AttemptId, (string)args!["attempt_id"]!);
    }

    [Fact]
    public async Task ACancelTheDaemonRefusesDoesNotEscapeTheSignIn()
    {
        OpensBrowser = false;
        try
        {
            var daemon = new FirstRunDaemon(
                (m, _) => m == "near_ai_credential_cancel"
                    ? throw new TcException("gone")
                    : Task.FromResult(DaemonResponse.Parse(DefaultReply(m))),
                _ => Task.FromResult(true), () => true, () => Task.FromResult(true), () => Task.FromResult(true), _ => { }, _ => Task.FromResult(false));
            Assert.Equal(NearAILoginOutcome.Failed, await daemon.NearAILoginOutcome(default));
        }
        finally
        {
            OpensBrowser = true;
        }
    }
}
