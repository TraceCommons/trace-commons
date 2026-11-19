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

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Calls.Add((method, parameters));
            string frame = method == SpendOrganizationPromptModel.SelectMethod
                ? SelectReply
                : "{\"id\":1,\"result\":" + (Listings.Count > 1 ? Listings.Dequeue() : Listings.Peek()) + "}";
            return Task.FromResult(DaemonResponse.Parse(frame));
        }
    }

    [Fact]
    public async Task NotNowRecordsTheCurrentOrganizationAndEndsThePrompt()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }, @explicit: true));
        var model = new SpendOrganizationPromptModel(daemon.Call, () => daemon.Connection);
        await model.RefreshAsync();
        Assert.True(model.IsShown);
        Assert.Equal(("PROMPT-TITLE", "PROMPT-LINE", "PROMPT-ACTION", "PROMPT-ACTION-AX", "PROMPT-DISMISS", "PROMPT-DISMISS-AX"),
            (model.Title, model.Line, model.Action, model.ActionName, model.Dismiss, model.DismissName));
        await model.DismissAsync();
        Assert.Equal(new[] { "{\"organization_id\":\"org-a\"}" },
            daemon.Calls.Where(c => c.Method == SpendOrganizationPromptModel.SelectMethod).Select(c => c.Params));
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
        var model = new SpendOrganizationPromptModel(daemon.Call, () => daemon.Connection);
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
        var model = new SpendOrganizationPromptModel(daemon.Call, () => daemon.Connection);
        await model.RefreshAsync();
        await model.DismissAsync();
        Assert.DoesNotContain(daemon.Calls, c => c.Method == SpendOrganizationPromptModel.SelectMethod);
    }

    // A listing read for an earlier connection is not drawn for the new one.
    [Fact]
    public async Task AListingFromAnEarlierConnectionIsDropped()
    {
        var daemon = new Daemon();
        daemon.Listings.Enqueue(Payload(new[] { ("org-a", true), ("org-b", false) }));
        var model = new SpendOrganizationPromptModel(
            async (method, parameters) => { DaemonResponse r = await daemon.Call(method, parameters); daemon.Connection++; return r; },
            () => daemon.Connection);
        await model.RefreshAsync();
        Assert.False(model.IsShown);
    }
}
