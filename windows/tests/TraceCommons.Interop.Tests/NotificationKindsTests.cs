using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W17-D2 (owner ruling 2026-11-19): the Mac's shown notification kinds
/// (<c>NudgeSettings.shownKinds</c>, NudgeCopy.swift) as per-kind switches,
/// its two one-time offers, and a notification per <c>reengage_due</c>, in
/// the core's nudge words only.
/// </summary>
public class NotificationKindsTests
{
    private const string AllOn = """
        {"notifications_enabled": true,
         "notify": {"digest": true, "idle_sessions": false, "verdicts_landed": true, "weekly_recap": true,
                    "insights_tip": true, "settlement_estimate": true, "near_ai_credit": false},
         "verdicts_offer_pending": true, "idle_offer_pending": true}
        """;

    private static NotificationSettingsState State(string json) =>
        NotificationSettingsState.Parse(json) ?? throw new InvalidOperationException("unparsed");

    // ---- Switches ----------------------------------------------------------

    [Fact]
    public void TheKindsAreTheMacsShownFourInItsOrderWithTheCoresWords()
    {
        IReadOnlyList<NotifySwitchRow> rows = NotificationKinds.Rows(State(AllOn), NudgeWords.Word, digestHelp: null);
        Assert.Equal(new[] { "digest", "verdicts_landed", "idle_sessions", "near_ai_credit" }, rows.Select(r => r.Kind));
        Assert.Equal(NudgeWords.Word("SETTING_DIGEST"), rows[0].Label);
        Assert.Equal(NudgeWords.Word("SETTING_NOTIFY_VERDICTS"), rows[1].Label);
        Assert.Equal(NudgeWords.Word("SETTING_NOTIFY_IDLE"), rows[2].Label);
        Assert.Equal(NudgeWords.Word("SETTING_NOTIFY_IDLE_HELP"), rows[2].Help);
        Assert.Equal(NudgeWords.Word("SETTING_NOTIFY_NEAR_AI_CREDIT"), rows[3].Label);
        Assert.All(rows, r => Assert.False(string.IsNullOrEmpty(r.Label)));
        Assert.Equal(new[] { true, true, false, false }, rows.Select(r => r.IsOn));
        Assert.All(rows, r => Assert.True(r.Enabled));
    }

    [Fact]
    public void TheHeldKindsAreNeverDrawn()
    {
        IEnumerable<string> kinds = NotificationKinds.Rows(State(AllOn), NudgeWords.Word, null).Select(r => r.Kind);
        Assert.DoesNotContain("weekly_recap", kinds);
        Assert.DoesNotContain("insights_tip", kinds);
        Assert.DoesNotContain("settlement_estimate", kinds);
    }

    [Fact]
    public void TheDigestHelpIsTheCoresFinishedLine()
    {
        IReadOnlyList<NotifySwitchRow> rows = NotificationKinds.Rows(State(AllOn), NudgeWords.Word, digestHelp: "At most one a day.");
        Assert.Equal("At most one a day.", rows[0].Help);
    }

    [Fact]
    public void AnUnreportedKindHasNoSwitchAndAnUnreadSettingsHasNone()
    {
        NotificationSettingsState state = State("""{"notifications_enabled": true, "notify": {"digest": true}}""");
        Assert.Equal(new[] { "digest" }, NotificationKinds.Rows(state, NudgeWords.Word, null).Select(r => r.Kind));
        Assert.Empty(NotificationKinds.Rows(null, NudgeWords.Word, null));
        Assert.Null(NotificationSettingsState.Parse("not json"));
    }

    [Fact]
    public void AKindWithoutTheCoresWordIsNotDrawn()
    {
        Assert.Empty(NotificationKinds.Rows(State(AllOn), _ => null, null));
    }

    [Fact]
    public void EveryKindIsDisabledUnderAnOffMasterButKeepsItsValue()
    {
        NotificationSettingsState state = State(AllOn.Replace("\"notifications_enabled\": true", "\"notifications_enabled\": false", StringComparison.Ordinal));
        IReadOnlyList<NotifySwitchRow> rows = NotificationKinds.Rows(state, NudgeWords.Word, null);
        Assert.All(rows, r => Assert.False(r.Enabled));
        Assert.True(rows[0].IsOn);
    }

