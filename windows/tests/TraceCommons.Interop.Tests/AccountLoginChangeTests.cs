using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The change of login's pure logic (F10), ported from macOS's
/// <c>AccountLoginChangeSurfaceTests</c>. The words are placeholders ("T:"
/// prefixed) so a test passes only by reading the payload;
/// <see cref="TheLiveExportCarriesExactlyTheConsumedFields"/> checks the live
/// export's keys.
/// </summary>
public sealed class AccountLoginChangeSurfaceTests
{
    internal static string CopyJson(IReadOnlyDictionary<string, string?>? overrides = null, string? remove = null)
    {
        var map = new Dictionary<string, string?>();
        foreach (string field in AccountChangeCopy.ConsumedFields)
        {
            map[field] = "T:" + field;
        }

        map["body"] = "T:body {old_provider}>{new_provider}";
        map["body_old_unknown"] = "T:body_old_unknown >{new_provider}";
        map["body_wallet"] = "T:body_wallet >{new_provider}";
        map["private_ai"] = "T:private_ai {new_provider}";
        map["error_current_login"] = "T:error_current_login {old_provider}";
        map["step_current_accessibility_name"] = "T:a11y_current {old_provider}";
        map["confirm_accessibility_name"] = "T:a11y_confirm {new_provider}";
        map["provider_near"] = "NEAR wallet";
        map["provider_github"] = "GitHub";
        map["provider_google"] = "Google";
        if (overrides is not null)
        {
            foreach (var (key, value) in overrides)
            {
                map[key] = value;
            }
        }

        if (remove is not null)
        {
            map.Remove(remove);
        }

        return JsonSerializer.Serialize(map);
    }

    private readonly AccountChangeCopy _copy = AccountChangeCopy.Decode(CopyJson())!;

    private static AccountLoginChangeState State(
        AccountLoginChangeStep step,
        string kind = "near_ai_login",
        string? old = "github",
        string? @new = "google",
        bool busy = false,
        string? error = null) =>
        new("c1", step, kind, old, @new, busy, error, null);

    [Fact]
    public void APayloadWithAMissingOrBlankFieldIsRefusedWhole()
    {
        Assert.NotNull(AccountChangeCopy.Decode(CopyJson()));
        Assert.Null(AccountChangeCopy.Decode(CopyJson(new Dictionary<string, string?> { ["title"] = " " })));
        Assert.Null(AccountChangeCopy.Decode(CopyJson(new Dictionary<string, string?> { ["title"] = null })));
        Assert.Null(AccountChangeCopy.Decode(CopyJson(remove: "mismatch_change")));
        Assert.Null(AccountChangeCopy.Decode("not json"));
        Assert.Null(AccountChangeCopy.Decode(null));
        Assert.Null(AccountChangeCopy.Decode("[]"));
    }

    [Fact]
    public void EveryConsumedFieldIsASentenceOfTheRecord()
    {
        Assert.Equal(AccountChangeCopy.ConsumedFields.Count, _copy.Sentences.Count);
        Assert.Equal(
            AccountChangeCopy.ConsumedFields.Count,
            AccountChangeCopy.ConsumedFields.Distinct().Count());
        Assert.Equal("T:mismatch_body", _copy.MismatchBody);
        Assert.Equal("T:payouts_needs_login_add", _copy.PayoutsNeedsLoginAdd);
        Assert.Equal("T:error_retired", _copy.ErrorRetired);
    }

    [Fact]
    public void TemplatesNameLoginsByTitleAndNeverByTag()
    {
        Assert.Equal("T:body NEAR wallet>GitHub", _copy.Render(_copy.Body, old: "near", @new: "github"));
        Assert.Null(_copy.Render(_copy.Body, old: null, @new: "github"));
        Assert.Null(_copy.Render(_copy.Body, old: "myspace", @new: "github"));
        Assert.Equal("T:title", _copy.Render(_copy.Title));
        Assert.Equal("Google", _copy.ProviderTitle("google"));
        Assert.Null(_copy.ProviderTitle(null));
    }

