using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Payouts section's link (W16-D2 (a)) against the Mac's
/// <c>PayoutLinkSurface</c>, <c>PayoutPicker</c> and <c>PayoutLinkModel</c>.
/// </summary>
/// <remarks>
/// Expected values were printed by a throwaway Swift probe that compiles
/// macos/Sources/TCShellCore/PayoutLinkSurface.swift and ReferralSurface.swift
/// verbatim at b8d4a6d70, linked to this worktree's FFI build, over the live
/// <c>tc_payout_link_copy_json</c>.
/// </remarks>
public class PayoutLinkTests
{
    private static PayoutLinkSurface Surface() => new(PayoutLinkCopy.Live!);

    public static IEnumerable<object?[]> Rails() => new[]
    {
        // name, rail, then the probe's RAIL line: statusLine, needsOrgName,
        // canChoose, canUnlink, canChange, showsChangeButton(idle),
        // showsChangeButton(changing), showsPicker(idle), unlink name.
        new object?[] { "none", new PayoutRailStatus(), "No organization chosen", false, true, false, false, false, false, true, null },
        new object?[] { "active", new PayoutRailStatus(Linked: true, State: "active", OrganizationName: "Acme"), "Credit goes to Acme", false, false, true, true, true, false, false, "Stop sending credit to Acme" },
        new object?[] { "activeNoName", new PayoutRailStatus(Linked: true, State: "active"), null, true, false, false, true, true, false, false, null },
        new object?[] { "waiting", new PayoutRailStatus(Linked: true, State: "waiting", OrganizationName: "Acme"), "Credit goes to Acme from the next payout", false, false, false, false, false, false, false, "Stop sending credit to Acme" },
        new object?[] { "closing", new PayoutRailStatus(Linked: true, State: "closing", OrganizationName: "Acme"), "No organization chosen", false, false, false, false, false, false, false, "Stop sending credit to Acme" },
        new object?[] { "unlinkReq", new PayoutRailStatus(Linked: true, State: "active", UnlinkRequested: true, OrganizationName: "Acme"), "No organization chosen", false, true, false, false, false, false, true, "Stop sending credit to Acme" },
        new object?[] { "mismatch", new PayoutRailStatus(Linked: true, State: "active", RecipientUnavailable: true, RecipientRefCheck: "mismatch", OrganizationName: "Acme"), "Credit may be going to an organization you didn't choose.", false, true, true, false, false, false, true, "Stop sending credit to Acme" },
        new object?[] { "unavail", new PayoutRailStatus(Linked: true, State: "active", RecipientUnavailable: true, OrganizationName: "Acme"), "Organization unavailable.", false, true, true, false, false, false, true, "Stop sending credit to Acme" },
    };

    [Theory]
    [MemberData(nameof(Rails))]
    public void EachRailReadsAsOnTheMac(
        string name, PayoutRailStatus rail, string? line, bool needsName, bool canChoose, bool canUnlink,
        bool canChange, bool changeIdle, bool changeChanging, bool picker, string? unlinkName)
    {
        _ = name;
        PayoutLinkSurface s = Surface();
        var idle = new PayoutPicker();
        var changing = new PayoutPicker();
        changing.BeginChange();
        Assert.Equal(line, s.StatusLine(rail));
        Assert.Equal(needsName, s.NeedsOrgName(rail));
        Assert.Equal(canChoose, PayoutLinkSurface.CanChoose(rail));
        Assert.Equal(canUnlink, PayoutLinkSurface.CanUnlink(rail));
        Assert.Equal(canChange, PayoutLinkSurface.CanChange(rail));
        Assert.Equal(changeIdle, PayoutLinkSurface.ShowsChangeButton(rail, idle));
        Assert.Equal(changeChanging, PayoutLinkSurface.ShowsChangeButton(rail, changing));
        Assert.Equal(picker, PayoutLinkSurface.ShowsPicker(rail, idle));
        Assert.Equal(unlinkName, s.UnlinkAccessibilityName(rail));
    }