    [Theory]
    [InlineData("digest", true, """{"kind":"digest","on":true}""")]
    [InlineData("verdicts_landed", false, """{"kind":"verdicts_landed","on":false}""")]
    [InlineData("idle_sessions", true, """{"kind":"idle_sessions","on":true}""")]
    [InlineData("near_ai_credit", false, """{"kind":"near_ai_credit","on":false}""")]
    public void ASwitchWritesItsKindThroughSetNotifyKind(string kind, bool on, string expected)
    {
        NudgeRequest request = NotificationKinds.SetKind(kind, on);
        Assert.Equal("set_notify_kind", request.Method);
        Assert.Equal(expected, request.ParamsJson);
    }

    [Fact]
    public void AKindTheMacDoesNotShowIsNeverWritten()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => NotificationKinds.SetKind("weekly_recap", true));
    }

    // ---- Offers ------------------------------------------------------------

    [Fact]
    public void ThePendingOffersAreVerdictsThenIdleInTheCoresWords()
    {
        IReadOnlyList<NotifyOffer> offers = NotificationKinds.Offers(State(AllOn), NudgeWords.Word);
        Assert.Equal(new[] { "verdicts_landed", "idle_sessions" }, offers.Select(o => o.Kind));
        Assert.Equal(NudgeWords.Word("OFFER_NOTIFY_VERDICTS_EXISTING"), offers[0].Text);
        Assert.Equal(NudgeWords.Word("OFFER_NOTIFY_IDLE_EXISTING"), offers[1].Text);
        Assert.All(offers, o =>
        {
            Assert.Equal(NudgeWords.Word("OFFER_TURN_ON"), o.Accept);
            Assert.Equal(NudgeWords.Word("OFFER_NO_THANKS"), o.Decline);
            Assert.False(string.IsNullOrEmpty(o.Text));
        });
    }

    [Fact]
    public void NoOfferIsDrawnThatTheDaemonDidNotMarkPending()
    {
        NotificationSettingsState state = State("""{"notifications_enabled": true, "notify": {"digest": true}}""");
        Assert.Empty(NotificationKinds.Offers(state, NudgeWords.Word));
        Assert.Empty(NotificationKinds.Offers(null, NudgeWords.Word));
        Assert.Empty(NotificationKinds.Offers(State(AllOn), _ => null));
    }

    [Fact]
    public void EachPlaceDrawsItsOwnOffer()
    {
        NotificationSettingsState state = State(AllOn);
        Assert.Equal("verdicts_landed", Assert.Single(NotificationKinds.Offers(state, NudgeWords.Word, OfferPlace.History)).Kind);
        Assert.Equal("idle_sessions", Assert.Single(NotificationKinds.Offers(state, NudgeWords.Word, OfferPlace.Traces)).Kind);
    }

    [Fact]
    public void EnableTurnsTheKindOnAndNoThanksClearsOnlyTheMarker()
    {
        NotifyOffer verdicts = NotificationKinds.Offers(State(AllOn), NudgeWords.Word)[0];
        NotifyOffer idle = NotificationKinds.Offers(State(AllOn), NudgeWords.Word)[1];

        Assert.Equal(new NudgeRequest("set_notify_kind", """{"kind":"verdicts_landed","on":true}"""), NotificationKinds.Accept(verdicts));
        Assert.Equal(new NudgeRequest("set_notify_kind", """{"kind":"idle_sessions","on":true}"""), NotificationKinds.Accept(idle));
        Assert.Equal(new NudgeRequest("set_settings", """{"verdicts_offer_pending":false}"""), NotificationKinds.Decline(verdicts));
        Assert.Equal(new NudgeRequest("set_settings", """{"idle_offer_pending":false}"""), NotificationKinds.Decline(idle));
    }

    // ---- Notifications -----------------------------------------------------

    private static DaemonEvent Due(string kind, string actions = """[{"id":"review","label":"Review"},{"id":"not_now","label":"Not now"}]""") =>
        DaemonEvent.Parse($$$"""{"event":"reengage_due","data":{"kind":"{{{kind}}}","title":"T","body":"B","actions":{{{actions}}}}}""")!;

    [Fact]
    public void ANotificationIsTheDaemonsTitleAndBodyUnchanged()
    {
        ReengageNotice notice = Assert.IsType<ReengageNotice>(ReengageNotice.From(Due("idle_sessions")));
        Assert.Equal("idle_sessions", notice.Kind);
        Assert.Equal("T", notice.Title);
        Assert.Equal("B", notice.Body);
    }

    [Fact]
    public void AnUnknownKindOrOneWithoutWordsPostsNothing()
    {
        Assert.Null(ReengageNotice.From(Due("weekly_recap")));
        Assert.Null(ReengageNotice.From(DaemonEvent.Parse("""{"event":"reengage_due","data":{"kind":"idle_sessions","title":"","body":"B","actions":[]}}""")!));
        Assert.Null(ReengageNotice.From(DaemonEvent.Parse("""{"event":"digest_due","data":{"pending":1}}""")!));
    }

    /// <summary>
    /// A click on the balloon is the notification's default action, the
    /// Mac's <c>defaultIntent</c>: its first button that is not Not now.
    /// </summary>
    [Theory]
    [InlineData("idle_sessions", """[{"id":"review","label":"Review"},{"id":"not_now","label":"Not now"}]""", "nudge_opened", NudgeDestination.TracesIdle)]
    [InlineData("review_backlog", """[{"id":"review","label":"Review"},{"id":"not_now","label":"Not now"}]""", "nudge_opened", NudgeDestination.Traces)]
    [InlineData("verdicts_landed", """[{"id":"see_history","label":"See history"}]""", "nudge_opened", NudgeDestination.History)]
    [InlineData("near_ai_credit", """[{"id":"review","label":"View"},{"id":"not_now","label":"Not now"}]""", null, NudgeDestination.PrivateAI)]
    [InlineData("settlement_estimate", """[{"id":"review","label":"Review traces"},{"id":"not_now","label":"Not now"}]""", null, NudgeDestination.Traces)]
    public void AClickDoesWhatTheMacsDefaultActionDoes(string kind, string actions, string? method, NudgeDestination destination)
    {
        ReengageNotice notice = ReengageNotice.From(Due(kind, actions))!;
        NudgeClick click = Assert.IsType<NudgeClick>(notice.Click);
        Assert.Equal(destination, click.Destination);
        if (method is null)
        {
            Assert.Null(click.Request);
        }
        else
        {
            Assert.Equal(new NudgeRequest(method, $$"""{"kind":"{{kind}}"}"""), click.Request);
        }
    }

    [Fact]
    public void ANotificationWithOnlyNotNowOpensNothing()
    {
        ReengageNotice notice = ReengageNotice.From(Due("idle_sessions", """[{"id":"not_now","label":"Not now"}]"""))!;
        Assert.Null(notice.Click);
    }

    /// <summary>
    /// <c>reengage_due</c> is declared only while a balloon can be shown:
    /// the daemon stamps its caps only while a renderer is declared, so a
    /// declaration nobody can draw would spend the budget on nothing
    /// (<c>Notifier.acceptedEvents</c>).
    /// </summary>
    [Fact]
    public void ReengageIsDeclaredOnlyWhileABalloonCanBeShown()
    {
        Assert.Equal(new[] { "reengage_due" }, ReengageNotice.AcceptedEvents(canShow: true));
        Assert.Empty(ReengageNotice.AcceptedEvents(canShow: false));
        Assert.Equal("[\"reengage_due\"]", JsonSerializer.Serialize(ReengageNotice.AcceptedEvents(true)));
    }
}

