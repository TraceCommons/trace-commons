using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The move of a legacy invite identity to a NEAR AI account: the core's
/// words across the ABI, the offered flag, the dismissed-card marker, and the
/// passkey-first Move.
/// </summary>
public sealed class LegacyMigrationMoveTests
{
    private static JsonElement Json(string json) => JsonDocument.Parse(json).RootElement.Clone();

    private static DaemonResponse Ok(string result) =>
        DaemonResponse.Parse("{\"id\":1,\"result\":" + result + "}");

    private static DaemonResponse Refused(string label) =>
        DaemonResponse.Parse("{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"" + label + "\"}}");

    [Fact]
    public void TheOfferIsTheCoresWholeOfferAndNothingTypedHere()
    {
        LegacyMigrationOfferCopy copy = Assert.IsType<LegacyMigrationOfferCopy>(LegacyMigrationOffers.Copy());
        Assert.Equal("Move", copy.Action);
        Assert.False(string.IsNullOrEmpty(copy.Title));
        Assert.False(string.IsNullOrEmpty(copy.PasskeyWaiting));
        Assert.False(string.IsNullOrEmpty(copy.PasskeyCancel));
        Assert.False(string.IsNullOrEmpty(copy.Dismiss));
        Assert.False(string.IsNullOrEmpty(copy.Later));
    }

    [Fact]
    public void EveryFieldTheAbiSendsIsDecodedHere()
    {
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_legacy_migration_offer_json())
            ?? throw new InvalidOperationException("tc_legacy_migration_offer_json returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        string[] keys = document.RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k).ToArray();
        Assert.Equal(LegacyMigrationOfferCopy.ConsumedFields.OrderBy(k => k), keys);
    }

    [Fact]
    public void AnOfferWithABlankSentenceIsNotShown()
    {
        Assert.Null(LegacyMigrationOffers.Parse(null));
        Assert.Null(LegacyMigrationOffers.Parse("not json"));
        Assert.Null(LegacyMigrationOffers.Parse("{\"title\":\"T\"}"));
    }

    [Fact]
    public void TheRefusalLineIsTheCoresForTheLabel()
    {
        string line = Assert.IsType<string>(LegacyMigrationOffers.RefusalLine("legacy_migration_tenant_pooled"));
        Assert.Contains("shared by many people", line);
        // An unknown label still gets a sentence from the core.
        Assert.False(string.IsNullOrEmpty(LegacyMigrationOffers.RefusalLine("no_such_label")));
    }

    [Fact]
    public void OfferedIsLiterallyTrueOrNothing()
    {
        Assert.True(LegacyMigrationOffers.IsOffered(Json("{\"offered\":true,\"notice\":null}")));
        Assert.False(LegacyMigrationOffers.IsOffered(Json("{\"offered\":false}")));
        Assert.False(LegacyMigrationOffers.IsOffered(Json("{\"offered\":\"true\"}")));
        Assert.False(LegacyMigrationOffers.IsOffered(Json("{}")));
        Assert.False(LegacyMigrationOffers.IsOffered(null));
    }

    [Fact]
    public void TheInviteIsSentOnlyWhenThereIsOneAndTrimmed()
    {
        Assert.Equal("{}", LegacyMigrationOffers.MigrateParams(null));
        Assert.Equal("{}", LegacyMigrationOffers.MigrateParams("   "));
        Assert.Equal("{\"invite\":\"abc\"}", LegacyMigrationOffers.MigrateParams("  abc "));
    }

    [Fact]
    public void TheDigestIsSha256HexOfTheTenant()
    {
        Assert.Null(LegacyMigrationOffers.TenantDigest(null));
        Assert.Null(LegacyMigrationOffers.TenantDigest(""));
        Assert.Equal(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            LegacyMigrationOffers.TenantDigest("abc"));
    }

    [Fact]
    public void ADismissalIsPerTenantAndNeverWritesTheTenant()
    {
        string path = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N"), "dismissed.json");
        try
        {
            var store = new LegacyMigrationDismissals(path);
            Assert.False(store.IsDismissed("tenant-a"));
            store.Dismiss("tenant-a");
            Assert.True(new LegacyMigrationDismissals(path).IsDismissed("tenant-a"));
            Assert.False(store.IsDismissed("tenant-b"));
            Assert.DoesNotContain("tenant-a", File.ReadAllText(path));
            store.Dismiss(null);
            Assert.False(store.IsDismissed(null));
        }
        finally
        {
            if (File.Exists(path)) File.Delete(path);
        }
    }

