using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Adding Train coding models to sent traces: the pure logic, the two IPC
/// calls and the model, ported from the Mac's
/// <c>SentTracesTrainingSurfaceTests</c>, <c>SentTracesTrainingClientTests</c>
/// (macos/Tests/TCShellCoreTests/SentTracesTrainingSurfaceTests.swift) and
/// <c>SentTracesTrainingTests</c> (macos/Tests/TraceCommonsAppTests/SentTracesTrainingTests.swift).
/// The words are placeholders ("T:" prefixed), so a test passes only by
/// reading the payload.
/// </summary>
public class SentTracesTrainingTests
{
    private static string CopyJson(IDictionary<string, string?>? overrides = null, string? remove = null)
    {
        var obj = new SortedDictionary<string, string?>(StringComparer.Ordinal);
        foreach (string field in SentTracesTrainingCopy.ConsumedFields)
        {
            obj[field] = $"T:{field} {{count}}";
        }

        foreach (var (key, value) in overrides ?? new Dictionary<string, string?>())
        {
            obj[key] = value;
        }

        if (remove is not null)
        {
            obj.Remove(remove);
        }

        return JsonSerializer.Serialize(obj);
    }

    private static readonly SentTracesTrainingCopy Copy = SentTracesTrainingCopy.Decode(CopyJson())!;

    private static ModelTrainingConsentStatus Status(ulong count, params string[] ids) => new(count, ids);

    private static ModelTrainingGrantResult Result(string id, string outcome, string award, string? reason = null) =>
        new(id, outcome, null, award, reason);

    // MARK: SentTracesTrainingSurfaceTests

    [Fact]
    public void APayloadWithAMissingOrBlankFieldIsRefusedWhole()
    {
        Assert.Null(SentTracesTrainingCopy.Decode(CopyJson(new Dictionary<string, string?> { ["pending"] = "  " })));
        Assert.Null(SentTracesTrainingCopy.Decode(CopyJson(remove: "not_now_button")));
        Assert.Null(SentTracesTrainingCopy.Decode("not json"));
        Assert.Null(SentTracesTrainingCopy.Decode(null));
    }

    [Fact]
    public void OneTraceReadsTheOneSentenceAndMoreReadTheCountedOne()
    {
        Assert.Equal("T:body_one {count}", Copy.Body(1));
        Assert.Equal("T:body_other 7", Copy.Body(7));
        Assert.Equal("T:line_one {count}", Copy.Line(1));
        Assert.Equal("T:line_other 12", Copy.Line(12));
        Assert.Equal("T:allow_accessibility_name 1", Copy.AllowAccessibilityNameFor(1));
    }

    [Fact]
    public void OnlyATurnOnRaisesTheOffer()
    {
        string scope = SentTracesTrainingState.Scope;
        Assert.Equal("model_training", scope);
        Assert.True(SentTracesTrainingState.TurnsOn(new[] { "benchmark_only" }, new[] { "benchmark_only", scope }));
        Assert.False(SentTracesTrainingState.TurnsOn(new[] { scope }, new[] { scope }));
        Assert.False(SentTracesTrainingState.TurnsOn(new[] { scope }, Array.Empty<string>()));
        Assert.False(SentTracesTrainingState.TurnsOn(Array.Empty<string>(), new[] { "benchmark_only" }));
    }

    [Fact]
    public void NothingIsDrawnUntilTracesAreEligible()
    {
        var state = new SentTracesTrainingState();
        Assert.Null(state.Count);
        Assert.Null(state.Sheet(SentTracesTrainingSite.Settings));
        state.Apply(Status(0));
        Assert.Equal(SentTracesTrainingEligibility.Hidden, state.Eligibility);
        Assert.Null(state.Sheet(SentTracesTrainingSite.Settings));
        state.Apply(Status(3, "a", "b", "c"));
        Assert.Equal(3UL, state.Count);
        Assert.Equal(new SentTracesTrainingSheet(SentTracesTrainingSite.Settings, 3), state.Sheet(SentTracesTrainingSite.Settings));
        Assert.True(state.Offers("a"));
        Assert.False(state.Offers("z"));
        state.ApplyStatusRefusal();
        Assert.Null(state.Count);
        Assert.False(state.Offers("a"));
    }