/// <summary>
/// The Notifications store (<c>NudgeSettingsStore</c>): every write is
/// followed by a fresh read, so a switch shows what the daemon stored; a
/// refused write says so and changes nothing drawn.
/// </summary>
public class NotificationKindsStoreTests
{
    private const string Settings = """{"notifications_enabled": true, "notify": {"digest": true, "verdicts_landed": false, "idle_sessions": true, "near_ai_credit": true}, "verdicts_offer_pending": true}""";

    private sealed class FakeDaemon
    {
        public List<(string Method, string Params)> Calls { get; } = new();
        public string SettingsJson { get; set; } = Settings;
        public bool RefuseWrites { get; set; }
        public bool FailReads { get; set; }

        public System.Threading.Tasks.Task<DaemonResponse> Call(string method, string json)
        {
            Calls.Add((method, json));
            string frame = method == DaemonProtocol.Methods.GetSettings
                ? (FailReads ? """{"error":{"code":"unavailable","message":"x"}}""" : $$"""{"result":{{SettingsJson}}}""")
                : (RefuseWrites ? """{"error":{"code":"bad_params","message":"x"}}""" : """{"result":{"ok":true}}""");
            return System.Threading.Tasks.Task.FromResult(DaemonResponse.Parse(frame));
        }
    }