    [Fact]
    public void StepOneOfANearAiAccountNamesTheLoginUsedLastTime()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Current), _copy)!;
        Assert.Equal(AccountLoginChangeAction.ProveCurrent, page.Action);
        Assert.Equal("T:step_current", page.Heading);
        // The button has its own key, apart from the heading (rule 4).
        Assert.Equal("T:step_current_button", page.Primary);
        Assert.Equal("T:a11y_current GitHub", page.PrimaryAccessibilityName);
        Assert.False(page.AsksWalletAccount);
        Assert.Empty(page.Lines);
        Assert.Equal("T:cancel", page.Cancel);
        Assert.Equal("T:title", page.Title);
    }

    [Fact]
    public void StepOneOfAWalletAccountAsksForTheNameAndTheWallet()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(
            State(AccountLoginChangeStep.Current, kind: "wallet", old: null), _copy)!;
        Assert.True(page.AsksWalletAccount);
        Assert.Equal(new[] { "T:step_current_wallet" }, page.Lines);
        Assert.Equal("T:step_current", page.PrimaryAccessibilityName);
    }

    [Fact]
    public void StepTwoSignsInWithTheNewAccount()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(
            State(AccountLoginChangeStep.New, @new: null), _copy)!;
        Assert.Equal(AccountLoginChangeAction.SignInNew, page.Action);
        Assert.Equal("T:step_new", page.Heading);
        Assert.Equal("T:step_new_button", page.Primary);
        Assert.Equal("T:step_new_accessibility_name", page.PrimaryAccessibilityName);
    }

    [Fact]
    public void StepThreeSaysWhatStaysAndWhatTheNewLoginIs()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Confirm), _copy)!;
        Assert.Equal(AccountLoginChangeAction.Confirm, page.Action);
        Assert.Equal(new[] { "T:body GitHub>Google", "T:private_ai Google" }, page.Lines);
        Assert.Equal("T:confirm", page.Primary);
        Assert.Equal("T:a11y_confirm Google", page.PrimaryAccessibilityName);
        Assert.Null(page.Heading);
        // Switch moves the account: it is never one Enter away.
        Assert.False(page.IsDefault);
    }

    [Fact]
    public void OnlySwitchIsNotTheDefaultButton()
    {
        Assert.True(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Current), _copy)!.IsDefault);
        Assert.True(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.New), _copy)!.IsDefault);
        Assert.False(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Confirm), _copy)!.IsDefault);
    }

    [Fact]
    public void StepThreeWithAnUnknownOldLoginUsesTheBodyWithoutIt()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(
            State(AccountLoginChangeStep.Confirm, old: null), _copy)!;
        Assert.Equal("T:body_old_unknown >Google", page.Lines.First());
    }

    [Fact]
    public void StepThreeOfAWalletAccountUsesTheWalletBody()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(
            State(AccountLoginChangeStep.Confirm, kind: "wallet", old: null), _copy)!;
        Assert.Equal("T:body_wallet >Google", page.Lines.First());
    }

    [Fact]
    public void StepThreeWithoutANewLoginItCanNameIsNotDrawn()
    {
        Assert.Null(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Confirm, @new: null), _copy));
        Assert.Null(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Confirm, @new: "myspace"), _copy));
    }

    [Fact]
    public void EachErrorIsItsApprovedSentence()
    {
        string? Line(string error, string? old = "github") =>
            AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Current, old: old, error: error), _copy)?.Error;

        Assert.Equal("T:error_current_login GitHub", Line("current_login"));
        Assert.Equal("T:error_unavailable", Line("current_login", old: null));
        Assert.Equal("T:error_wallet", Line("wallet"));
        Assert.Equal("T:error_in_use", Line("in_use"));
        Assert.Equal("T:error_too_soon", Line("too_soon"));
        Assert.Equal("T:error_unavailable", Line("unavailable"));
        Assert.Equal("T:error_unavailable", Line("something_new"));
        Assert.Null(AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Current), _copy)?.Error);
    }

    [Fact]
    public void DoneClosesWithNoConfirmationOfItsOwn()
    {
        AccountLoginChangePage page = AccountLoginChangeSurface.Page(State(AccountLoginChangeStep.Done), _copy)!;
        Assert.Equal(AccountLoginChangeAction.Close, page.Action);
        Assert.Empty(page.Lines);
        Assert.Null(page.Error);
        Assert.False(page.Busy);
    }

    [Fact]
    public void ThePayoutsLineShowsOnlyWhereTheChangeIsOffered()
    {
        Assert.True(new AccountLoginChangeStatus(available: true, payoutsNeedLogin: true, held: false).ShowsPayoutsLine);
        Assert.False(new AccountLoginChangeStatus(available: false, payoutsNeedLogin: true, held: false).ShowsPayoutsLine);
        Assert.False(AccountLoginChangeStatus.None.ShowsPayoutsLine);
    }

    [Fact]
    public void TheMismatchShowsAloneAndTheChangeOnlyWithAHeldLogin()
    {
        var refused = new AccountLoginChangeStatus(
            available: false, payoutsNeedLogin: false, held: false, mismatch: true);
        Assert.True(refused.Mismatch);
        Assert.False(refused.OffersChange);
        var held = new AccountLoginChangeStatus(available: true, payoutsNeedLogin: false, held: true);
        Assert.True(held.Mismatch);
        Assert.True(held.OffersChange);
    }

    [Fact]
    public void TheChangeButtonShowsOnlyWhenNothingElseOffersIt()
    {
        Assert.True(new AccountLoginChangeStatus(true, false, false).ShowsChangeButton);
        Assert.False(new AccountLoginChangeStatus(true, false, true).ShowsChangeButton);
        Assert.False(new AccountLoginChangeStatus(true, true, false).ShowsChangeButton);
        Assert.False(new AccountLoginChangeStatus(false, false, false).ShowsChangeButton);
    }

    [Fact]
    public void TheStatusKeysAreReadAndAnOlderDaemonOffersNothing()
    {
        using JsonDocument full = JsonDocument.Parse(
            "{\"logged_in\":true,\"login_change_available\":true,\"payouts_need_login\":false,"
            + "\"login_change_held\":true,\"login_mismatch\":false}");
        AccountLoginChangeStatus read = AccountLoginChangeStatus.From(full.RootElement);
        Assert.True(read.Available);
        Assert.False(read.PayoutsNeedLogin);
        Assert.True(read.Held);
        Assert.True(read.Mismatch, "a held login is a mismatch");

        using JsonDocument old = JsonDocument.Parse("{\"logged_in\":true}");
        Assert.Equal(AccountLoginChangeStatus.None, AccountLoginChangeStatus.From(old.RootElement));
        using JsonDocument odd = JsonDocument.Parse("{\"login_change_available\":\"yes\"}");
        Assert.False(AccountLoginChangeStatus.From(odd.RootElement).Available);
        using JsonDocument array = JsonDocument.Parse("[]");
        Assert.Equal(AccountLoginChangeStatus.None, AccountLoginChangeStatus.From(array.RootElement));
    }

    [Fact]
    public void TheMismatchLineIsTheApprovedBodyOrNothing()
    {
        var mismatch = new AccountLoginChangeStatus(false, false, false, mismatch: true);
        Assert.Equal("T:mismatch_body", AccountLoginChangeSurface.MismatchLine(mismatch, _copy));
        Assert.Null(AccountLoginChangeSurface.MismatchLine(AccountLoginChangeStatus.None, _copy));
        Assert.Null(AccountLoginChangeSurface.MismatchLine(mismatch, null));
    }

    [Fact]
    public void AStateDecodesFromTheDaemonsAnswer()
    {
        AccountLoginChangeState decoded = AccountLoginChangeState.Parse(
            "{\"change_id\":\"c9\",\"step\":\"confirm\",\"current_kind\":\"wallet\",\"old_provider\":null,"
            + "\"new_provider\":\"github\",\"busy\":false,\"error\":null,"
            + "\"browser_url\":\"http://127.0.0.1:1/x\",\"future\":1}")!;
        Assert.Equal("c9", decoded.ChangeId);
        Assert.Equal(AccountLoginChangeStep.Confirm, decoded.Step);
        Assert.True(decoded.IsWallet);
        Assert.Null(decoded.OldProvider);
        Assert.Equal("github", decoded.NewProvider);
        Assert.Equal("http://127.0.0.1:1/x", decoded.BrowserUrl);

        Assert.Null(AccountLoginChangeState.Parse("{\"change_id\":\"c9\",\"step\":\"later\",\"current_kind\":\"wallet\"}"));
        Assert.Null(AccountLoginChangeState.Parse("{\"step\":\"new\",\"current_kind\":\"wallet\"}"));
        Assert.Null(AccountLoginChangeState.Parse("nope"));
    }

    /// <summary>
    /// Through the real ABI: the export carries exactly the fields this shell
    /// decodes, and every one of them is a finished sentence.
    /// </summary>
    [Fact]
    public void TheLiveExportCarriesExactlyTheConsumedFields()
    {
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_account_change_copy_json())
            ?? throw new InvalidOperationException("tc_account_change_copy_json returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        var keys = document.RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k, StringComparer.Ordinal);
        Assert.Equal(AccountChangeCopy.ConsumedFields.OrderBy(k => k, StringComparer.Ordinal), keys);

        AccountChangeCopy copy = Assert.IsType<AccountChangeCopy>(AccountChangeCopy.Copy());
        // Owner changed this 2026-11-04 (Settings copy pass 2, P53).
        Assert.Equal("Switch account", copy.Change);
        Assert.Equal("Switch", copy.Confirm);
        Assert.Equal("Switch to Google", copy.Render(copy.ConfirmAccessibilityName, @new: "google"));
    }
}