    [Fact]
    public void RefusalsRoute()
    {
        foreach (string hidden in new[] { "model-training-not-permitted", "model-training-consent-unavailable", "not-logged-in" })
        {
            Assert.Equal(SentTracesTrainingRefusal.Hidden, SentTracesTrainingRefusals.From(hidden));
        }

        Assert.Equal(SentTracesTrainingRefusal.Failed, SentTracesTrainingRefusals.From("model-training-consent-failed"));
        Assert.Equal(SentTracesTrainingRefusal.Failed, SentTracesTrainingRefusals.From("anything-new"));
    }

    [Fact]
    public void AGrantMarksPendingOnlyWhereCreditFollows()
    {
        var state = new SentTracesTrainingState();
        state.Apply(Status(5, "a", "b", "c", "d", "e"));
        state.Apply(new ModelTrainingGrantAnswer(new[]
        {
            Result("a", "granted", "credited"),
            Result("b", "granted", "pending"),
            Result("c", "granted", "none", "below_floor"),
            Result("d", "not_eligible", "none"),
        }, 1));
        Assert.Equal(SentTraceTrainingMark.Pending, state.Mark("a", credited: false));
        Assert.Equal(SentTraceTrainingMark.Pending, state.Mark("b", credited: false));
        Assert.Null(state.Mark("c", credited: false));
        Assert.Null(state.Mark("d", credited: false));
        foreach (string answered in new[] { "a", "b", "c", "d" })
        {
            Assert.False(state.Offers(answered), answered);
        }

        Assert.True(state.Offers("e"));
        Assert.Equal(1UL, state.Count);
    }

    [Fact]
    public void PendingLastsUntilTheHistoryRowShowsCredit()
    {
        var state = new SentTracesTrainingState();
        state.Apply(Status(1, "a"));
        state.Apply(new ModelTrainingGrantAnswer(new[] { Result("a", "granted", "pending") }, 0));
        Assert.Null(state.Count);
        Assert.False(SentTracesTrainingState.Credited(null));
        Assert.False(SentTracesTrainingState.Credited(0));
        Assert.True(SentTracesTrainingState.Credited(1.5));
        Assert.Equal(SentTraceTrainingMark.Pending, state.Mark("a", credited: false));
        Assert.Null(state.Mark("a", credited: true));
    }

    /// <summary>
    /// A status read that still lists a trace whose grant is waiting on its
    /// credit does not offer it again (SentTracesTrainingSurface.swift:262-265).
    /// </summary>
    [Fact]
    public void ATraceWaitingOnItsCreditIsNotOfferedAgainByALaterRead()
    {
        var state = new SentTracesTrainingState();
        state.Apply(Status(2, "a", "b"));
        state.Apply(new ModelTrainingGrantAnswer(new[] { Result("a", "granted", "pending") }, 1));
        state.Apply(Status(2, "a", "b"));
        Assert.False(state.Offers("a"));
        Assert.True(state.Offers("b"));
        Assert.Equal(SentTraceTrainingMark.Pending, state.Mark("a", credited: false));
    }

    [Fact]
    public void ARefusedTraceGrantSaysFailedAndIsOfferedAgain()
    {
        var state = new SentTracesTrainingState();
        state.Apply(Status(2, "a", "b"));
        state.ApplyGrantRefusal(SentTracesTrainingRefusal.Failed, new[] { "a" });
        Assert.Equal(SentTraceTrainingMark.Failed, state.Mark("a", credited: false));
        Assert.True(state.Offers("a"));
        Assert.Null(state.Mark("b", credited: false));
        state.ApplyGrantRefusal(SentTracesTrainingRefusal.Hidden, new[] { "b" });
        Assert.Null(state.Count);
        Assert.False(state.Offers("a"));
        Assert.Null(state.Mark("a", credited: false));
    }