    private static NotificationKindsStore Store(FakeDaemon daemon) =>
        new(daemon.Call) { RefusalLine = () => "refused" };

    [Fact]
    public async System.Threading.Tasks.Task NothingIsDrawnUntilTheSettingsAnswer()
    {
        var daemon = new FakeDaemon { FailReads = true };
        NotificationKindsStore store = Store(daemon);
        Assert.Empty(store.Rows);
        await store.ReadAsync();
        Assert.True(store.ReadFailed);
        Assert.Empty(store.Rows);
        Assert.Empty(store.Offers(OfferPlace.History));
    }

    [Fact]
    public async System.Threading.Tasks.Task ASwitchWritesItsKindThenReadsWhatTheDaemonStored()
    {
        var daemon = new FakeDaemon();
        NotificationKindsStore store = Store(daemon);
        await store.ReadAsync();
        Assert.False(store.Rows.Single(r => r.Kind == "verdicts_landed").IsOn);

        daemon.SettingsJson = Settings.Replace("\"verdicts_landed\": false", "\"verdicts_landed\": true", StringComparison.Ordinal);
        await store.SetAsync("verdicts_landed", true);

        Assert.Equal(("set_notify_kind", """{"kind":"verdicts_landed","on":true}"""), daemon.Calls[1]);
        Assert.Equal(DaemonProtocol.Methods.GetSettings, daemon.Calls[2].Method);
        Assert.True(store.Rows.Single(r => r.Kind == "verdicts_landed").IsOn);
        Assert.Null(store.Refusal);
    }

    [Fact]
    public async System.Threading.Tasks.Task ARefusedWriteSaysSoAndKeepsTheStoredValue()
    {
        var daemon = new FakeDaemon();
        NotificationKindsStore store = Store(daemon);
        await store.ReadAsync();
        daemon.RefuseWrites = true;
        await store.SetAsync("digest", false);
        Assert.Equal("refused", store.Refusal);
        Assert.True(store.Rows.Single(r => r.Kind == "digest").IsOn);
        Assert.Null(store.RefusedOfferKind);
    }

    [Fact]
    public async System.Threading.Tasks.Task AnOffersAnswerIsWrittenAndTheOfferGoesWithTheFreshRead()
    {
        var daemon = new FakeDaemon();
        NotificationKindsStore store = Store(daemon);
        await store.ReadAsync();
        NotifyOffer offer = Assert.Single(store.Offers(OfferPlace.History));

        daemon.SettingsJson = Settings.Replace(", \"verdicts_offer_pending\": true", string.Empty, StringComparison.Ordinal);
        await store.AnswerAsync(offer, accept: false);

        Assert.Equal(("set_settings", """{"verdicts_offer_pending":false}"""), daemon.Calls[1]);
        Assert.Empty(store.Offers(OfferPlace.History));
    }

    [Fact]
    public async System.Threading.Tasks.Task ARefusedOfferAnswerIsSaidUnderThatOffer()
    {
        var daemon = new FakeDaemon();
        NotificationKindsStore store = Store(daemon);
        await store.ReadAsync();
        NotifyOffer offer = Assert.Single(store.Offers(OfferPlace.History));
        daemon.RefuseWrites = true;
        await store.AnswerAsync(offer, accept: true);
        Assert.Equal(("set_notify_kind", """{"kind":"verdicts_landed","on":true}"""), daemon.Calls[1]);
        Assert.Equal("verdicts_landed", store.RefusedOfferKind);
        Assert.Equal("refused", store.Refusal);
        Assert.Single(store.Offers(OfferPlace.History));
    }
}