/// <summary>The two IPC methods: the names and parameters the daemon reads.</summary>
public sealed class AccountLoginChangeClientTests
{
    internal sealed class Fake
    {
        public List<(string Method, JsonElement Params)> Calls { get; } = new();
        public Queue<string> Replies { get; } = new();

        public string Default { get; set; } =
            "{\"id\":1,\"result\":{\"change_id\":\"c1\",\"step\":\"current\",\"current_kind\":\"near_ai_login\",\"busy\":false}}";

        public Task<DaemonResponse> Call(string method, string paramsJson)
        {
            Calls.Add((method, JsonDocument.Parse(paramsJson).RootElement.Clone()));
            return Task.FromResult(DaemonResponse.Parse(Replies.Count > 0 ? Replies.Dequeue() : Default));
        }

        public IEnumerable<string?> Steps => Calls.Select(c =>
            c.Params.TryGetProperty("step", out JsonElement s) ? s.GetString() : null);
    }

    [Fact]
    public async Task EveryStepSendsWhatTheDaemonReads()
    {
        var fake = new Fake();
        var client = new AccountLoginChangeClient(fake.Call);
        await client.StartAsync();
        await client.ProveCurrentAsync("c1", "alice.near");
        await client.ProveCurrentAsync("c1", null);
        await client.SignInNewAsync("c1");
        await client.ConfirmAsync("c1");
        await client.StatusAsync("c1");
        await client.CancelAsync("c1");
        await client.CancelAsync(null);

        Assert.Equal(
            Enumerable.Repeat("account_login_change", 6)
                .Concat(new[] { "account_login_change_cancel", "account_login_change_cancel" }),
            fake.Calls.Select(c => c.Method));
        Assert.Equal(
            new string?[] { "start", "prove_current", "prove_current", "sign_in_new", "confirm", "status", null, null },
            fake.Steps);
        Assert.False(fake.Calls[0].Params.TryGetProperty("change_id", out _));
        Assert.Equal("alice.near", fake.Calls[1].Params.GetProperty("wallet_account_id").GetString());
        Assert.False(fake.Calls[2].Params.TryGetProperty("wallet_account_id", out _));
        Assert.True(fake.Calls[4].Params.GetProperty("confirmed").GetBoolean());
        Assert.False(fake.Calls[3].Params.TryGetProperty("confirmed", out _), "only Switch confirms");
        Assert.Equal("c1", fake.Calls[5].Params.GetProperty("change_id").GetString());
        Assert.Equal("c1", fake.Calls[6].Params.GetProperty("change_id").GetString());
        Assert.False(fake.Calls[7].Params.TryGetProperty("change_id", out _));
        Assert.Equal(DaemonProtocol.Methods.AccountLoginChange, "account_login_change");
        Assert.Equal(DaemonProtocol.Methods.AccountLoginChangeCancel, "account_login_change_cancel");
    }

