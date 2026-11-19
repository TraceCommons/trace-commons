using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The spending-organization prompt (spec W19-D2 (a)). The cases are the
/// Mac's own: macos/Tests/TCShellCoreTests/SpendOrganizationSurfaceTests.swift
/// and macos/Tests/TraceCommonsAppTests/SpendOrganizationTests.swift:145-187.
/// </summary>
public sealed class SpendOrganizationsTests
{
    private static JsonObject View() => new()
    {
        ["title"] = "TITLE", ["what"] = "WHAT", ["picker"] = "PICKER", ["switching"] = "SWITCHING", ["failed"] = "FAILED",
        ["session_expired"] = "EXPIRED", ["refused"] = "REFUSED", ["revoke_pending"] = "PENDING",
        ["prompt_title"] = "PROMPT-TITLE", ["prompt_line"] = "PROMPT-LINE",
        ["prompt_action"] = "PROMPT-ACTION", ["prompt_action_accessibility"] = "PROMPT-ACTION-AX",
        ["prompt_dismiss"] = "PROMPT-DISMISS", ["prompt_dismiss_accessibility"] = "PROMPT-DISMISS-AX",
    };

    private static string Payload(
        (string Id, bool Current)[] organizations, string state = "listed", string? current = "org-a",
        bool @explicit = false, bool? prompt = null, bool revokePending = false, JsonObject? view = null, bool noView = false)
    {
        var rows = new JsonArray(organizations.Select(o => (JsonNode)new JsonObject
        {
            ["id"] = o.Id, ["name"] = "Name " + o.Id, ["role"] = "owner", ["current"] = o.Current,
        }).ToArray());
        var obj = new JsonObject
        {
            ["state"] = state, ["organizations"] = rows, ["current_organization_id"] = current, ["explicit"] = @explicit,
            ["multiple"] = organizations.Length >= 2,
            ["prompt"] = prompt ?? (organizations.Length >= 2 && !@explicit && current != null),
            ["revoke_pending"] = revokePending,
        };
        if (!noView) obj["view"] = view ?? View();
        return obj.ToJsonString();
    }

    private static SpendOrganizations Parse(string json) => SpendOrganizations.Parse(JsonDocument.Parse(json).RootElement);

    [Fact]
    public void ASingleOrganizationDrawsNeitherPickerNorPrompt()
    {
        SpendOrganizations status = Parse(Payload(new[] { ("org-a", true) }));
        Assert.Equal("listed", status.State);
        Assert.False(status.ShowsPicker);
        Assert.False(status.ShowsPrompt);
    }

    [Fact]
    public void SeveralOrganizationsWithNoChoiceDrawBoth()
    {
        SpendOrganizations status = Parse(Payload(new[] { ("org-a", true), ("org-b", false) }));
        Assert.True(status.ShowsPicker);
        Assert.True(status.ShowsPrompt);
        Assert.Equal("PROMPT-LINE", status.View?.PromptLine);
        Assert.False(status.WithPromptEnded().ShowsPrompt);
        Assert.True(status.WithPromptEnded().ShowsPicker);
    }

    [Fact]
    public void AnEmptyExplanationStillDecodesTheWords()
    {
        JsonObject view = View();
        view["what"] = "";
        Assert.True(Parse(Payload(new[] { ("org-a", true), ("org-b", false) }, view: view)).ShowsPrompt);
    }

    [Fact]
    public void AnExplicitChoiceKeepsThePickerAndEndsThePrompt()
    {
        SpendOrganizations status = Parse(Payload(new[] { ("org-a", true), ("org-b", false) }, @explicit: true));
        Assert.True(status.ShowsPicker);
        Assert.False(status.ShowsPrompt);
    }

    [Theory]
    [InlineData("unavailable")]
    [InlineData("session_expired")]
    [InlineData("no_session")]
    [InlineData("no_organization")]
    public void AListingThatFailedDrawsNothing(string state)
    {
        SpendOrganizations status = Parse(Payload(Array.Empty<(string, bool)>(), state: state, current: null, prompt: false));
        Assert.False(status.ShowsPicker);
        Assert.False(status.ShowsPrompt);
    }

