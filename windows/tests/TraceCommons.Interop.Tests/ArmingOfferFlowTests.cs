using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The arming offer's answers, settled against the press that sent them
/// (checklist L1, L3), as the Mac runs them: AppModel.swift:2222-2259 at
/// origin/main 83016f544 and ArmingOfferMotion.swift:16-52.
/// </summary>
/// <remarks>
/// Reply frames are the daemon's own shapes: <c>handle_set_project_mode</c>'s
/// <c>{"ok":true,"purged":..,"retracted":..,"from_now":..,"overridden_by":..}</c>
/// and <c>Response::err(id, "unavailable", "policy-write-failed")</c>
/// (crates/trace-commons-contributor/src/daemon/ipc.rs).
/// </remarks>
public class ArmingOfferFlowTests
{
    private const string Armed =
        """{"id":1,"result":{"ok":true,"purged":0,"retracted":0,"from_now":false,"overridden_by":null}}""";

    private const string Declined = """{"id":1,"result":{"ok":true}}""";

    private static string Refused(string label, string code = "unavailable") =>
        $$$"""{"id":1,"error":{"code":"{{{code}}}","message":"{{{label}}}"}}""";

    private static readonly ArmingOffer Api = new("p-api", "api", 5);
    private static readonly ArmingOffer Web = new("p-web", "web", 6);

    /// <summary>A daemon whose replies the test releases one at a time.</summary>
    private sealed class Rig
    {
        public readonly List<(string Method, string Params)> Calls = new();
        public readonly Queue<TaskCompletionSource<DaemonResponse>> Replies = new();
        public int Refreshes;
        public Func<Task>? RefreshOverride;
        public TaskCompletionSource Hold = new();
        public TimeSpan? HeldFor;
        public ArmingOffer? RefreshAnswer;
        public bool RefreshAnswers;
        public ArmingOfferFlow Flow = null!;

        public Rig()
        {
            // Continuations run inline as each reply is released, so a test
            // reads the state the reply left.
            System.Threading.SynchronizationContext.SetSynchronizationContext(null);
            Flow = new ArmingOfferFlow(
                (method, parameters) =>
                {
                    Calls.Add((method, parameters));
                    var reply = new TaskCompletionSource<DaemonResponse>();
                    Replies.Enqueue(reply);
                    return reply.Task;
                },
                () =>
                {
                    Refreshes++;
                    if (RefreshOverride is { } refresh)
                    {
                        return refresh();
                    }

                    if (RefreshAnswers)
                    {
                        Flow.OnRefreshed(RefreshAnswer);
                    }

                    return Task.CompletedTask;
                },
                span =>
                {
                    HeldFor = span;
                    return Hold.Task;
                },
                () => "Couldn't change this folder's setting.");
        }

        public void Answer(string frame) => Replies.Dequeue().SetResult(DaemonResponse.Parse(frame));

        public void Throw() => Replies.Dequeue().SetException(new System.IO.IOException("pipe closed"));
    }