    [Fact]
    public async Task ARefusalCarriesItsLabel()
    {
        var fake = new Fake();
        fake.Replies.Enqueue("{\"id\":1,\"error\":{\"code\":\"bad_params\",\"message\":\"account_login_change_not_available\"}}");
        var refused = await Assert.ThrowsAsync<AccountLoginChangeRefusedException>(
            () => new AccountLoginChangeClient(fake.Call).StartAsync());
        Assert.Equal("account_login_change_not_available", refused.Label);

        fake.Replies.Enqueue("{\"id\":1,\"result\":{\"step\":\"current\"}}");
        var unparsed = await Assert.ThrowsAsync<AccountLoginChangeRefusedException>(
            () => new AccountLoginChangeClient(fake.Call).StartAsync());
        Assert.Equal("unparseable-response", unparsed.Label);
    }
}

/// <summary>
/// The sheet's driver, ported from macOS's <c>AccountLoginChangeModel</c>:
/// opens the browser for each leg, asks how it went until it ends, and
/// confirms only from Switch.
/// </summary>
public sealed class AccountLoginChangeFlowTests
{
    private static string Answer(string step, bool busy = false, string? url = null, string kind = "near_ai_login") =>
        JsonSerializer.Serialize(new Dictionary<string, object?>
        {
            ["id"] = 1,
            ["result"] = new Dictionary<string, object?>
            {
                ["change_id"] = "c1",
                ["step"] = step,
                ["current_kind"] = kind,
                ["old_provider"] = "github",
                ["new_provider"] = "google",
                ["busy"] = busy,
                ["error"] = null,
                ["browser_url"] = url,
            },
        });

