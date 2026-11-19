using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Referrals section (W16-D2 (a)) against the Mac's <c>ReferralSurface</c>
/// and <c>ReferralModel</c>.
/// </summary>
/// <remarks>
/// Expected values were printed by a throwaway Swift probe that compiles
/// macos/Sources/TCShellCore/ReferralSurface.swift verbatim at b8d4a6d70,
/// linked to this worktree's FFI build, over the live
/// <c>tc_referral_copy_json</c> and <c>tc_referral_code_display</c>.
/// </remarks>
public class ReferralsTests
{
    private static ReferralSurface Surface() => new(ReferralCopy.Live!);

    private static readonly ReferralCodeRow Live = new("c1", "TC-7K3M9-QX2RD", "live", 1);
    private static readonly ReferralCodeRow Hidden = new("c2", null, "suspended", 2);

    [Theory]
    // probe: STATE lines.
    [InlineData("live", null)]
    [InlineData("suspended", "Paused")]
    [InlineData("voided", "Closed")]
    [InlineData("disabled", "Off")]
    [InlineData("other", "Off")]
    public void StatesReadAsOnTheMac(string state, string? label)
    {
        Assert.Equal(label, Surface().StateLabel(new ReferralCodeRow("x", null, state)));
    }

    [Fact]
    public void RowsTotalsAndLinesMatchTheMac()
    {
        ReferralSurface s = Surface();
        // probe: JOINED, SHOWN, AX, CANCOPY, LIMIT, CREDIT, SCOPE
        Assert.Equal(new[] { "0 people joined", "1 person joined", "2 people joined" }, new ulong[] { 0, 1, 2 }.Select(s.JoinedLine));
        Assert.Equal("TC-7K3M9-QX2RD", s.ShownCode(Live));
        Assert.Equal("Code not shown", s.ShownCode(Hidden));
        Assert.Equal("Copy code TC-7K3M9-QX2RD", s.CopyAccessibilityName(Live));
        Assert.Equal("Turn off code Code not shown", s.DisableAccessibilityName(Hidden));
        Assert.True(ReferralSurface.CanCopy(Live));
        Assert.False(ReferralSurface.CanCopy(Hidden));
        Assert.True(ReferralSurface.CanDisable(Live));
        Assert.False(ReferralSurface.CanDisable(Hidden));
        Assert.Equal("You can have 3 codes on at once.", s.LimitLine(new ReferralCodesListing(new ReferralCodeRow[0], 3, CanCreate: false)));
        Assert.Null(s.LimitLine(new ReferralCodesListing(new ReferralCodeRow[0])));
        string Money(ulong n) => $"${n}";
        Assert.Equal("$5", s.CreditLine(new ReferralCodesListing(new ReferralCodeRow[0], BonusOwedNanoUsd: 5, BonusSentNanoUsd: 0), Money));
        Assert.Equal("$5 · $2 sent", s.CreditLine(new ReferralCodesListing(new ReferralCodeRow[0], BonusOwedNanoUsd: 5, BonusSentNanoUsd: 2), Money));
        Assert.Null(s.CreditLine(new ReferralCodesListing(new ReferralCodeRow[0], BonusOwedNanoUsd: null), Money));
        Assert.Equal("You can enter one code until Dec 1. It can't be changed after.", s.EnterScope("Dec 1"));
    }

    [Theory]
    // probe: REFUSAL lines (with and without the live-code limit).
    [InlineData("account-session-required", ReferralRefusal.SessionEnded, "Sign in again", "Sign in again")]
    [InlineData("referral_not_applied", ReferralRefusal.NotApplied, "That code couldn't be applied.", "That code couldn't be applied.")]
    [InlineData("referral-code-invalid", ReferralRefusal.InvalidCode, "That isn't a referral code.", "That isn't a referral code.")]
    [InlineData("referral_unavailable", ReferralRefusal.Unavailable, "Referral codes aren't open yet.", "Referral codes aren't open yet.")]
    [InlineData("referral_code_limit_reached", ReferralRefusal.LimitReached, "You can have 5 codes on at once.", "Nothing changed.")]
    [InlineData("zzz", ReferralRefusal.Failed, "Nothing changed.", "Nothing changed.")]
    public void RefusalsRouteAndReadAsOnTheMac(string label, ReferralRefusal want, string withLimit, string withoutLimit)
    {
        Assert.Equal(want, ReferralRefusals.From(label));
        Assert.Equal(withLimit, Surface().RefusalLine(want, 5));
        Assert.Equal(withoutLimit, Surface().RefusalLine(want));
    }

    [Theory]
    // probe: FIELD lines, through the live tc_referral_code_display.
    [InlineData("", null)]
    [InlineData("  ", null)]
    [InlineData("tc-7k3m9-qx2rd", null)]
    [InlineData("nope", "That isn't a referral code.")]
    [InlineData(" 7K3M9 QX2RD ", null)]
    public void ATypedCodeIsCheckedAsTheDaemonReadsIt(string typed, string? problem)
    {
        Assert.Equal(problem, Surface().FieldProblem(typed, ReferralCopy.Display));
    }