    [Theory]
    // probe: REFUSAL lines.
    [InlineData("payout-link-near-ai-sign-in-required", PayoutLinkRefusal.SignIn)]
    [InlineData("payout-link-near-ai-session-expired", PayoutLinkRefusal.SignIn)]
    [InlineData("payout-link-near-ai-session-changed", PayoutLinkRefusal.SignIn)]
    [InlineData("account-session-required", PayoutLinkRefusal.SessionEnded)]
    [InlineData("payout-link-orgs-changed", PayoutLinkRefusal.Changed)]
    [InlineData("payout-link-changed", PayoutLinkRefusal.Changed)]
    [InlineData("version_conflict", PayoutLinkRefusal.Changed)]
    [InlineData("org_not_owned", PayoutLinkRefusal.Changed)]
    [InlineData("payout_link_change_pending", PayoutLinkRefusal.Changed)]
    [InlineData("x", PayoutLinkRefusal.Failed)]
    public void RefusalsRouteAsOnTheMac(string label, PayoutLinkRefusal want)
    {
        Assert.Equal(want, PayoutLinkRefusals.From(label));
    }

    [Fact]
    public void OnlyTheLiveOrDryRunNearAiRailIsShown()
    {
        // probe: NEAR 7 true
        Assert.Equal(7, PayoutLinkSurface.NearAiRail(new[]
        {
            new PayoutRailStatus(RailId: "other"), new PayoutRailStatus(Mode: "off"), new PayoutRailStatus(Mode: "dry_run", Version: 7),
        })!.Version);
        Assert.Null(PayoutLinkSurface.NearAiRail(new[] { new PayoutRailStatus(Mode: "off") }));
    }

    [Fact]
    public void ThePickerMovesAsTheMacs()
    {
        PayoutLinkCopy copy = PayoutLinkCopy.Live!;
        var listing = new PayoutOrganizationListing(new[] { new PayoutOrganization("o1", "Acme"), new PayoutOrganization("o2", "Beta") }, "r1");
        var p = new PayoutPicker();
        p.Select("o1");
        Assert.Equal(new PayoutPicker.Phase.Idle(), p.Current); // P0
        Assert.Null(p.SelectedId);
        p.BeginLoading();
        p.Listed(listing);
        Assert.Equal(new PayoutPicker.Phase.Choosing(), p.Current); // P1
        p.Select("zz");
        Assert.Null(p.SelectedId); // P2
        p.Select("o2");
        Assert.Equal("o2", p.SelectedId); // P3
        Assert.Equal(
            new PayoutPicker.Consent(
                "Beta",
                "Beta gets NEAR AI credit for your traces, on top of your Trace Credit. It can't be moved out, other members can spend it, and it pays past NEAR AI use first.",
                "Sends credit to Beta only.", "Choose", "Choose Beta for NEAR AI credit", "Not now"),
            p.ConsentFor(copy));
        p.BeginWorking();
        Assert.Equal(new PayoutPicker.Phase.Working(), p.Current); // P4
        p.Refuse(PayoutLinkRefusal.Failed);
        Assert.Equal(new PayoutPicker.Phase.Refused(PayoutLinkRefusal.Failed), p.Current); // P5
        Assert.NotNull(p.Listing);
        p.DismissSelection();
        Assert.Equal(new PayoutPicker.Phase.Choosing(), p.Current); // P6
        p.Refuse(PayoutLinkRefusal.Changed);
        Assert.Null(p.Listing); // P7
        p.DismissSelection();
        Assert.Equal(new PayoutPicker.Phase.Refused(PayoutLinkRefusal.Changed), p.Current); // P8

        var q = new PayoutPicker();
        q.BeginChange();
        q.Listed(new PayoutOrganizationListing(System.Array.Empty<PayoutOrganization>(), "r"));
        Assert.Equal(new PayoutPicker.Phase.NoOrganization(), q.Current); // Q1
        Assert.True(q.Changing);
        q.DismissSelection();
        Assert.Equal(new PayoutPicker.Phase.Idle(), q.Current); // Q2
        Assert.False(q.Changing);
    }