    [Fact]
    public void AnUnreadableDismissalFileMeansNotDismissed()
    {
        string path = Path.GetTempFileName();
        try
        {
            File.WriteAllText(path, "{broken");
            Assert.False(new LegacyMigrationDismissals(path).IsDismissed("tenant-a"));
        }
        finally
        {
            File.Delete(path);
        }
    }

    /// <summary>A scripted daemon: answers by method, recording the calls.</summary>
    private sealed class Daemon
    {
        public readonly List<(string Method, string Params)> Calls = new();
        public readonly Queue<DaemonResponse> Migrations = new();
        public DaemonResponse Start = Ok("{\"attempt_id\":\"a1\",\"browser_url\":\"https://example.test/login\"}");
        public Func<int, DaemonResponse> Status = _ => Ok("{\"state\":\"x\",\"session_state\":\"present\",\"attempt_status\":null}");
        private int _statusReads;

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Calls.Add((method, parameters));
            return Task.FromResult(method switch
            {
                DaemonProtocol.Methods.LegacyInviteMigrate => Migrations.Dequeue(),
                DaemonProtocol.Methods.NearAiCredentialStart => Start,
                DaemonProtocol.Methods.NearAiCredentialStatus => Status(_statusReads++),
                _ => Ok("{}"),
            });
        }