    [Fact]
    public void AListingAndStatusDecodeAsOnTheMac()
    {
        // probe: LISTING false 5 nil nil; STATUS false true ... true false
        using JsonDocument l = JsonDocument.Parse("""{"codes":[{"code_id":"a","state":"live","referee_count":2},{"code_id":"b","state":"voided","referee_count":3,"bonus_owed_nano_usd":"+5"}],"max_live_codes":5}""");
        ReferralCodesListing listing = ReferralCodesListing.From(l.RootElement)!;
        Assert.False(listing.CanCreate);
        Assert.Equal(5UL, listing.PeopleJoined);
        Assert.Null(listing.BonusOwedNanoUsd);
        Assert.Null(listing.Codes[1].BonusOwedNanoUsd);
        using JsonDocument a = JsonDocument.Parse("""{"applied":true,"can_apply":true}""");
        using JsonDocument b = JsonDocument.Parse("""{"applied":false,"can_apply":true,"apply_until":"2026-12-01T00:00:00Z"}""");
        ReferralStatus applied = ReferralStatus.From(a.RootElement)!;
        ReferralStatus open = ReferralStatus.From(b.RootElement)!;
        Assert.False(applied.CanApply);
        Assert.True(open.CanApply);
        Assert.Equal("2026-12-01T00:00:00Z", open.ApplyUntil);
        Assert.True(ReferralSurface.ShowsEnterField(open));
        Assert.False(ReferralSurface.ShowsEnterField(null));
    }

    private sealed class Daemon
    {
        public readonly List<(string Method, string Params)> Sent = new();
        public string List = """{"result":{"codes":[],"max_live_codes":5,"can_create":true}}""";
        public string Status = """{"result":{"applied":false,"can_apply":true}}""";
        public string Apply = """{"result":{}}""";

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Sent.Add((method, parameters));
            return Task.FromResult(DaemonResponse.Parse(method switch
            {
                "referral_codes_list" => List,
                "referral_status" => Status,
                "referral_apply" => Apply,
                "referral_codes_create" => """{"result":{"code_id":"n","code":"TC-AAAAA-BBBBB"}}""",
                _ => """{"result":{}}""",
            }));
        }
    }

    [Fact]
    public async Task ApplyingSendsTheTypedCodeClearsTheFieldAndReadsAgain()
    {
        var daemon = new Daemon();
        var flow = new ReferralFlow(new ReferralClient(daemon.Call), Surface());
        await flow.RefreshAsync();
        flow.Draft = " tc-7k3m9-qx2rd ";
        Assert.True(flow.CanApply);
        daemon.Sent.Clear();
        await flow.ApplyAsync();
        Assert.Equal(("referral_apply", """{"code":" tc-7k3m9-qx2rd "}"""), daemon.Sent[0]);
        Assert.Equal(new[] { "referral_codes_list", "referral_status" }, daemon.Sent.Skip(1).Select(s => s.Method));
        Assert.Equal(string.Empty, flow.Draft);
        Assert.Null(flow.Refusal);
    }

    [Fact]
    public async Task ARefusedApplyKeepsTheDraftAndSaysWhy()
    {
        var daemon = new Daemon { Apply = """{"error":{"code":"unavailable","message":"referral_not_applied"}}""" };
        var flow = new ReferralFlow(new ReferralClient(daemon.Call), Surface()) { Draft = "TC-7K3M9-QX2RD" };
        await flow.RefreshAsync();
        await flow.ApplyAsync();
        Assert.Equal(ReferralRefusal.NotApplied, flow.Refusal);
        Assert.Equal("TC-7K3M9-QX2RD", flow.Draft);
    }

    [Fact]
    public async Task NotACodeIsNeverSent()
    {
        var daemon = new Daemon();
        var flow = new ReferralFlow(new ReferralClient(daemon.Call), Surface()) { Draft = "nope" };
        Assert.False(flow.CanApply);
        await flow.ApplyAsync();
        Assert.Empty(daemon.Sent);
    }

    [Fact]
    public async Task AnEndedSessionLeavesNothingStaleAndAFailedReReadKeepsTheLastAnswer()
    {
        var daemon = new Daemon();
        var flow = new ReferralFlow(new ReferralClient(daemon.Call), Surface());
        await flow.RefreshAsync();
        Assert.NotNull(flow.Listing);
        daemon.List = """{"error":{"code":"unavailable","message":"unrecorded"}}""";
        await flow.RefreshAsync();
        Assert.NotNull(flow.Listing);
        Assert.False(flow.Failed);
        daemon.List = """{"error":{"code":"unavailable","message":"account-session-required"}}""";
        await flow.RefreshAsync();
        Assert.True(flow.SessionEnded);
        Assert.Null(flow.Listing);
        Assert.Null(flow.Status);
    }

    [Fact]
    public void ReferralMoneyIsTheCoresFigure()
    {
        // ReferralMoney.format: TCNearAiBalance.amount(present: 1, nanos, scale: 9).
        string viaCore = NativeMethods.TakeOwnedString(NativeMethods.tc_near_ai_balance_amount(1, 1_250_000_000, 9))!;
        Assert.Equal(viaCore, ReferralMoney.Format(1_250_000_000));
        Assert.False(string.IsNullOrEmpty(viaCore));
    }
}
