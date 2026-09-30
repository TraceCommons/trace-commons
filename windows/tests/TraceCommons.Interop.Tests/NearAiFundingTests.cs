// INTEGRATION: the existing test project includes this file automatically after
// NearAiFunding.cs and the Funding* copy fields are integrated. The copy below
// is synthetic and limited to this controller; no FFI, browser, or Cloud is used.
// Requires the fifth NearAiFunding constructor argument, Func<long> host generation.
// The held authorization failed before that guard; the initial-read case also
// asserts that a reply from the previous host cannot restore a displayed target.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class NearAiFundingTests
{
    private static readonly TimeSpan Deadline = TimeSpan.FromSeconds(5);
    private const string OrganizationA = "synthetic-organization-a";
    private const string OrganizationB = "synthetic-organization-b";
    private static readonly string RevisionA = new('a', 64);
    private static readonly string RevisionB = new('b', 64);
    private static readonly PrivateInferenceCopy Copy = new()
    {
        FundingTitle = "synthetic-title",
        FundingWhat = "synthetic-explanation",
        FundingManage = "synthetic-manage",
        FundingRefresh = "synthetic-refresh",
        FundingUnavailable = "synthetic-unavailable",
    };

    private static string Url(string organization) =>
        "https://cloud.near.ai/dashboard/organizations/" + organization + "/credits";

    private static Dictionary<string, object?> Payload(
        string organization = OrganizationA, string? revision = null, string state = "ready") => new()
    {
        ["state"] = state,
        ["view"] = new { message = "synthetic-current-organization" },
        ["organization_id"] = organization,
        ["connection_revision"] = revision ?? RevisionA,
        ["browser_url"] = Url(organization),
    };

    private static DaemonResponse Ready(string organization = OrganizationA, string? revision = null) =>
        Response(Payload(organization, revision));

    private static DaemonResponse Response(object value) => new()
    {
        Result = JsonSerializer.SerializeToElement(value),
    };

    private static void AssertExpected(string parameters, string organization, string revision)
    {
        using JsonDocument document = JsonDocument.Parse(parameters);
        Assert.Equal(2, document.RootElement.EnumerateObject().Count());
        Assert.Equal(organization, document.RootElement.GetProperty("expected_organization_id").GetString());
        Assert.Equal(revision, document.RootElement.GetProperty("expected_connection_revision").GetString());
    }

    private static void AssertCleared(NearAiFunding controller)
    {
        Assert.Equal(Copy.FundingRefresh, controller.Action);
        Assert.Equal(Copy.FundingUnavailable, controller.Message);
    }

    [Fact]
    public void ReadyStatusAcceptsBoundaryIdsAndCarriesOnlyTheDisplayedBinding()
    {
        foreach (string organization in new[] { "a", "org_under-score", new string('x', 128) })
        {
            FundingStatus status = Assert.IsType<FundingStatus>(
                FundingStatus.Parse(JsonSerializer.SerializeToElement(Payload(organization))));
            FundingDestination destination = Assert.IsType<FundingDestination>(status.Destination);
            Assert.Equal(Url(organization), destination.BrowserUri.AbsoluteUri);
            AssertExpected(destination.Parameters, organization, RevisionA);
        }
    }

    public static IEnumerable<object[]> InvalidReadyFields()
    {
        foreach (string organization in new[]
        {
            "", new string('x', 129), "../other", "org/other", "org%2Fother", "org.name",
            " org", "org\n", "org?next=other", "org#other", "orgé",
        }) yield return new object[] { "organization_id", JsonSerializer.Serialize(organization) };
        foreach (string revision in new[]
        {
            "", new string('a', 63), new string('a', 65), new string('A', 64),
            new string('g', 64), new string('a', 63) + "\n",
        }) yield return new object[] { "connection_revision", JsonSerializer.Serialize(revision) };
        foreach (string url in new[]
        {
            "http://cloud.near.ai/dashboard/organizations/" + OrganizationA + "/credits",
            "https://cloud.near.ai.attacker.invalid/dashboard/organizations/" + OrganizationA + "/credits",
            "https://cloud.near.ai:443/dashboard/organizations/" + OrganizationA + "/credits",
            "https://user@cloud.near.ai/dashboard/organizations/" + OrganizationA + "/credits",
            "https://CLOUD.NEAR.AI/dashboard/organizations/" + OrganizationA + "/credits",
            Url(OrganizationB), Url(OrganizationA) + "/", Url(OrganizationA) + "?next=other",
            Url(OrganizationA) + "#other", "/dashboard/organizations/" + OrganizationA + "/credits",
            "javascript:alert(1)", "not a URL",
        }) yield return new object[] { "browser_url", JsonSerializer.Serialize(url) };
        foreach (string field in new[] { "organization_id", "connection_revision", "browser_url" })
        {
            yield return new object[] { field, "null" };
            yield return new object[] { field, "42" };
        }
    }

    [Theory]
    [MemberData(nameof(InvalidReadyFields))]
    public void ReadyStatusRejectsMalformedBindingAndEveryNonExactUrl(string field, string json)
    {
        using JsonDocument document = JsonDocument.Parse(json);
        Dictionary<string, object?> payload = Payload();
        payload[field] = document.RootElement;
        Assert.Null(FundingStatus.Parse(JsonSerializer.SerializeToElement(payload)));
    }

    [Theory]
    [InlineData("invalid_request")]
    [InlineData("no_session")]
    [InlineData("no_inference_key")]
    [InlineData("no_organization")]
    [InlineData("session_expired")]
    [InlineData("changed")]
    [InlineData("unavailable")]
    [InlineData("future-unknown-state")]
    [InlineData("READY")]
    public void NonReadyStatesNeverExposeEvenAnOtherwiseValidDestination(string state)
    {
        FundingStatus? status = FundingStatus.Parse(JsonSerializer.SerializeToElement(Payload(state: state)));
        Assert.Null(status?.Destination);
    }

    [Theory]
    [InlineData("null")]
    [InlineData("[]")]
    [InlineData("{}")]
    [InlineData("{\"state\":7,\"view\":{\"message\":\"message\"}}")]
    [InlineData("{\"state\":\"ready\",\"view\":{\"message\":\"message\"}}")]
    [InlineData("{\"state\":\"unavailable\",\"view\":null}")]
    public void MalformedReportsAreRefusedWithoutThrowing(string json)
    {
        using JsonDocument document = JsonDocument.Parse(json);
        Assert.Null(FundingStatus.Parse(document.RootElement));
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1025)]
    public void EmptyAndOversizedMessagesAreRefused(int length)
    {
        Dictionary<string, object?> payload = Payload();
        payload["view"] = new { message = new string('x', length) };
        Assert.Null(FundingStatus.Parse(JsonSerializer.SerializeToElement(payload)));
    }

    [Fact]
    public async Task InitialReadNeverOpensAndClickRechecksBothDisplayedFields()
    {
        var fixture = new Fixture();
        await fixture.PrimeAsync();
        Assert.Equal(Copy.FundingTitle, fixture.Controller.Title);
        Assert.Equal(Copy.FundingWhat, fixture.Controller.What);
        Assert.Equal("{}", Assert.Single(fixture.Calls));
        Assert.Empty(fixture.Opened);
        fixture.Enqueue(Ready());
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        AssertExpected(fixture.Calls[1], OrganizationA, RevisionA);
        Assert.Equal(Url(OrganizationA), Assert.Single(fixture.Opened).AbsoluteUri);
        Assert.True(fixture.Controller.Enabled);
    }

    [Fact]
    public async Task DuplicateClickWhileAuthorizationIsHeldMakesOneCallAndOneOpen()
    {
        var fixture = new Fixture();
        await fixture.PrimeAsync();
        HeldReply held = fixture.Hold();
        Task pending = fixture.Controller.PressAsync();
        try
        {
            await held.Entered.Task.WaitAsync(Deadline);
            Assert.False(fixture.Controller.Enabled);
            await fixture.Controller.PressAsync().WaitAsync(Deadline);
            Assert.Equal(2, fixture.Calls.Count);
            Assert.Empty(fixture.Opened);
        }
        finally { held.Response.TrySetResult(Ready()); }
        await pending.WaitAsync(Deadline);
        AssertExpected(fixture.Calls[1], OrganizationA, RevisionA);
        Assert.Single(fixture.Opened);
    }

    [Theory]
    [InlineData(false, "credential-busy")]
    [InlineData(false, "forget")]
    [InlineData(false, "deactivate")]
    [InlineData(false, "shutdown")]
    [InlineData(true, "credential-busy")]
    [InlineData(true, "forget")]
    [InlineData(true, "deactivate")]
    [InlineData(true, "shutdown")]
    public async Task InvalidatedReadCannotOpenOrRestoreItsDestination(bool manageClick, string cause)
    {
        var fixture = new Fixture();
        fixture.Controller.CredentialChanged("present", false);
        if (manageClick) await fixture.PrimeAsync();
        HeldReply held = fixture.Hold();
        Task pending = manageClick ? fixture.Controller.PressAsync() : fixture.Controller.ActivateAsync();
        try
        {
            await held.Entered.Task.WaitAsync(Deadline);
            switch (cause)
            {
                case "credential-busy": fixture.Controller.CredentialChanged("present", true); break;
                case "forget": fixture.Controller.CredentialChanged("absent", false); break;
                case "deactivate": fixture.Controller.Deactivate(); break;
                case "shutdown": fixture.Stop(); break;
            }
        }
        finally { held.Response.TrySetResult(Ready()); }
        await pending.WaitAsync(Deadline);
        Assert.Empty(fixture.Opened);
        AssertCleared(fixture.Controller);
        if (cause != "forget") Assert.False(fixture.Controller.Enabled);

        fixture.Restart();
        fixture.Controller.CredentialChanged("present", false);
        fixture.Enqueue(Ready(OrganizationB, RevisionB));
        if (cause == "deactivate") await fixture.Controller.ActivateAsync().WaitAsync(Deadline);
        else await fixture.Controller.PressAsync().WaitAsync(Deadline);
        Assert.Equal("{}", fixture.Calls[^1]);
        Assert.Empty(fixture.Opened);
        fixture.Enqueue(Ready(OrganizationB, RevisionB));
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        AssertExpected(fixture.Calls[^1], OrganizationB, RevisionB);
        Assert.Equal(Url(OrganizationB), Assert.Single(fixture.Opened).AbsoluteUri);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task PendingAuthorizationCannotSurviveAHostRestart(bool manageClick)
    {
        var fixture = new Fixture();
        fixture.Controller.CredentialChanged("present", false);
        if (manageClick) await fixture.PrimeAsync();
        HeldReply held = fixture.Hold();
        Task pending = manageClick ? fixture.Controller.PressAsync() : fixture.Controller.ActivateAsync();
        try
        {
            await held.Entered.Task.WaitAsync(Deadline);
            fixture.Stop();
            Assert.False(fixture.Controller.Enabled);
            fixture.Restart();
        }
        finally { held.Response.TrySetResult(Ready()); }
        await pending.WaitAsync(Deadline);
        Assert.Empty(fixture.Opened);
        AssertCleared(fixture.Controller);
        fixture.Enqueue(Ready(OrganizationB, RevisionB));
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        Assert.Equal("{}", fixture.Calls[^1]);
        Assert.Empty(fixture.Opened);
    }

    [Theory]
    [InlineData("organization")]
    [InlineData("revision")]
    [InlineData("url")]
    [InlineData("refusal")]
    [InlineData("unknown-state")]
    [InlineData("daemon-error")]
    [InlineData("missing-result")]
    [InlineData("malformed-result")]
    [InlineData("transport-error")]
    [InlineData("browser-refused")]
    [InlineData("browser-error")]
    public async Task UnconfirmedClickClearsOldTargetAndRecoveryRefreshDoesNotOpen(string outcome)
    {
        var fixture = new Fixture();
        await fixture.PrimeAsync();
        switch (outcome)
        {
            case "organization": fixture.Enqueue(Ready(OrganizationB)); break;
            case "revision": fixture.Enqueue(Ready(revision: RevisionB)); break;
            case "url":
                Dictionary<string, object?> payload = Payload();
                payload["browser_url"] = Url(OrganizationB);
                fixture.Enqueue(Response(payload));
                break;
            case "refusal": fixture.Enqueue(Response(Payload(state: "changed"))); break;
            case "unknown-state": fixture.Enqueue(Response(Payload(state: "future-state"))); break;
            case "daemon-error": fixture.Enqueue(new DaemonResponse
                { Error = new DaemonError { Code = "unavailable", Message = "synthetic-refusal" } }); break;
            case "missing-result": fixture.Enqueue(new DaemonResponse()); break;
            case "malformed-result": fixture.Enqueue(Response(new { state = 42 })); break;
            case "transport-error": fixture.Replies.Enqueue(() => Task.FromException<DaemonResponse>(
                new InvalidOperationException("synthetic-transport-refusal"))); break;
            case "browser-refused": fixture.Enqueue(Ready()); fixture.Launch = _ => Task.FromResult(false); break;
            case "browser-error": fixture.Enqueue(Ready()); fixture.Launch = _ => Task.FromException<bool>(
                new InvalidOperationException("synthetic-browser-refusal")); break;
        }
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        AssertExpected(fixture.Calls[^1], OrganizationA, RevisionA);
        AssertCleared(fixture.Controller);
        Assert.True(fixture.Controller.Enabled);
        int failedLaunches = outcome.StartsWith("browser-", StringComparison.Ordinal) ? 1 : 0;
        Assert.Equal(failedLaunches, fixture.Opened.Count);
        fixture.Launch = _ => Task.FromResult(true);
        fixture.Enqueue(Ready(OrganizationB, RevisionB));
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        Assert.Equal("{}", fixture.Calls[^1]);
        Assert.Equal(failedLaunches, fixture.Opened.Count);
        fixture.Enqueue(Ready(OrganizationB, RevisionB));
        await fixture.Controller.PressAsync().WaitAsync(Deadline);
        AssertExpected(fixture.Calls[^1], OrganizationB, RevisionB);
        Assert.Equal(failedLaunches + 1, fixture.Opened.Count);
        Assert.Equal(Url(OrganizationB), fixture.Opened[^1].AbsoluteUri);
    }

    private sealed class HeldReply
    {
        public TaskCompletionSource<bool> Entered { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource<DaemonResponse> Response { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    }

    private sealed class Fixture
    {
        public bool Running { get; private set; } = true;
        public long ConnectionGeneration { get; private set; } = 1;
        public List<string> Calls { get; } = new();
        public List<Uri> Opened { get; } = new();
        public Queue<Func<Task<DaemonResponse>>> Replies { get; } = new();
        public Func<Uri, Task<bool>> Launch { get; set; } = _ => Task.FromResult(true);
        public NearAiFunding Controller { get; }

        public Fixture()
        {
            Controller = new NearAiFunding(Copy, parameters =>
            {
                Calls.Add(parameters);
                return Replies.Count == 0
                    ? Task.FromException<DaemonResponse>(new InvalidOperationException("unexpected-synthetic-request"))
                    : Replies.Dequeue()();
            }, () => Running, uri =>
            {
                Opened.Add(uri);
                return Launch(uri);
            }, () => ConnectionGeneration);
        }

        public void Enqueue(DaemonResponse response) => Replies.Enqueue(() => Task.FromResult(response));
        public void Stop() { Running = false; ConnectionGeneration++; }
        public void Restart() { Running = true; ConnectionGeneration++; }

        public HeldReply Hold()
        {
            var held = new HeldReply();
            Replies.Enqueue(() =>
            {
                held.Entered.TrySetResult(true);
                return held.Response.Task;
            });
            return held;
        }

        public async Task PrimeAsync()
        {
            Controller.CredentialChanged("present", false);
            Enqueue(Ready());
            await Controller.ActivateAsync().WaitAsync(Deadline);
            Assert.Equal(Copy.FundingManage, Controller.Action);
            Assert.True(Controller.Enabled);
            Assert.Empty(Opened);
        }
    }
}