    [Theory]
    [InlineData("[]")]
    [InlineData("{\"organizations\":[]}")]
    [InlineData("{\"state\":\"listed\",\"organizations\":[{\"id\":\"org-a\"}]}")]
    public void UnreadablePayloadsAreUnreported(string json) => Assert.Equal(SpendOrganizations.Unreported, Parse(json));

    [Fact]
    public void MissingWordsDrawNothing()
    {
        JsonObject partial = View();
        partial.Remove("prompt_line");
        foreach (string json in new[]
        {
            Payload(new[] { ("org-a", true), ("org-b", false) }, noView: true),
            Payload(new[] { ("org-a", true), ("org-b", false) }, view: partial),
        })
        {
            SpendOrganizations status = Parse(json);
            Assert.Null(status.View);
            Assert.False(status.ShowsPrompt);
        }
    }

    [Fact]
    public void NoCurrentOrganizationDrawsNothing() =>
        Assert.False(Parse(Payload(new[] { ("org-a", false), ("org-b", false) }, current: null, prompt: false)).ShowsPrompt);

    [Fact]
    public void FailureLinesByLabel()
    {
        SpendOrganizations status = Parse(Payload(new[] { ("org-a", true), ("org-b", false) }));
        Assert.Equal("EXPIRED", status.FailureLine("near_ai_spend_org_session_expired"));
        Assert.Equal("EXPIRED", status.FailureLine("near_ai_spend_org_no_session"));
        Assert.Equal("REFUSED", status.FailureLine("near_ai_spend_org_refused"));
        Assert.Equal("FAILED", status.FailureLine("near_ai_spend_org_something_new"));
        Assert.Null(status.FailureLine("unparseable-response"));
        Assert.Null(SpendOrganizations.Unreported.FailureLine("near_ai_spend_org_refused"));
    }

    private sealed class Daemon
    {
        public List<(string Method, string Params)> Calls { get; } = new();
        public Queue<string> Listings { get; } = new();
        public string SelectReply = """{"id":1,"result":{"switched":false,"revoke_pending":false}}""";
        public long Connection = 1;

        /// <summary>When set, a select waits for it, so a test can look at the switch in flight.</summary>
        public TaskCompletionSource? SelectGate;

        /// <summary>When set, a select throws, as a transport that gave up waiting does.</summary>
        public bool SelectThrows;

        public async Task<DaemonResponse> Call(string method, string parameters)
        {
            Calls.Add((method, parameters));
            if (method == SpendOrganizationModel.SelectMethod)
            {
                if (SelectGate is { } gate) await gate.Task;
                if (SelectThrows) throw new InvalidOperationException("transport");
                return DaemonResponse.Parse(SelectReply);
            }

            return DaemonResponse.Parse("{\"id\":1,\"result\":" + (Listings.Count > 1 ? Listings.Dequeue() : Listings.Peek()) + "}");
        }

        public IEnumerable<string> Selects => Calls.Where(c => c.Method == SpendOrganizationModel.SelectMethod).Select(c => c.Params);
    }