    [Fact]
    public void EnableSendsSetProjectModeAutomaticForTheOfferedProjectAndLocks()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);

        Task pressed = rig.Flow.AcceptAsync();

        Assert.Equal(("set_project_mode", """{"project_id":"p-api","mode":"auto_upload"}"""), Assert.Single(rig.Calls));
        Assert.Equal(ArmingOfferStep.Arming, rig.Flow.Step);
        Assert.True(rig.Flow.EnableLocked);
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.False(pressed.IsCompleted);
    }

    [Fact]
    public async Task ASecondPressWhileTheCallIsOutSendsNothing()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task first = rig.Flow.AcceptAsync();

        await rig.Flow.AcceptAsync();
        await rig.Flow.DeclineAsync();

        Assert.Single(rig.Calls);
        rig.Answer(Refused("project-id-unrecognized", "bad_params"));
        await first;
    }

    [Fact]
    public async Task AConfirmedReplyTurnsTheMarkToACheckAndTheCardLeavesAfterTheHold()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Armed);

        Assert.Equal(ArmingOfferStep.Armed, rig.Flow.Step);
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Equal(1, rig.Refreshes);
        // ArmingOfferMotion.swift:18: .milliseconds(900).
        Assert.Equal(TimeSpan.FromMilliseconds(900), rig.HeldFor);
        rig.Hold.SetResult();
        await pressed;
        Assert.Null(rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
    }

    [Fact]
    public async Task ARefreshThatAlreadyReadsTheProjectArmedDoesNotPullTheCardFromUnderTheCheck()
    {
        var rig = new Rig { RefreshAnswers = true, RefreshAnswer = null };
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Armed);

        Assert.Equal(1, rig.Refreshes);
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Equal(ArmingOfferStep.Armed, rig.Flow.Step);
        rig.Hold.SetResult();
        await pressed;
        Assert.Null(rig.Flow.Offer);
    }

    // A SwiftUI removal draws the view's last state: the armed card fades
    // with its words and its locked buttons, not as an empty frame (box
    // frames, 2026-11-19). A new offer, or an account boundary, ends that.
    [Fact]
    public async Task TheArmedCardLeavesDrawingWhatItLastShowed()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();
        rig.Answer(Armed);
        rig.Hold.SetResult();
        await pressed;

        Assert.Null(rig.Flow.Offer);
        Assert.Equal(Api, rig.Flow.DrawnOffer);
        Assert.True(rig.Flow.Leaving);

        rig.Flow.OnRefreshed(Web);
        Assert.Equal(Web, rig.Flow.DrawnOffer);
        Assert.False(rig.Flow.Leaving);

        rig.Flow.AccountBoundary();
        Assert.Null(rig.Flow.DrawnOffer);
        Assert.False(rig.Flow.Leaving);
    }

    [Fact]
    public async Task ADeclinedCardDrawsNothingOnceItHasGone()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task declined = rig.Flow.DeclineAsync();
        rig.Answer(Declined);
        await declined;

        Assert.Null(rig.Flow.DrawnOffer);
        Assert.False(rig.Flow.Leaving);
    }

    [Fact]
    public async Task AFailedReReadNeverStrandsTheCheckedCard()
    {
        var rig = new Rig { RefreshOverride = () => throw new System.IO.IOException("pipe closed") };
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Armed);
        Assert.Equal(ArmingOfferStep.Armed, rig.Flow.Step);
        rig.Hold.SetResult();
        await pressed;

        Assert.Null(rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
        Assert.True(rig.Flow.LeftArmed);
    }

    [Fact]
    public async Task ASlowReReadDoesNotHoldTheCardPastTheHold()
    {
        var never = new TaskCompletionSource();
        var rig = new Rig { RefreshOverride = () => never.Task };
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Armed);
        // The hold started at the reply, not after the re-read.
        Assert.Equal(TimeSpan.FromMilliseconds(900), rig.HeldFor);
        rig.Hold.SetResult();
        await pressed;

        Assert.Null(rig.Flow.Offer);
    }

    [Fact]
    public async Task OnlyAnArmedCardLeavesOnTheArmingExit()
    {
        var rig = new Rig();
        bool leftArmedWhenItWent = false;
        rig.Flow.Changed += () =>
        {
            if (rig.Flow.Offer is null)
            {
                leftArmedWhenItWent = rig.Flow.LeftArmed;
            }
        };
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();
        rig.Answer(Armed);
        rig.Hold.SetResult();
        await pressed;
        Assert.True(leftArmedWhenItWent);

        rig.Flow.OnRefreshed(Web);
        Assert.False(rig.Flow.LeftArmed);
        Task declined = rig.Flow.DeclineAsync();
        rig.Answer(Declined);
        await declined;
        Assert.False(leftArmedWhenItWent);
    }

    [Fact]
    public async Task ARefusalKeepsTheCardUnlocksEnableAndSaysTheCoresLine()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Refused("project-id-unrecognized", "bad_params"));
        await pressed;

        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
        Assert.False(rig.Flow.EnableLocked);
        Assert.Equal("Couldn't change this folder's setting.", rig.Flow.Refusal);
        Assert.Equal(0, rig.Refreshes);
    }

    [Fact]
    public async Task ATransportFailureIsARefusalAndNeverEscapesTheClickHandler()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Throw();
        await pressed;

        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
        Assert.NotEmpty(rig.Flow.Refusal);
    }

    [Theory]
    [InlineData("policy-write-failed")]
    [InlineData("queue-write-failed")]
    public async Task AWriteFailureIsARefusalLikeAnyOtherAndNothingIsReRead(string label)
    {
        // The Mac shows its refusal for every failure and does not re-read
        // (acceptArmingOffer's onFailure, AppModel.swift:2222-2238): the card
        // and its line stay, even though the daemon armed the folder in
        // memory and its next answer would no longer offer it.
        var rig = new Rig { RefreshAnswers = true, RefreshAnswer = null };
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Answer(Refused(label));
        await pressed;

        Assert.Equal(0, rig.Refreshes);
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
        Assert.Equal("Couldn't change this folder's setting.", rig.Flow.Refusal);
    }

    [Fact]
    public async Task TheRefusalLineLeavesWhenTheOfferItWasAboutDoes()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();
        rig.Answer(Refused("project-id-unrecognized", "bad_params"));
        await pressed;

        rig.Flow.OnRefreshed(Api);
        Assert.NotEmpty(rig.Flow.Refusal);

        rig.Flow.OnRefreshed(Web);
        Assert.Equal(string.Empty, rig.Flow.Refusal);
        Assert.Equal(Web, rig.Flow.Offer);
    }

    [Fact]
    public async Task ASuccessfulEnableAfterARefusalClearsTheLine()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task first = rig.Flow.AcceptAsync();
        rig.Answer(Refused("project-id-unrecognized", "bad_params"));
        await first;

        Task second = rig.Flow.AcceptAsync();
        Assert.Equal(string.Empty, rig.Flow.Refusal);
        rig.Answer(Armed);
        Assert.Equal(string.Empty, rig.Flow.Refusal);
        rig.Hold.SetResult();
        await second;
    }

    [Fact]
    public async Task AReplyFromBeforeAnAccountBoundaryIsNotApplied()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.AcceptAsync();

        rig.Flow.AccountBoundary();
        rig.Flow.OnRefreshed(Api);
        rig.Answer(Armed);
        rig.Hold.TrySetResult();
        await pressed;

        // The new account's card is the daemon's fresh answer, untouched by
        // the old press: not locked, not checked, nothing re-read.
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.Null(rig.Flow.Step);
        Assert.Equal(0, rig.Refreshes);
        Assert.Null(rig.HeldFor);
    }

    [Fact]
    public async Task NotNowLeavesOnlyOnTheDaemonsYes()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.DeclineAsync();

        Assert.Equal(("decline_arming", """{"project_id":"p-api"}"""), Assert.Single(rig.Calls));
        Assert.Equal(Api, rig.Flow.Offer);
        Assert.True(rig.Flow.DeclineLocked);
        rig.Answer(Declined);
        await pressed;
        Assert.Null(rig.Flow.Offer);
    }

    [Fact]
    public async Task ARefusedNotNowKeepsTheCard()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.DeclineAsync();

        rig.Answer(Refused("project-id-unrecognized", "bad_params"));
        await pressed;

        Assert.Equal(Api, rig.Flow.Offer);
        Assert.False(rig.Flow.DeclineLocked);
    }

    /// <summary>
    /// The interleave of checklist L1: Not now on api is out, a refresh
    /// brings the web offer, then api's reply lands. It must not take web's
    /// card with it.
    /// </summary>
    [Fact]
    public async Task AnOldNotNowReplyDoesNotDismissANewerOffer()
    {
        var rig = new Rig();
        rig.Flow.OnRefreshed(Api);
        Task pressed = rig.Flow.DeclineAsync();

        rig.Flow.OnRefreshed(Web);
        rig.Answer(Declined);
        await pressed;

        Assert.Equal(Web, rig.Flow.Offer);
    }

    [Fact]
    public async Task NoOfferSendsNothing()
    {
        var rig = new Rig();
        await rig.Flow.AcceptAsync();
        await rig.Flow.DeclineAsync();
        Assert.Empty(rig.Calls);
    }

    /// <summary>
    /// The line is the core's, as the Mac shows it
    /// (AppModel.swift:4752-4756; shell_words_copy.rs <c>project_mode_failed</c>).
    /// </summary>
    [Fact]
    public void TheRefusalLineIsTheCoresProjectModeFailed()
    {
        Assert.Equal("Couldn't change this folder's setting.", MonitorWords.ProjectModeFailed);
    }

    /// <summary>
    /// The card's words come from <c>tc_arming_offer_copy_json</c>
    /// (project_copy.rs <c>arming_offer_copy</c>), the table the Mac decodes.
    /// </summary>
    [Fact]
    public void TheCardsWordsAreTheCores()
    {
        ProjectArmingCopy copy = Assert.IsType<ProjectArmingCopy>(ProjectArmingCopy.For(Api));
        Assert.Equal("You've contributed from api 5 times.", copy.Evidence);
        Assert.Equal("Contribute from api automatically?", copy.Question);
        Assert.Equal("Enable", copy.Confirm);
        Assert.Equal("Not now", copy.Decline);
        Assert.NotEmpty(copy.Body);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("not json")]
    [InlineData("""{"evidence":"e","question":"q","confirm":"c","decline":"","body":"b"}""")]
    [InlineData("""{"evidence":"e","question":"q","confirm":"c","decline":"d"}""")]
    public void AnUnreadableTableDrawsNoCard(string? json)
    {
        Assert.Null(ProjectArmingCopy.Decode(json));
    }
}