    private static AccountChangeCopy Copy() => AccountChangeCopy.Decode(AccountLoginChangeSurfaceTests.CopyJson())!;

    private sealed class Harness
    {
        public AccountLoginChangeClientTests.Fake Fake { get; } = new();
        public List<Uri> Opened { get; } = new();
        public int Delays { get; private set; }
        public AccountLoginChangeFlow Flow { get; }

        public Harness(int pollLimit = 360)
        {
            Flow = new AccountLoginChangeFlow(
                Fake.Call,
                uri =>
                {
                    Opened.Add(uri);
                    return Task.FromResult(true);
                },
                Copy(),
                (_, _) =>
                {
                    Delays++;
                    return Task.CompletedTask;
                },
                TimeSpan.FromSeconds(1),
                TimeSpan.FromSeconds(pollLimit));
        }
    }

    [Fact]
    public async Task BeginOpensTheSheetAndARefusalShowsTheGenericLine()
    {
        var h = new Harness();
        h.Fake.Replies.Enqueue("{\"id\":1,\"error\":{\"code\":\"bad_params\",\"message\":\"account_login_change_not_available\"}}");
        await h.Flow.BeginAsync();
        Assert.False(h.Flow.Presented);
        Assert.True(h.Flow.StartFailed);
        Assert.Equal("T:error_unavailable", h.Flow.StartFailedLine);

        h.Fake.Replies.Enqueue(Answer("current"));
        await h.Flow.BeginAsync();
        Assert.True(h.Flow.Presented);
        Assert.False(h.Flow.StartFailed);
        Assert.Null(h.Flow.StartFailedLine);
        Assert.Equal(AccountLoginChangeAction.ProveCurrent, h.Flow.Page!.Action);
    }

    [Fact]
    public async Task ABrowserLegOpensTheBrowserAndAsksUntilItEnds()
    {
        var h = new Harness();
        h.Fake.Replies.Enqueue(Answer("current"));
        await h.Flow.BeginAsync();

        h.Fake.Replies.Enqueue(Answer("current", busy: true, url: "http://127.0.0.1:9/leg"));
        h.Fake.Replies.Enqueue(Answer("current", busy: true));
        h.Fake.Replies.Enqueue(Answer("new"));
        await h.Flow.PrimaryAsync();

        Assert.Equal(new[] { new Uri("http://127.0.0.1:9/leg") }, h.Opened);
        Assert.Equal(new string?[] { "start", "prove_current", "status", "status" }, h.Fake.Steps);
        Assert.Equal(2, h.Delays);
        Assert.Equal(AccountLoginChangeStep.New, h.Flow.State!.Step);
        Assert.False(h.Flow.Calling);
    }

    [Fact]
    public async Task AWalletAccountSendsTheTypedNameAndNeverAnEmptyOne()
    {
        var h = new Harness();
        h.Fake.Replies.Enqueue(Answer("current", kind: "wallet"));
        await h.Flow.BeginAsync();

        h.Flow.WalletAccount = "   ";
        await h.Flow.PrimaryAsync();
        Assert.Single(h.Fake.Calls);
        Assert.False(h.Flow.CanPressPrimary);

        h.Flow.WalletAccount = " alice.near ";
        Assert.True(h.Flow.CanPressPrimary);
        h.Fake.Replies.Enqueue(Answer("new"));
        await h.Flow.PrimaryAsync();
        Assert.Equal("alice.near", h.Fake.Calls[1].Params.GetProperty("wallet_account_id").GetString());
    }