    /// <summary>The live export decodes, and carries exactly the fields this shell reads (TCBridgeTests).</summary>
    [Fact]
    public void TheLiveExportCarriesExactlyTheFieldsThisShellDecodes()
    {
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_sent_traces_training_copy_json())
            ?? throw new InvalidOperationException("tc_sent_traces_training_copy_json returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        Assert.Equal(
            SentTracesTrainingCopy.ConsumedFields.OrderBy(f => f, StringComparer.Ordinal),
            document.RootElement.EnumerateObject().Select(p => p.Name).OrderBy(f => f, StringComparer.Ordinal));
        Assert.NotNull(SentTracesTrainingCopy.Decode(json));
        Assert.NotNull(SentTracesTrainingCopy.Current);
    }

    // MARK: SentTracesTrainingClientTests

    private sealed class Fake
    {
        public readonly List<(string Method, JsonElement Params)> Calls = new();
        private readonly Queue<string> _replies;

        public Fake(params string[] replies) => _replies = new Queue<string>(replies);

        public Task<DaemonResponse> Call(string method, string paramsJson)
        {
            Calls.Add((method, JsonDocument.Parse(paramsJson).RootElement.Clone()));
            return Task.FromResult(DaemonResponse.Parse(_replies.Count == 0 ? "{\"id\":1,\"result\":{}}" : _replies.Dequeue()));
        }

        public string[] Keys(int index) =>
            Calls[index].Params.EnumerateObject().Select(p => p.Name).OrderBy(n => n, StringComparer.Ordinal).ToArray();
    }

    [Fact]
    public async Task StatusSendsNothingAndDecodes()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"eligible_count\":612,\"submission_ids\":[\"00000000-0000-4000-8000-000000000001\"],\"future\":1}}");
        ModelTrainingConsentStatus status = await new SentTracesTrainingClient(fake.Call).StatusAsync();
        Assert.Equal(new[] { "model_training_consent_status" }, fake.Calls.Select(c => c.Method));
        Assert.Empty(fake.Keys(0));
        Assert.Equal(612UL, status.EligibleCount);
        Assert.Equal(new[] { "00000000-0000-4000-8000-000000000001" }, status.SubmissionIds);
    }

    [Fact]
    public async Task AStatusWithoutIdsReadsAsNone()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"eligible_count\":2}}");
        ModelTrainingConsentStatus status = await new SentTracesTrainingClient(fake.Call).StatusAsync();
        Assert.Equal(2UL, status.EligibleCount);
        Assert.Empty(status.SubmissionIds);
    }

    [Fact]
    public async Task GrantAllSendsOnlyAllEligible()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"results\":[{\"submission_id\":\"a\",\"outcome\":\"granted\",\"granted_at\":\"2026-10-28T00:00:00Z\",\"award\":\"pending\"}],\"remaining_eligible\":0}}");
        ModelTrainingGrantAnswer answer = await new SentTracesTrainingClient(fake.Call).GrantAllAsync();
        Assert.Equal("model_training_consent_grant", fake.Calls[0].Method);
        Assert.Equal(new[] { "all_eligible" }, fake.Keys(0));
        Assert.True(fake.Calls[0].Params.GetProperty("all_eligible").GetBoolean());
        Assert.Equal("2026-10-28T00:00:00Z", answer.Results[0].GrantedAt);
        Assert.Equal(0UL, answer.RemainingEligible);
    }

    [Fact]
    public async Task GrantByIdSendsOnlyThoseIds()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"results\":[{\"submission_id\":\"a\",\"outcome\":\"granted\",\"award\":\"none\",\"award_withheld_reason\":\"x\"}],\"remaining_eligible\":4}}");
        ModelTrainingGrantAnswer answer = await new SentTracesTrainingClient(fake.Call).GrantAsync(new[] { "a" });
        Assert.Equal("model_training_consent_grant", fake.Calls[0].Method);
        Assert.Equal(new[] { "submission_ids" }, fake.Keys(0));
        Assert.Equal(new[] { "a" }, fake.Calls[0].Params.GetProperty("submission_ids").EnumerateArray().Select(e => e.GetString()));
        Assert.Equal("x", answer.Results[0].AwardWithheldReason);
        Assert.False(answer.Results[0].CreditFollows);
    }

    [Fact]
    public async Task ARefusalCarriesItsLabel()
    {
        var fake = new Fake("{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"model-training-not-permitted\"}}");
        var hidden = await Assert.ThrowsAsync<SentTracesTrainingFailure>(() => new SentTracesTrainingClient(fake.Call).StatusAsync());
        Assert.Equal(SentTracesTrainingRefusal.Hidden, hidden.Refusal);
        var failing = new Fake("{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"model-training-consent-failed\"}}");
        var failed = await Assert.ThrowsAsync<SentTracesTrainingFailure>(() => new SentTracesTrainingClient(failing.Call).GrantAllAsync());
        Assert.Equal(SentTracesTrainingRefusal.Failed, failed.Refusal);
    }

    [Fact]
    public async Task AnUnreadableAnswerIsARefusalNotAZero()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"eligible_count\":\"many\"}}");
        var error = await Assert.ThrowsAsync<SentTracesTrainingFailure>(() => new SentTracesTrainingClient(fake.Call).StatusAsync());
        Assert.Equal("unparseable-response", error.Label);
    }

    // MARK: SentTracesTrainingTests (the model)

    private static SentTracesTrainingModel Model(Fake fake) => new(fake.Call, Copy);

    private const string ThreeEligible = "{\"id\":1,\"result\":{\"eligible_count\":3,\"submission_ids\":[\"a\",\"b\",\"c\"]}}";

    [Fact]
    public void TheTwoChoicesShareOneStyleAndNeitherIsTheDefault()
    {
        IReadOnlyList<SentTracesTrainingAction> actions = SentTracesTrainingModel.Actions(Copy, 4, busy: false);
        Assert.Equal(new[] { "T:not_now_button {count}", "T:allow_button {count}" }, actions.Select(a => a.Title));
        Assert.Equal(new[] { SentTracesTrainingChoice.NotNow, SentTracesTrainingChoice.Allow }, actions.Select(a => a.Choice));
        foreach (SentTracesTrainingAction action in actions)
        {
            Assert.False(action.IsDefault, action.Title);
            Assert.True(action.IsEnabled, action.Title);
        }

        Assert.Equal("T:not_now_accessibility_name {count}", actions[0].AccessibilityName);
        Assert.Equal("T:allow_accessibility_name 4", actions[1].AccessibilityName);
    }

    [Fact]
    public void BothChoicesWaitForAGrantInFlight()
    {
        Assert.All(SentTracesTrainingModel.Actions(Copy, 4, busy: true), a => Assert.False(a.IsEnabled));
    }

    [Fact]
    public void NothingIsRaisedBeforeTheDaemonCounts()
    {
        var fake = new Fake();
        SentTracesTrainingModel model = Model(fake);
        Assert.Null(model.Sheet);
        model.Review();
        Assert.Null(model.Sheet);
        model.NotNow();
        Assert.Null(model.Sheet);
        Assert.False(model.Busy);
        Assert.Empty(fake.Calls);
    }

    [Fact]
    public async Task ATurnOnRaisesTheSheetOnceAtItsSiteAndSendsNoGrant()
    {
        var fake = new Fake(ThreeEligible, ThreeEligible);
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        Assert.Equal(new SentTracesTrainingSheet(SentTracesTrainingSite.Settings, 3), model.Sheet);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Monitor);
        Assert.Equal(SentTracesTrainingSite.Settings, model.Sheet?.Site);
        Assert.All(fake.Calls, c => Assert.Equal("model_training_consent_status", c.Method));
    }

    [Fact]
    public async Task NotNowClosesTheSheetAndSendsNothing()
    {
        var fake = new Fake(ThreeEligible);
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        model.NotNow();
        Assert.Null(model.Sheet);
        Assert.Single(fake.Calls);
        Assert.Equal(3UL, model.State.Count);
    }

    [Fact]
    public async Task NoTurnOnRaisesNothingWhenNoneIsEligible()
    {
        var fake = new Fake("{\"id\":1,\"result\":{\"eligible_count\":0}}");
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        Assert.Null(model.Sheet);
    }

    [Fact]
    public async Task AllowAllSendsTheGrantClosesTheSheetAndReadsAgain()
    {
        var fake = new Fake(
            ThreeEligible,
            "{\"id\":1,\"result\":{\"results\":[{\"submission_id\":\"a\",\"outcome\":\"granted\",\"award\":\"pending\"}],\"remaining_eligible\":0}}",
            "{\"id\":1,\"result\":{\"eligible_count\":0}}");
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        await model.AllowAllAsync();
        Assert.Equal(new[] { "model_training_consent_status", "model_training_consent_grant", "model_training_consent_status" },
            fake.Calls.Select(c => c.Method));
        Assert.Null(model.Sheet);
        Assert.False(model.Busy);
        Assert.Equal(SentTraceTrainingMark.Pending, model.State.Mark("a", credited: false));
    }

    [Fact]
    public async Task AllowAllWithNoSheetSendsNothing()
    {
        var fake = new Fake();
        SentTracesTrainingModel model = Model(fake);
        await model.AllowAllAsync();
        Assert.Empty(fake.Calls);
    }

    [Fact]
    public async Task ARefusedGrantKeepsTheSheetUpAndSaysSo()
    {
        var fake = new Fake(ThreeEligible, "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"model-training-consent-failed\"}}");
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        await model.AllowAllAsync();
        Assert.NotNull(model.Sheet);
        Assert.True(model.SheetFailed);
    }

    [Fact]
    public async Task AHidingRefusalClosesTheSheet()
    {
        var fake = new Fake(ThreeEligible, "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"model-training-not-permitted\"}}");
        SentTracesTrainingModel model = Model(fake);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        await model.AllowAllAsync();
        Assert.Null(model.Sheet);
        Assert.Null(model.State.Count);
    }

    [Fact]
    public async Task AllowOnOneTraceSendsOnlyThatTrace()
    {
        var fake = new Fake(ThreeEligible,
            "{\"id\":1,\"result\":{\"results\":[{\"submission_id\":\"b\",\"outcome\":\"granted\",\"award\":\"credited\"}],\"remaining_eligible\":2}}");
        SentTracesTrainingModel model = Model(fake);
        await model.RefreshAsync();
        await model.AllowAsync("z");
        Assert.Single(fake.Calls);
        await model.AllowAsync("b");
        Assert.Equal(new[] { "submission_ids" }, fake.Keys(1));
        Assert.Equal("b", fake.Calls[1].Params.GetProperty("submission_ids")[0].GetString());
        Assert.Equal(SentTraceTrainingMark.Pending, model.State.Mark("b", credited: false));
        Assert.Null(model.Sheet);
    }

    [Fact]
    public async Task ARefusedStatusHidesEverything()
    {
        var fake = new Fake("{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"sample-unrecorded\"}}");
        SentTracesTrainingModel model = Model(fake);
        await model.RefreshAsync();
        Assert.Equal(SentTracesTrainingEligibility.Hidden, model.State.Eligibility);
        model.Review();
        Assert.Null(model.Sheet);
    }

    [Fact]
    public async Task NoCopyRaisesNothingAndReadsNothing()
    {
        var fake = new Fake(ThreeEligible);
        var model = new SentTracesTrainingModel(fake.Call, copy: null);
        await model.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        Assert.Null(model.Sheet);
        Assert.Empty(fake.Calls);
    }

    [Fact]
    public async Task AnOlderReadNeverOverwritesANewerOne()
    {
        var first = new TaskCompletionSource<DaemonResponse>();
        int calls = 0;
        var model = new SentTracesTrainingModel((_, _) =>
            ++calls == 1 ? first.Task : Task.FromResult(DaemonResponse.Parse("{\"id\":1,\"result\":{\"eligible_count\":0}}")), Copy);
        Task older = model.RefreshAsync();
        await model.RefreshAsync();
        first.SetResult(DaemonResponse.Parse(ThreeEligible));
        await older;
        Assert.Equal(SentTracesTrainingEligibility.Hidden, model.State.Eligibility);
    }

    [Fact]
    public void OnlyAConfirmedWriteSaysWhichScopesAreOn()
    {
        Assert.Equal(new[] { "benchmark_only", "model_training" }, SentTracesTrainingState.ConfirmedScopes(
            DaemonResponse.Parse("{\"id\":1,\"result\":{\"consent_scopes\":[\"benchmark_only\",\"model_training\"]}}")));
        Assert.Null(SentTracesTrainingState.ConfirmedScopes(
            DaemonResponse.Parse("{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"not-logged-in\"}}")));
        Assert.Null(SentTracesTrainingState.ConfirmedScopes(DaemonResponse.Parse("{\"id\":1,\"result\":{}}")));
        Assert.Null(SentTracesTrainingState.ConfirmedScopes(DaemonResponse.Parse("{\"id\":1,\"result\":{\"consent_scopes\":[1]}}")));
    }

    private static string Shell(string relative) => File.ReadAllText(
        Path.Combine(AppContext.BaseDirectory, "shell-source", relative + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    /// <summary>
    /// A confirmed turn-on in Settings raises the sheet in the Settings window,
    /// only after the daemon confirmed the write (<c>test_aConfirmedTurnOnRaisesTheSheet</c>).
    /// </summary>
    [Fact]
    public void AConfirmedTurnOnRaisesTheSheetInSettings()
    {
        string settings = Shell("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        int save = settings.IndexOf("public async Task SaveConsentAsync()", StringComparison.Ordinal);
        int error = settings.IndexOf("if (response.IsError)", save, StringComparison.Ordinal);
        int confirmed = settings.IndexOf("SentTracesTrainingState.ConfirmedScopes(response)", save, StringComparison.Ordinal);
        int raised = settings.IndexOf("ConsentScopesConfirmed?.Invoke(before, confirmed);", save, StringComparison.Ordinal);
        Assert.True(save > 0 && error > save && confirmed > error && raised > confirmed);

        string window = Shell("TraceCommons.App/MainWindow.SentTracesTraining.cs");
        int turnsOn = window.IndexOf("SentTracesTrainingState.TurnsOn(before, confirmed)", StringComparison.Ordinal);
        int offer = window.IndexOf("OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings)", StringComparison.Ordinal);
        Assert.True(turnsOn > 0 && offer > turnsOn);
        Assert.Contains("settings.Settings.ConsentScopesConfirmed += (before, confirmed) =>", Shell("TraceCommons.App/MainWindow.Shell.cs"));
    }

    /// <summary>
    /// The line sits under Home's Insights card; the inspector offers Allow on
    /// the opened sent trace (<c>test_theLineAndTheInspectorAreWired</c>).
    /// </summary>
    [Fact]
    public void TheLineAndTheInspectorAreWired()
    {
        string home = Shell("TraceCommons.App/Controls/HomeView.cs");
        int heroes = home.IndexOf("_column.Children.Add(BuildHero(hero, model));", StringComparison.Ordinal);
        int line = home.IndexOf("_column.Children.Add(_trainingSlot);", StringComparison.Ordinal);
        int catalogue = home.IndexOf("if (model.Catalogue is HomeCard catalogue)", StringComparison.Ordinal);
        Assert.True(heroes > 0 && line > heroes && catalogue > line);
        Assert.Contains("_ = RefreshSentTrainingLineAsync();", Shell("TraceCommons.App/MainWindow.Home.cs"));

        string inspector = Shell("TraceCommons.App/Controls/HistoryDetailInspector.cs");
        int summary = inspector.IndexOf("_root.Children.Add(Summary(copy));", StringComparison.Ordinal);
        int section = inspector.IndexOf("_root.Children.Add(_trainingSlot);", StringComparison.Ordinal);
        Assert.True(summary > 0 && section > summary);
        Assert.Contains("new HistoryDetailInspector(_host, record, SentTraining, SkillLearning)", Shell("TraceCommons.App/MainWindow.HistoryDetail.cs"));

        string views = Shell("TraceCommons.App/Controls/SentTracesTrainingViews.cs");
        Assert.Contains("AutomationProperties.SetName(button, copy.ReviewAccessibilityName);", views);
        Assert.Contains("AutomationProperties.SetName(allow, copy.TraceAccessibilityName);", views);
        Assert.Contains("await model.AllowAsync(submissionId);", views);
        Assert.Contains("DefaultButton = ContentDialogButton.None,", views);
        Assert.DoesNotContain("ContentDialogButton.Primary", views);
        Assert.DoesNotContain("ContentDialogButton.Secondary", views);
        Assert.DoesNotContain("TcPrimaryButtonStyle", views);
    }

    /// <summary>
    /// Only the person's choice sends a grant: the model's two grant calls sit
    /// in its two Allow handlers, and no other file calls them
    /// (<c>test_onlyAllowSendsAGrant</c>).
    /// </summary>
    [Fact]
    public void OnlyAllowSendsAGrant()
    {
        string sources = Path.Combine(AppContext.BaseDirectory, "shell-source");
        string model = File.ReadAllText(Path.Combine(sources, "TraceCommons.Interop", "SentTracesTraining.cs.txt"));
        int allowAll = model.IndexOf("public async Task AllowAllAsync()", StringComparison.Ordinal);
        int allowOne = model.IndexOf("public async Task AllowAsync(string submissionId)", StringComparison.Ordinal);
        int grantAll = model.IndexOf(".GrantAllAsync()", StringComparison.Ordinal);
        int grantIds = model.IndexOf(".GrantAsync(", StringComparison.Ordinal);
        Assert.True(allowAll > 0 && allowAll < grantAll && grantAll < allowOne && allowOne < grantIds);
        Assert.Equal(2, model.Split(".GrantAllAsync()").Length);
        Assert.Equal(2, model.Split(".GrantAsync(").Length);
        string[] others = Directory.EnumerateFiles(sources, "*.cs.txt", SearchOption.AllDirectories)
            .Where(f => !f.EndsWith("SentTracesTraining.cs.txt", StringComparison.Ordinal)).ToArray();
        Assert.NotEmpty(others);
        foreach (string file in others)
        {
            string text = File.ReadAllText(file);
            Assert.DoesNotContain(".GrantAllAsync()", text);
            Assert.DoesNotContain(".GrantAsync(", text);
        }
    }
}