        public string[] Methods => Calls.Select(c => c.Method).ToArray();
    }

    private static (LegacyMigrationMove Move, Daemon Daemon, List<Uri> Opened) Build(
        Func<Uri, Task<bool>>? open = null,
        Func<TimeSpan, CancellationToken, Task>? delay = null,
        TimeSpan? limit = null)
    {
        var daemon = new Daemon();
        var opened = new List<Uri>();
        var step = new NearAiPasskeyStep(
            daemon.Call,
            open ?? (uri => { opened.Add(uri); return Task.FromResult(true); }),
            delay ?? ((_, _) => Task.CompletedTask),
            TimeSpan.Zero,
            limit);
        var move = new LegacyMigrationMove(daemon.Call, step, label => "LINE:" + label);
        return (move, daemon, opened);
    }

    [Fact]
    public async Task AMoveThatSucceedsRunsNoPasskeyStep()
    {
        var (move, daemon, _) = Build();
        daemon.Migrations.Enqueue(Ok("{}"));
        int moved = 0;
        move.Moved += () => moved++;

        await move.MoveAsync(null);

        Assert.Equal(new[] { DaemonProtocol.Methods.LegacyInviteMigrate }, daemon.Methods);
        Assert.Equal(1, moved);
        Assert.False(move.Busy);
        Assert.Null(move.Refusal);
    }

    [Fact]
    public async Task NoSessionRunsThePasskeyStepThenRetriesOnce()
    {
        var (move, daemon, opened) = Build();
        daemon.Status = read => read == 0
            ? Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":null}")
            : Ok("{\"state\":\"x\",\"session_state\":\"present\",\"attempt_status\":null}");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));
        daemon.Migrations.Enqueue(Ok("{}"));
        int moved = 0;
        move.Moved += () => moved++;

        await move.MoveAsync(null);

        Assert.Equal(
            new[]
            {
                DaemonProtocol.Methods.LegacyInviteMigrate,
                DaemonProtocol.Methods.NearAiCredentialStatus,
                DaemonProtocol.Methods.NearAiCredentialStart,
                DaemonProtocol.Methods.NearAiCredentialStatus,
                DaemonProtocol.Methods.LegacyInviteMigrate,
            },
            daemon.Methods);
        Assert.Single(opened);
        Assert.Equal(1, moved);
        Assert.False(move.PasskeyFailed);
        Assert.False(move.AwaitingPasskey);
    }

    [Fact]
    public async Task ASecondNoSessionAfterThePasskeyIsARefusalNotALoop()
    {
        var (move, daemon, _) = Build();
        daemon.Status = read => read == 0
            ? Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":null}")
            : Ok("{\"state\":\"x\",\"session_state\":\"present\",\"attempt_status\":null}");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));

        await move.MoveAsync(null);

        Assert.Equal(2, daemon.Methods.Count(m => m == DaemonProtocol.Methods.LegacyInviteMigrate));
        Assert.Equal("LINE:" + LegacyMigrationOffers.PasskeyNeededLabel, move.Refusal);
    }

    [Fact]
    public async Task APasskeyStepThatEndsWithoutASessionMovesNothing()
    {
        var (move, daemon, _) = Build();
        daemon.Status = read => read == 0
            ? Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":null}")
            : Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":\"failed\"}");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));
        int moved = 0;
        move.Moved += () => moved++;

        await move.MoveAsync(null);

        Assert.Equal(1, daemon.Methods.Count(m => m == DaemonProtocol.Methods.LegacyInviteMigrate));
        Assert.True(move.PasskeyFailed);
        Assert.Null(move.Refusal);
        Assert.Equal(0, moved);
        Assert.False(move.Busy);
    }

    [Fact]
    public async Task APasskeyStepThatNeverStartsMovesNothing()
    {
        var (move, daemon, opened) = Build();
        daemon.Status = _ => Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":null}");
        daemon.Start = Refused("nearai_credential_unavailable");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));

        await move.MoveAsync(null);

        Assert.Empty(opened);
        Assert.True(move.PasskeyFailed);
        Assert.Equal(1, daemon.Methods.Count(m => m == DaemonProtocol.Methods.LegacyInviteMigrate));
    }

    [Fact]
    public async Task ABrowserThatWillNotOpenCancelsTheAttemptAndMovesNothing()
    {
        var (move, daemon, _) = Build(open: _ => Task.FromResult(false));
        daemon.Status = _ => Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":\"waiting_for_browser\"}");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));

        await move.MoveAsync(null);

        Assert.Contains(DaemonProtocol.Methods.NearAiCredentialCancel, daemon.Methods);
        var cancel = daemon.Calls.First(c => c.Method == DaemonProtocol.Methods.NearAiCredentialCancel);
        Assert.Contains("a1", cancel.Params);
        Assert.True(move.PasskeyFailed);
    }

    [Fact]
    public async Task CancelWhileWaitingEndsTheBrowserStepChangesNothingAndIsNotAFailure()
    {
        var gate = new TaskCompletionSource();
        Func<TimeSpan, CancellationToken, Task> delay = async (_, token) =>
        {
            await gate.Task.WaitAsync(token);
        };
        var (move, daemon, _) = Build(delay: delay);
        daemon.Status = _ => Ok("{\"state\":\"x\",\"session_state\":\"absent\",\"attempt_status\":\"waiting_for_browser\"}");
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.PasskeyNeededLabel));

        Task running = move.MoveAsync(null);
        for (int spin = 0; spin < 200 && !move.AwaitingPasskey; spin++)
        {
            await Task.Delay(5);
        }

        Assert.True(move.AwaitingPasskey);
        await move.CancelPasskeyAsync();
        await running;

        Assert.False(move.AwaitingPasskey);
        Assert.False(move.PasskeyFailed);
        Assert.Null(move.Refusal);
        Assert.False(move.Busy);
        Assert.Equal(1, daemon.Methods.Count(m => m == DaemonProtocol.Methods.LegacyInviteMigrate));
        Assert.Contains(DaemonProtocol.Methods.NearAiCredentialCancel, daemon.Methods);
    }

    [Fact]
    public async Task InviteNeededAsksThenTheNextMoveSendsTheInvite()
    {
        var (move, daemon, _) = Build();
        daemon.Migrations.Enqueue(Refused(LegacyMigrationOffers.InviteNeededLabel));
        daemon.Migrations.Enqueue(Ok("{}"));

        await move.MoveAsync("ignored-before-asked");
        Assert.True(move.InviteNeeded);
        Assert.Equal("LINE:" + LegacyMigrationOffers.InviteNeededLabel, move.Refusal);
        Assert.Equal("{}", daemon.Calls[0].Params);

        await move.MoveAsync("  tc-invite  ");
        Assert.Equal("{\"invite\":\"tc-invite\"}", daemon.Calls[1].Params);
        Assert.False(move.InviteNeeded);
        Assert.Null(move.Refusal);
    }

    [Fact]
    public async Task AFailureThatIsNotALabelIsStartFailedNotAnInventedSentence()
    {
        var (move, daemon, _) = Build();
        daemon.Migrations.Enqueue(Refused("daemon-not-started"));

        await move.MoveAsync(null);

        Assert.True(move.StartFailed);
        Assert.Null(move.Refusal);
    }

    [Fact]
    public async Task APressWhileBusyIsIgnored()
    {
        var gate = new TaskCompletionSource<DaemonResponse>();
        int asked = 0;
        var step = new NearAiPasskeyStep((_, _) => Task.FromResult(Ok("{}")), _ => Task.FromResult(true));
        var move = new LegacyMigrationMove(
            (_, _) => { asked++; return gate.Task; }, step, label => label);

        Task first = move.MoveAsync(null);
        await move.MoveAsync(null);
        Assert.Equal(1, asked);
        Assert.True(move.Busy);
        gate.SetResult(Ok("{}"));
        await first;
    }
}