    [Fact]
    public async Task SwitchConfirmsThenTheSheetClosesAndTheAccountIsReRead()
    {
        var h = new Harness();
        int finished = 0;
        h.Flow.Finished += (_, _) => finished++;
        h.Fake.Replies.Enqueue(Answer("confirm"));
        await h.Flow.BeginAsync();

        h.Fake.Replies.Enqueue(Answer("done"));
        await h.Flow.PrimaryAsync();

        Assert.Equal(new string?[] { "start", "confirm" }, h.Fake.Steps);
        Assert.True(h.Fake.Calls[1].Params.GetProperty("confirmed").GetBoolean());
        Assert.False(h.Flow.Presented);
        Assert.Null(h.Flow.State);
        Assert.Equal(1, finished);
    }

    [Fact]
    public async Task ARefusedStepReReadsTheCeremony()
    {
        var h = new Harness();
        h.Fake.Replies.Enqueue(Answer("confirm"));
        await h.Flow.BeginAsync();

        h.Fake.Replies.Enqueue("{\"id\":1,\"error\":{\"code\":\"bad_params\",\"message\":\"account_login_change_out_of_order\"}}");
        h.Fake.Replies.Enqueue(Answer("new"));
        await h.Flow.PrimaryAsync();
        Assert.Equal(new string?[] { "start", "confirm", "status" }, h.Fake.Steps);
        Assert.Equal(AccountLoginChangeStep.New, h.Flow.State!.Step);
        Assert.True(h.Flow.Presented);
    }

    [Fact]
    public async Task AskingStopsAtTheLimit()
    {
        var h = new Harness(pollLimit: 3);
        h.Fake.Replies.Enqueue(Answer("current"));
        await h.Flow.BeginAsync();
        h.Fake.Default = Answer("current", busy: true);
        await h.Flow.PrimaryAsync();
        Assert.Equal(3, h.Delays);
    }

    [Fact]
    public async Task CancelDropsTheCeremonyAndClosesTheSheet()
    {
        var h = new Harness();
        h.Fake.Replies.Enqueue(Answer("new"));
        await h.Flow.BeginAsync();
        await h.Flow.CancelAsync();
        Assert.Equal("account_login_change_cancel", h.Fake.Calls.Last().Method);
        Assert.Equal("c1", h.Fake.Calls.Last().Params.GetProperty("change_id").GetString());
        Assert.False(h.Flow.Presented);
        Assert.Null(h.Flow.Page);
    }

    [Fact]
    public async Task ACancelDuringALegStopsTheAsking()
    {
        var h = new Harness();
        h.Fake.Default = Answer("current", busy: true);
        var gate = new TaskCompletionSource();
        var paused = new AccountLoginChangeFlow(
            h.Fake.Call,
            _ => Task.FromResult(true),
            Copy(),
            async (_, _) => await gate.Task,
            TimeSpan.FromSeconds(1),
            TimeSpan.FromSeconds(360));
        h.Fake.Replies.Enqueue(Answer("current"));
        await paused.BeginAsync();
        h.Fake.Replies.Enqueue(Answer("current", busy: true, url: "http://127.0.0.1:9/leg"));
        Task leg = paused.PrimaryAsync();
        int before = h.Fake.Calls.Count;
        await paused.CancelAsync();
        gate.SetResult();
        await leg;
        // The cancel was the last call: nothing asked after it.
        Assert.Equal(before + 1, h.Fake.Calls.Count);
        Assert.Equal("account_login_change_cancel", h.Fake.Calls.Last().Method);
        Assert.False(paused.Presented);
    }

    [Fact]
    public async Task NothingIsStartedWithoutTheWords()
    {
        var fake = new AccountLoginChangeClientTests.Fake();
        var flow = new AccountLoginChangeFlow(fake.Call, _ => Task.FromResult(true), null);
        await flow.BeginAsync();
        Assert.Empty(fake.Calls);
        Assert.False(flow.Presented);
    }
}