    private sealed class FakeDaemon
    {
        public readonly List<(string Method, string Params)> Sent = new();
        public Dictionary<string, string> Replies { get; } = new();

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Sent.Add((method, parameters));
            string body = Replies.TryGetValue(method, out string? r) ? r : """{"error":{"code":"unavailable","message":"unrecorded"}}""";
            return Task.FromResult(DaemonResponse.Parse(body));
        }
    }

    [Fact]
    public async Task ChoosingSendsStartThenConfirmWithTheRailsVersionAndRereads()
    {
        var daemon = new FakeDaemon();
        daemon.Replies["payout_rails_status"] = """{"result":{"rails":[{"rail_id":"near_ai","mode":"live","linked":false,"version":4}]}}""";
        daemon.Replies["payout_link_orgs"] = """{"result":{"organizations":[{"id":"o1","name":"Acme"}],"orgs_revision":"r9"}}""";
        daemon.Replies["payout_link_start"] = """{"result":{}}""";
        daemon.Replies["payout_link_confirm"] = """{"result":{}}""";
        var flow = new PayoutLinkFlow(new PayoutLinkClient(daemon.Call), Surface());
        await flow.RefreshAsync();
        Assert.True(flow.Visible);
        Assert.Equal(new PayoutPicker.Phase.Choosing(), flow.Picker.Current);
        // Nothing is preselected (checklist L5).
        Assert.Null(flow.Picker.SelectedId);
        flow.Select("o1");
        daemon.Sent.Clear();
        await flow.ChooseAsync();
        Assert.Equal(
            new[]
            {
                ("payout_link_start", """{"rail_id":"near_ai","organization_id":"o1","orgs_revision":"r9","expected_current_version":4}"""),
                ("payout_link_confirm", """{"rail_id":"near_ai","action":"link","organization_id":"o1","confirmed":true}"""),
                ("payout_rails_status", "{}"),
            },
            daemon.Sent.Take(3).ToArray());
        Assert.Null(flow.Refusal);
        Assert.False(flow.Busy);
    }

    [Fact]
    public async Task ARefusedLinkSendsNoConfirmAndTheRereadListsAgain()
    {
        var daemon = new FakeDaemon();
        daemon.Replies["payout_rails_status"] = """{"result":{"rails":[{"rail_id":"near_ai","mode":"live","linked":false,"version":4}]}}""";
        daemon.Replies["payout_link_orgs"] = """{"result":{"organizations":[{"id":"o1","name":"Acme"}],"orgs_revision":"r9"}}""";
        daemon.Replies["payout_link_start"] = """{"error":{"code":"unavailable","message":"version_conflict"}}""";
        var flow = new PayoutLinkFlow(new PayoutLinkClient(daemon.Call), Surface());
        await flow.RefreshAsync();
        flow.Select("o1");
        PayoutLinkRefusal? seen = null;
        flow.Changed += () => seen ??= flow.Refusal;
        await flow.ChooseAsync();
        // The refusal was said, then the Mac's finish() re-reads the status,
        // whose organization listing clears it (PayoutLinkModel.swift
        // loadOrganizations: refusal = nil on success).
        Assert.Equal(PayoutLinkRefusal.Changed, seen);
        Assert.DoesNotContain(daemon.Sent, s => s.Method == "payout_link_confirm");
        Assert.Equal("Your organizations changed.", flow.Surface!.RefusalLine(PayoutLinkRefusal.Changed));
        Assert.Null(flow.Refusal);
        Assert.Equal(new PayoutPicker.Phase.Choosing(), flow.Picker.Current);
        Assert.Null(flow.Picker.SelectedId);
    }

    [Fact]
    public async Task AStatusReadFailureIsSaidOnlyForAnEndedSession()
    {
        var daemon = new FakeDaemon();
        daemon.Replies["payout_rails_status"] = """{"error":{"code":"unavailable","message":"account-session-required"}}""";
        var flow = new PayoutLinkFlow(new PayoutLinkClient(daemon.Call), Surface());
        await flow.RefreshAsync();
        Assert.Equal(PayoutLinkRefusal.SessionEnded, flow.StatusRefusal);
        Assert.True(flow.Visible);

        daemon.Replies["payout_rails_status"] = """{"error":{"code":"unavailable","message":"unrecorded"}}""";
        await flow.RefreshAsync();
        Assert.Null(flow.StatusRefusal);
        Assert.False(flow.Visible);
    }
}