    [Fact]
    public async Task NotNowRecordsTheCurrentOrganizationAndEndsThePrompt()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }, @explicit: true));
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, new CredentialActivity());
        await model.RefreshAsync();
        Assert.True(model.IsShown);
        Assert.Equal(("PROMPT-TITLE", "PROMPT-LINE", "PROMPT-ACTION", "PROMPT-ACTION-AX", "PROMPT-DISMISS", "PROMPT-DISMISS-AX"),
            (model.Title, model.Line, model.Action, model.ActionName, model.Dismiss, model.DismissName));
        await model.DismissAsync();
        Assert.Equal(new[] { "{\"organization_id\":\"org-a\"}" },
            daemon.Calls.Where(c => c.Method == SpendOrganizationModel.SelectMethod).Select(c => c.Params));
        // The re-read after it is fresh (AppModel.swift:1113).
        Assert.Equal("{\"fresh\":true}", daemon.Calls.Last().Params);
        Assert.False(model.IsShown);
        Assert.True(model.Status.ShowsPicker);
    }

    [Fact]
    public async Task NotNowFailingKeepsThePrompt()
    {
        var daemon = new Daemon { SelectReply = """{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}""" };
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, new CredentialActivity());
        await model.RefreshAsync();
        // The prompt never leaves, not even until the re-read lands.
        var shown = new List<bool>();
        model.PropertyChanged += (_, _) => shown.Add(model.IsShown);
        await model.DismissAsync();
        Assert.True(model.IsShown);
        Assert.All(shown, Assert.True);
    }

    [Fact]
    public async Task NotNowWithoutACurrentOrganizationCallsNothing()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", false), ("org-b", false) }, current: null));
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, new CredentialActivity());
        await model.RefreshAsync();
        await model.DismissAsync();
        Assert.DoesNotContain(daemon.Calls, c => c.Method == SpendOrganizationModel.SelectMethod);
    }

    // A listing read for an earlier connection is not drawn for the new one.
    [Fact]
    public async Task AListingFromAnEarlierConnectionIsDropped()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        var model = new SpendOrganizationModel(
            async (method, parameters) => { DaemonResponse r = await daemon.Call(method, parameters); daemon.Connection++; return r; },
            () => daemon.Connection, new CredentialActivity());
        await model.RefreshAsync();
        Assert.False(model.IsShown);
    }

    // Not now does nothing while a credential action is in flight
    // (AppModel.swift:1104, !credentialBusy).
    [Fact]
    public async Task NotNowWaitsForACredentialAction()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        var credential = new CredentialActivity { Busy = true };
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, credential);
        await model.RefreshAsync();
        await model.DismissAsync();
        Assert.DoesNotContain(daemon.Calls, c => c.Method == SpendOrganizationModel.SelectMethod);
        Assert.True(model.IsShown);
        credential.Busy = false;
        await model.DismissAsync();
        Assert.Contains(daemon.Calls, c => c.Method == SpendOrganizationModel.SelectMethod);
    }

    // ---- The Settings picker (SpendOrganizationCard.swift, AppModel.swift:1075-1096) ----

    private static async Task<(Daemon Daemon, SpendOrganizationModel Model)> Listed(
        CredentialActivity? credential = null, bool revokePending = false)
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }, revokePending: revokePending));
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, credential ?? new CredentialActivity());
        await model.RefreshAsync();
        return (daemon, model);
    }

    // testPickingSendsTheOrganizationAndRereadsTheListing.
    [Fact]
    public async Task PickingSendsTheOrganizationAndRereadsTheListing()
    {
        var (daemon, model) = await Listed();
        daemon.Listings.Clear();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", false), ("org-b", true) }, current: "org-b", @explicit: true));
        await model.SelectAsync("org-b");
        Assert.False(model.Switching);
        Assert.Null(model.Failure);
        Assert.Equal(new[] { "{\"organization_id\":\"org-b\"}" }, daemon.Selects);
        Assert.Equal((SpendOrganizationModel.ListMethod, "{\"fresh\":true}"), daemon.Calls.Last());
        Assert.Equal("org-b", model.Status.CurrentOrganizationId);
        Assert.Equal(1, model.CurrentIndex);
        Assert.False(model.IsShown);
    }

    // testChoosingTheCurrentOrganizationEndsThePromptImmediately: before the
    // re-read lands, even if that re-read still says otherwise.
    [Fact]
    public async Task ChoosingTheCurrentOrganizationEndsThePromptImmediately()
    {
        var (daemon, model) = await Listed();
        Assert.True(model.IsShown);
        // The re-read still asks; the prompt ended before it was sent.
        var settled = new List<(bool Shown, int Calls)>();
        model.PropertyChanged += (_, _) => { if (!model.Switching) settled.Add((model.IsShown, daemon.Calls.Count)); };
        await model.SelectAsync("org-a");
        Assert.Equal((false, 2), settled[0]);
        Assert.True(model.ShowsPicker);
        Assert.Null(model.Failure);
        Assert.Equal((SpendOrganizationModel.ListMethod, "{\"fresh\":true}"), daemon.Calls[2]);
    }

    // testARefusedSwitchSaysSoInTheDaemonsWords.
    [Theory]
    [InlineData("near_ai_spend_org_refused", "REFUSED")]
    [InlineData("near_ai_spend_org_session_expired", "EXPIRED")]
    [InlineData("near_ai_spend_org_no_session", "EXPIRED")]
    [InlineData("near_ai_spend_org_unavailable", "FAILED")]
    public async Task ARefusedSwitchSaysSoInTheDaemonsWords(string label, string line)
    {
        var (daemon, model) = await Listed();
        daemon.SelectReply = "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"" + label + "\"}}";
        await model.SelectAsync("org-b");
        Assert.False(model.Switching);
        Assert.Equal(line, model.Failure);
        Assert.Equal(new SpendOrganizationLine(line, SpendOrganizationTone.Outside), model.StateLine);
        // A refusal changed nothing, so the prompt stays.
        Assert.True(model.IsShown);
    }

    // A failure that is not the daemon's refusal may have switched: no line,
    // and the re-read shows which organization is current
    // (SpendOrganizationSurface.swift failureLine).
    [Fact]
    public async Task ATransportFailureIsNoRefusalLine()
    {
        var (daemon, model) = await Listed();
        daemon.SelectThrows = true;
        await model.SelectAsync("org-b");
        Assert.Null(model.Failure);
        Assert.Null(model.StateLine);
        Assert.Equal((SpendOrganizationModel.ListMethod, "{\"fresh\":true}"), daemon.Calls.Last());
    }

    // The refusal line clears when the next switch starts (AppModel.swift:1078).
    [Fact]
    public async Task ARefusalLineClearsWhenTheNextSwitchStarts()
    {
        var (daemon, model) = await Listed();
        daemon.SelectReply = """{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}""";
        await model.SelectAsync("org-b");
        Assert.Equal("REFUSED", model.Failure);
        daemon.SelectReply = """{"id":1,"result":{"switched":true,"revoke_pending":false}}""";
        daemon.SelectGate = new TaskCompletionSource();
        Task pending = model.SelectAsync("org-b");
        Assert.Null(model.Failure);
        daemon.SelectGate.SetResult();
        await pending;
        Assert.Null(model.Failure);
    }

    // testTheLineUnderThePickerPrefersSwitchingThenFailureThenPendingRevoke.
    [Fact]
    public void TheLineUnderThePickerPrefersSwitchingThenFailureThenPendingRevoke()
    {
        SpendOrganizations status = Parse(Payload(new[] { ("org-a", true), ("org-b", false) }, revokePending: true));
        Assert.Equal(new SpendOrganizationLine("SWITCHING", SpendOrganizationTone.Ask), SpendOrganizationModel.LineFor(status, true, "FAILED"));
        Assert.Equal(new SpendOrganizationLine("FAILED", SpendOrganizationTone.Outside), SpendOrganizationModel.LineFor(status, false, "FAILED"));
        Assert.Equal(new SpendOrganizationLine("PENDING", SpendOrganizationTone.Ask), SpendOrganizationModel.LineFor(status, false, null));
        Assert.Null(SpendOrganizationModel.LineFor(Parse(Payload(new[] { ("org-a", true), ("org-b", false) })), false, null));
        Assert.Null(SpendOrganizationModel.LineFor(SpendOrganizations.Unreported, true, "FAILED"));
    }

    // While a switch is in flight (about ten seconds) the picker is held, its
    // line says so, a second pick sends nothing, and Not now waits for it
    // (AppModel.swift:1076 and :1104, review round 2 blocker 1).
    [Fact]
    public async Task ASwitchInFlightHoldsThePickerAndNotNow()
    {
        var (daemon, model) = await Listed();
        daemon.SelectGate = new TaskCompletionSource();
        Task pending = model.SelectAsync("org-b");
        Assert.True(model.Switching);
        Assert.False(model.CanPick);
        Assert.Equal(new SpendOrganizationLine("SWITCHING", SpendOrganizationTone.Ask), model.StateLine);
        await model.SelectAsync("org-a");
        await model.DismissAsync();
        Assert.Equal(new[] { "{\"organization_id\":\"org-b\"}" }, daemon.Selects);
        daemon.SelectGate.SetResult();
        await pending;
        Assert.True(model.CanPick);
    }

    // A pick waits for a sign-in or credential write (AppModel.swift:1076,
    // !credentialBusy), and the picker is held meanwhile (the card's
    // .disabled(spendOrganizationSwitching || credentialBusy)).
    [Fact]
    public async Task APickWaitsForACredentialAction()
    {
        var credential = new CredentialActivity { Busy = true };
        var (daemon, model) = await Listed(credential);
        Assert.False(model.CanPick);
        await model.PickAsync(1);
        Assert.Empty(daemon.Selects);
        var raised = new List<string?>();
        model.PropertyChanged += (_, e) => raised.Add(e.PropertyName);
        credential.Busy = false;
        Assert.True(model.CanPick);
        Assert.NotEmpty(raised);
        await model.PickAsync(1);
        Assert.Equal(new[] { "{\"organization_id\":\"org-b\"}" }, daemon.Selects);
    }

    // Nothing is preselected (one-pass 10, L5): reading, re-reading and the
    // picker echoing the organization already current send nothing; only a
    // pick of a different organization does (the card's
    // `guard wanted != status.currentOrganizationID`).
    [Fact]
    public async Task FillingOrEchoingThePickerSendsNothing()
    {
        var (daemon, model) = await Listed();
        await model.RefreshAsync();
        await model.RefreshAsync(fresh: true);
        Assert.Equal(0, model.CurrentIndex);
        await model.PickAsync(model.CurrentIndex);
        await model.PickAsync(-1);
        await model.PickAsync(2);
        Assert.Empty(daemon.Selects);
        await model.PickAsync(1);
        Assert.Equal(new[] { "{\"organization_id\":\"org-b\"}" }, daemon.Selects);
    }

    // The picker's rows are the daemon's, by name, in its order, and the
    // current one is the selection; one that is not listed selects nothing.
    [Fact]
    public async Task ThePickerShowsTheDaemonsOrganizationsInItsOrder()
    {
        var (_, model) = await Listed();
        Assert.True(model.ShowsPicker);
        Assert.Equal(new[] { "Name org-a", "Name org-b" }, model.OptionNames);
        Assert.Equal(("TITLE", "WHAT", true, "PICKER"), (model.PickerTitle, model.What, model.HasWhat, model.PickerName));

        var daemon = new Daemon();
        JsonObject view = View();
        view["what"] = "";
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", false), ("org-b", false) }, current: "org-z", view: view));
        var other = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, new CredentialActivity());
        await other.RefreshAsync();
        Assert.Equal(-1, other.CurrentIndex);
        Assert.False(other.HasWhat);
    }

    // One organization, or a listing that failed, draws no picker.
    [Fact]
    public async Task OneOrganizationDrawsNoPicker()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true) }));
        var model = new SpendOrganizationModel(daemon.Call, () => daemon.Connection, new CredentialActivity());
        await model.RefreshAsync();
        Assert.False(model.ShowsPicker);
        Assert.Empty(model.OptionNames);
        Assert.Null(model.StateLine);
    }

    // The key moved, so the balance it is read with did too: every settled
    // switch asks for a fresh balance (AppModel.swift:1093); Not now does not.
    [Fact]
    public async Task EverySettledSwitchAsksForAFreshBalance()
    {
        var (daemon, model) = await Listed();
        int settled = 0;
        model.SwitchSettled += () => settled++;
        await model.SelectAsync("org-b");
        Assert.Equal(1, settled);
        daemon.SelectReply = """{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}""";
        await model.SelectAsync("org-b");
        Assert.Equal(2, settled);
        await model.DismissAsync();
        Assert.Equal(2, settled);
    }

    // A switch that settles on another connection settles nothing here: no
    // line, no prompt change, and the picker is free again.
    [Fact]
    public async Task ASwitchThatSettlesOnAnotherConnectionSaysNothing()
    {
        var (daemon, model) = await Listed();
        daemon.SelectReply = """{"id":1,"error":{"code":"unavailable","message":"near_ai_spend_org_refused"}}""";
        daemon.SelectGate = new TaskCompletionSource();
        Task pending = model.SelectAsync("org-b");
        daemon.Connection++;
        daemon.SelectGate.SetResult();
        await pending;
        Assert.False(model.Switching);
        Assert.Null(model.Failure);
        Assert.True(model.IsShown);
    }
}
