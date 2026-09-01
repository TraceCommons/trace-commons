using System;
using System.Collections.Concurrent;
using System.IO;
using System.Text.Json;
using System.Threading;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// End-to-end tests against the real cdylib: a genuine daemon, started in a
/// temp directory, driven through the same binding the WinUI app uses.
///
/// These are the tests that matter. The offset conversion is logic and can be
/// reasoned about; pointer ownership, UTF-8 marshalling, delegate rooting and
/// the unsubscribe barrier can only be confirmed by running them. They pass on
/// macOS and Linux as readily as on Windows because the interop assembly is
/// platform-neutral -- the whole reason it targets plain net8.0.
///
/// EVERY test here starts the daemon with explicit settings pointing
/// claude_root and codex_root inside the temp directory. That is not tidiness:
/// tc_daemon_start alone would scan the developer's REAL ~/.claude and
/// ~/.codex, and a test suite that reads a person's actual transcripts is not
/// a test suite anyone should run.
/// </summary>
public sealed class NativeRoundTripTests : IDisposable
{
    private const string PlantedSecret = "sk-fake-windows-preview-secret-1234";

    private readonly string _configDir;
    private readonly string _settingsJson;

    /// <summary>
    /// Builds a DELIBERATELY SHORT config directory path.
    ///
    /// On Unix the daemon serves its IPC over a unix domain socket inside this
    /// directory, and <c>sun_path</c> is capped at 104 bytes on macOS. The
    /// obvious fixture -- a nested folder plus a 32-character GUID under
    /// <c>$TMPDIR</c>, which is itself 48 characters on macOS -- overruns that
    /// cap once the socket filename is appended, and every daemon start fails.
    ///
    /// It fails as the opaque label <c>daemon-start-failed</c>, which the ABI
    /// uses for every start failure that is not a settings problem, so nothing
    /// in the error points at path length. That cost real debugging time; the
    /// short name and this comment are here so it costs none the next time.
    ///
    /// Windows is unaffected -- the transport there is a named pipe with no
    /// comparable limit -- but the path stays short on every platform rather
    /// than branching, since there is no benefit to a long one.
    /// </summary>
    private static string ShortTempDir()
    {
        // 8 hex characters is collision-safe enough for a test fixture and
        // keeps the total well under the cap even on macOS.
        string name = "tc-" + Guid.NewGuid().ToString("n").Substring(0, 8);
        return Path.Combine(Path.GetTempPath(), name);
    }

    public NativeRoundTripTests()
    {
        _configDir = ShortTempDir();
        Directory.CreateDirectory(_configDir);

        string claudeRoot = Path.Combine(_configDir, "claude");
        string codexRoot = Path.Combine(_configDir, "codex");
        Directory.CreateDirectory(claudeRoot);
        Directory.CreateDirectory(codexRoot);

        _settingsJson = JsonSerializer.Serialize(new
        {
            claude_root = claudeRoot,
            codex_root = codexRoot,
        });
    }

    private TcDaemon StartDaemon() => new(_configDir, _settingsJson);

    /// <summary>
    /// Seeds the exact local state the Windows app sees after enrollment and
    /// discovery: one enrolled contributor, one real Claude session, and one
    /// pending queue entry pointing at it.
    /// </summary>
    /// <remarks>
    /// The queue is written before the daemon starts instead of waiting for
    /// two watcher polls. A first sighting is deliberately unstable and the
    /// production poll interval is a minute, neither of which belongs in a
    /// test whose subject is preview-then-approve rather than discovery.
    /// Every path remains inside this test's private temp directory.
    /// </remarks>
    private string SeedEnrolledQueuedSession()
    {
        string claudeRoot = Path.Combine(_configDir, "claude");
        string projectDir = Path.Combine(claudeRoot, "-Users-testuser-code-windows-preview");
        Directory.CreateDirectory(projectDir);

        string sessionPath = Path.Combine(projectDir, "preview-session.jsonl");
        string session = JsonSerializer.Serialize(new
        {
            type = "user",
            message = new
            {
                role = "user",
                content = $"deploy with key {PlantedSecret}",
            },
            cwd = "/Users/testuser/code/windows-preview",
            timestamp = "2026-08-18T10:00:00Z",
            version = "2.0.1",
            sessionId = "preview-session",
            uuid = "a1",
        });
        File.WriteAllText(sessionPath, session + Environment.NewLine);

        string contributor = JsonSerializer.Serialize(new
        {
            schema_version = "trace_commons.contributor_config.v1",
            issuer_url = "http://issuer.invalid",
            ingest_url = "http://ingest.invalid",
            audience = "trace-commons-upload",
            tenant_id = "tenant-windows-test",
            instance_id = "instance-windows-test",
            user_subject = "windows-test-user",
            device_key_id = "sha256:windows-test",
            consent_scopes = new[] { "debugging_evaluation" },
            pii_filter = (string?)null,
            allowed_hosts = (string?)null,
        });
        File.WriteAllText(Path.Combine(_configDir, "contributor.json"), contributor);

        string entryId = Guid.NewGuid().ToString();
        string queueEntry = JsonSerializer.Serialize(new
        {
            entry_id = entryId,
            session_hash = "sha256:windows-preview-round-trip",
            source = "claude-code",
            project_key = "/Users/testuser/code/windows-preview",
            project_label = "windows-preview",
            path = sessionPath,
            size_bytes = new FileInfo(sessionPath).Length,
            discovered_at = "2026-08-18T10:01:00Z",
            state = "pending",
            reason_label = (string?)null,
            attempts = 0,
            retry_after = (string?)null,
            submission_id = (string?)null,
            approved_scopes = (string[]?)null,
            approved_inputs = (string?)null,
            previewed_envelope_digest = (string?)null,
            approved_at = (string?)null,
            subagent_count = 0,
            subagents_dropped = 0,
        });
        File.WriteAllText(
            Path.Combine(_configDir, "daemon-queue.jsonl"),
            queueEntry + Environment.NewLine);

        return entryId;
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_configDir, recursive: true);
        }
        catch (IOException)
        {
            // A daemon that leaked its lock file on a failing test must not
            // turn into a second, confusing failure in teardown.
        }
    }

    [Fact]
    public void DaemonStartsAndHandshakes()
    {
        using TcDaemon daemon = StartDaemon();

        DaemonResponse response = DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.Hello));

        Assert.False(response.IsError);
        DaemonHello? hello = response.ResultAs<DaemonHello>();
        Assert.NotNull(hello);
        Assert.Equal(DaemonProtocol.SchemaVersion, hello!.SchemaVersion);
        Assert.True(
            hello.AcceptsClientSchema,
            "the daemon must accept the schema this binding speaks");
    }

    [Fact]
    public void HelloAdvertisesTheMethodsAndEventsThisBindingNames()
    {
        // Guards the constants in DaemonProtocol against silent drift. A
        // renamed method on the Rust side would otherwise surface as an
        // unknown_method error at runtime, in the UI, in front of a
        // contributor.
        using TcDaemon daemon = StartDaemon();

        DaemonHello? hello = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.Hello))
            .ResultAs<DaemonHello>();

        Assert.NotNull(hello);

        foreach (string method in new[]
                 {
                     DaemonProtocol.Methods.Status,
                     DaemonProtocol.Methods.ListPending,
                     DaemonProtocol.Methods.Pause,
                     DaemonProtocol.Methods.Resume,
                     DaemonProtocol.Methods.GetSettings,
                     DaemonProtocol.Methods.SetSettings,
                     DaemonProtocol.Methods.ListProjects,
                     DaemonProtocol.Methods.SetProjectMode,
                     DaemonProtocol.Methods.ListAudit,
                     DaemonProtocol.Methods.Approve,
                     DaemonProtocol.Methods.Dismiss,
                     DaemonProtocol.Methods.Cancel,
                     DaemonProtocol.Methods.Shutdown,

                     // The roster profile. Named here rather than trusted,
                     // because this is the only check that these three
                     // constants are the daemon's own strings: everything
                     // else about the Settings panel is exercised against
                     // fixtures, and a typo would surface as an
                     // unknown_method in front of a contributor claiming a
                     // handle.
                     DaemonProtocol.Methods.GetPublicProfile,
                     DaemonProtocol.Methods.SetPublicProfile,
                     DaemonProtocol.Methods.ClearPublicProfile,
                 })
        {
            Assert.Contains(method, hello!.Methods);
        }

        foreach (string evt in new[]
                 {
                     DaemonProtocol.Events.Snapshot,
                     DaemonProtocol.Events.QueueChanged,
                     DaemonProtocol.Events.StatusChanged,
                 })
        {
            Assert.Contains(evt, hello!.Events);
        }
    }

    [Fact]
    public void ListPendingReturnsAParsableQueue()
    {
        using TcDaemon daemon = StartDaemon();

        DaemonResponse response =
            DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.ListPending));

        Assert.False(response.IsError);
        PendingList? pending = response.ResultAs<PendingList>();
        Assert.NotNull(pending);

        // Empty is the expected result against an empty session root. The
        // assertion that matters is that the envelope and payload deserialize
        // into the shapes the UI binds to.
        Assert.NotNull(pending!.Pending);
    }

    [Fact]
    public void PauseDeadlineAndResumeRoundTripThroughTheNativeBinding()
    {
        using TcDaemon daemon = StartDaemon();
        string pause = PauseRequest.Serialize(
            PauseDuration.OneHour,
            DateTimeOffset.UtcNow);

        DaemonResponse paused = DaemonResponse.Parse(
            daemon.Call(DaemonProtocol.Methods.Pause, pause));
        Assert.False(paused.IsError);
        Assert.True(Status(daemon).Paused);

        DaemonResponse resumed = DaemonResponse.Parse(
            daemon.Call(DaemonProtocol.Methods.Resume));
        Assert.False(resumed.IsError);
        Assert.False(Status(daemon).Paused);
    }

    [Fact]
    public void ProjectSettingsUseTheDaemonsOpaqueIdAndLabel()
    {
        SeedEnrolledQueuedSession();
        using TcDaemon daemon = StartDaemon();

        ProjectSettingsPayload? payload = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.ListProjects))
            .ResultAs<ProjectSettingsPayload>();

        ProjectSetting project = Assert.Single(payload!.Projects);
        Assert.False(string.IsNullOrWhiteSpace(project.ProjectId));
        Assert.Equal("windows-preview", project.ProjectLabel);
        Assert.DoesNotContain('/', project.ProjectId);
        Assert.DoesNotContain('\\', project.ProjectId);
    }

    [Fact]
    public void BehaviorSettingAndAuditRoundTripThroughTheNativeBinding()
    {
        SeedEnrolledQueuedSession();
        using TcDaemon daemon = StartDaemon();

        DaemonResponse changed = DaemonResponse.Parse(
            daemon.Call(
                DaemonProtocol.Methods.SetSettings,
                BehaviorSettingsRequest.Serialize(BehaviorSetting.QuiescenceMinutes, 12)));
        Assert.False(changed.IsError);
        Assert.Equal(
            720UL,
            changed.ResultAs<DaemonSettingsSnapshot>()!.QuiescenceSeconds);

        DaemonResponse consent = DaemonResponse.Parse(
            daemon.Call(
                DaemonProtocol.Methods.SetConsentScopes,
                "{\"scopes\":[\"debugging_evaluation\"]}"));
        Assert.False(consent.IsError);

        AuditSettingsPayload? audit = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.ListAudit, "{\"limit\":20}"))
            .ResultAs<AuditSettingsPayload>();
        Assert.Contains(audit!.Entries, entry => entry.Action == "consent-scopes-changed");
    }

    [Fact]
    public void UnknownMethodReturnsAnErrorFrameNotACrash()
    {
        using TcDaemon daemon = StartDaemon();

        DaemonResponse response = DaemonResponse.Parse(daemon.Call("no_such_method"));

        Assert.True(response.IsError);
        Assert.Equal("unknown_method", response.Error!.Code);
    }

    [Fact]
    public void MalformedParamsReturnAnErrorFrameNotACrash()
    {
        // The header promises tc_call never returns NULL even for malformed
        // params. This is that promise, exercised.
        using TcDaemon daemon = StartDaemon();

        DaemonResponse response =
            DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.Status, "{not json"));

        Assert.True(response.IsError);
    }

    [Fact]
    public void NonAsciiParamsSurviveTheUtf8Boundary()
    {
        // The marshalling rule that would fail silently under the .NET default
        // of UTF-16/ANSI. Approving a non-existent entry is expected to fail;
        // what is asserted is that it fails as a clean daemon error rather
        // than a marshalling crash or a mangled round trip.
        using TcDaemon daemon = StartDaemon();

        string paramsJson = JsonSerializer.Serialize(new { entry_id = "日本語-café-🎯" });
        DaemonResponse response =
            DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.Approve, paramsJson));

        Assert.True(response.IsError);
        Assert.False(string.IsNullOrEmpty(response.Error!.Code));
    }

    [Fact]
    public void SecondDaemonAgainstTheSameDirectoryIsRefused()
    {
        // Two loops racing one on-disk queue is the failure the daemon lock
        // exists to prevent, and the binding must surface it as an exception
        // rather than a null handle.
        using TcDaemon first = StartDaemon();

        TcException error = Assert.Throws<TcException>(() => StartDaemon());
        Assert.Contains("tc_daemon_start", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void SubscribeDeliversEventsAndUnsubscribeIsConfirmed()
    {
        // The delegate-rooting and ctx-lifetime path. If the delegate were not
        // rooted, this is where a GC would take the process down; if the
        // unsubscribe barrier were assumed rather than checked, this is where
        // the roots would be freed while a callback could still fire.
        using TcDaemon daemon = StartDaemon();

        var received = new ConcurrentQueue<string>();
        using var gotOne = new ManualResetEventSlim(false);

        TcSubscription? subscription = daemon.Subscribe(json =>
        {
            received.Enqueue(json);
            gotOne.Set();
        });

        Assert.NotNull(subscription);
        Assert.True(subscription!.RootsHeld);

        // Subscribing does not itself publish anything -- the daemon emits on
        // state change -- so an event has to be provoked. `resume` is the
        // cheapest one that always produces a status change against an idle
        // daemon.
        daemon.Call(DaemonProtocol.Methods.Resume);

        Assert.True(
            gotOne.Wait(TimeSpan.FromSeconds(10)),
            "expected an event frame within 10s of provoking one");

        Assert.True(received.TryDequeue(out string? first));
        DaemonEvent? evt = DaemonEvent.Parse(first!);
        Assert.NotNull(evt);
        Assert.False(string.IsNullOrEmpty(evt!.Event));

        // xunit runs the test body on a pool thread, which is not a tokio
        // runtime context, so the barrier is expected to hold on the first
        // attempt.
        Assert.True(
            daemon.Unsubscribe(subscription),
            "tc_unsubscribe was refused; the barrier did not hold");
        Assert.False(subscription.RootsHeld);

        // The barrier's actual contract: no callback fires AFTER
        // tc_unsubscribe returns. So the count must already be final the
        // instant it returned -- a burst of further events must not move it,
        // and deliberately without any wait, since waiting would test a
        // weaker claim than the one being made.
        int countAtUnsubscribe = received.Count;
        for (int i = 0; i < 10; i++)
        {
            daemon.Call(DaemonProtocol.Methods.Pause);
            daemon.Call(DaemonProtocol.Methods.Resume);
        }

        Assert.Equal(countAtUnsubscribe, received.Count);
    }

    [Fact]
    public void CallsAfterShutdownReturnUnavailableRatherThanCrashing()
    {
        TcDaemon daemon = StartDaemon();
        Assert.Equal(TcDaemon.ShutdownOutcome.Freed, daemon.Shutdown());

        DaemonResponse response = DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.Status));

        Assert.True(response.IsError);
        Assert.Equal("unavailable", response.Error!.Code);
    }

    [Fact]
    public void ShutdownIsIdempotent()
    {
        // The second call must not re-enter teardown -- that is exactly the
        // concurrent free the header forbids.
        TcDaemon daemon = StartDaemon();

        Assert.Equal(TcDaemon.ShutdownOutcome.Freed, daemon.Shutdown());
        Assert.Equal(TcDaemon.ShutdownOutcome.Freed, daemon.Shutdown());
        daemon.Dispose();
    }

    [Fact]
    public void ShutdownWithASubscriptionReleasesItsRoots()
    {
        TcDaemon daemon = StartDaemon();
        TcSubscription? subscription = daemon.Subscribe(_ => { });
        Assert.NotNull(subscription);

        Assert.Equal(TcDaemon.ShutdownOutcome.Freed, daemon.Shutdown(subscription));
        Assert.False(subscription!.RootsHeld);
    }

    [Fact]
    public void ConcurrentCallsAreAdmittedSimultaneously()
    {
        // The counted gate must admit concurrent callers rather than
        // serializing them; the header explicitly permits the concurrency and
        // a lock-based binding would quietly give it up.
        using TcDaemon daemon = StartDaemon();

        const int threads = 8;
        using var start = new ManualResetEventSlim(false);
        var errors = new ConcurrentQueue<string>();
        var workers = new Thread[threads];

        for (int i = 0; i < threads; i++)
        {
            workers[i] = new Thread(() =>
            {
                start.Wait();
                for (int call = 0; call < 20; call++)
                {
                    DaemonResponse response =
                        DaemonResponse.Parse(daemon.Call(DaemonProtocol.Methods.Status));
                    if (response.IsError)
                    {
                        errors.Enqueue(response.Error!.Code);
                    }
                }
            });
            workers[i].Start();
        }

        start.Set();
        foreach (Thread worker in workers)
        {
            Assert.True(worker.Join(TimeSpan.FromSeconds(30)), "worker did not finish");
        }

        Assert.Empty(errors);
        Assert.Equal(0, daemon.InFlightCalls);
    }

    [Fact]
    public void PreviewOpenOnAnUnknownEntryFailsCleanly()
    {
        // The preview path's error branch, which is the one a UI hits when an
        // entry is dismissed between listing and opening. The success branch
        // needs a real session file and belongs with the queue-fixture work,
        // not here.
        using TcDaemon daemon = StartDaemon();

        TcException error =
            Assert.Throws<TcException>(() => daemon.OpenPreview("no-such-entry"));
        Assert.Contains("tc_preview_open", error.Message, StringComparison.Ordinal);
    }

    private static DaemonStatus Status(TcDaemon daemon)
    {
        DaemonStatus? status = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.Status))
            .ResultAs<DaemonStatus>();
        return Assert.IsType<DaemonStatus>(status);
    }

    [Fact]
    public void EnrolledPreviewCanBeApprovedAndUndoneThroughTheNativeBinding()
    {
        // This is the Windows app's complete consequential path below WinUI:
        // the same queue shape it binds, the same in-process preview it shows,
        // and the same approve/cancel calls its buttons issue. It does not
        // contact an ingest server; the daemon's real approval hold keeps the
        // fixture local until it is recalled.
        string entryId = SeedEnrolledQueuedSession();
        using TcDaemon daemon = StartDaemon();

        PendingList? before = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.ListPending))
            .ResultAs<PendingList>();
        Assert.NotNull(before);
        Assert.Contains(before!.Pending, entry => entry.EntryId == entryId);

        using (TcPreview preview = daemon.OpenPreview(entryId))
        {
            PreviewSummary? summary = PreviewSummary.Parse(preview.SummaryJson);
            Assert.NotNull(summary);
            Assert.True(summary!.Enrolled);
            Assert.True(summary.WouldSendBytes > 0);
            Assert.DoesNotContain(PlantedSecret, preview.Body, StringComparison.Ordinal);
        }

        string entryParams = JsonSerializer.Serialize(new { entry_id = entryId });
        DateTimeOffset beforeApprove = DateTimeOffset.UtcNow;
        DaemonResponse approved = DaemonResponse.Parse(
            daemon.Call(DaemonProtocol.Methods.Approve, entryParams));
        DateTimeOffset afterApprove = DateTimeOffset.UtcNow;
        Assert.False(approved.IsError);

        ApprovalHold? hold = ApprovalHold.Parse(approved);
        Assert.NotNull(hold);
        Assert.True(hold!.HoldSecs >= 5);

        // Check that something was actually approved BEFORE reading the
        // deadline, because `hold_until` is legitimately null when nothing
        // was -- "no undo may be offered" is a real answer, not a fault.
        //
        // This ordering is the point of the rework. The single assertion
        // that used to stand here, `hold.Deadline > DateTimeOffset.UtcNow`,
        // was false in two completely different worlds: a deadline that had
        // already passed, and no deadline at all because the entry was
        // skipped. It carried no message, so Windows CI reported only
        // "Expected: True / Actual: False" and could not distinguish a
        // stalled runner from an entry the daemon declined to approve.
        // Running it here shows the second world is reachable: on this macOS
        // checkout the entry comes back skipped, approved=0, hold_until
        // null, and the old assertion fails exactly as it did on CI for what
        // may be an entirely different reason.
        //
        // So name the precondition and report the daemon's own reason when
        // it does not hold.
        Assert.True(
            hold.Skipped.Count == 0,
            "the daemon skipped the entry instead of approving it: "
                + string.Join(
                    ", ",
                    hold.Skipped.ConvertAll(skip => skip.ReasonLabel)));
        Assert.Equal(1UL, hold.Approved);

        // Now the deadline, bracketed rather than raced. The daemon stamps
        // hold_until as its own clock plus hold_secs while handling the call
        // above, so it must land between "the call had started" and "the
        // call had returned", each plus hold_secs. That holds no matter how
        // long this test then takes to reach this line, which the old form
        // did not: it required the test to get here within hold_secs of the
        // stamp, so a runner that stalled failed a daemon that had behaved.
        //
        // The one-second slack absorbs RFC 3339 formatting, which does not
        // necessarily preserve sub-second precision.
        Assert.NotNull(hold.Deadline);
        Assert.InRange(
            hold.Deadline!.Value,
            beforeApprove.AddSeconds(hold.HoldSecs - 1),
            afterApprove.AddSeconds(hold.HoldSecs + 1));

        PendingList? whileApproved = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.ListPending))
            .ResultAs<PendingList>();
        Assert.NotNull(whileApproved);
        Assert.DoesNotContain(whileApproved!.Pending, entry => entry.EntryId == entryId);

        DaemonResponse cancelled = DaemonResponse.Parse(
            daemon.Call(DaemonProtocol.Methods.Cancel, entryParams));
        Assert.False(cancelled.IsError);

        PendingList? afterUndo = DaemonResponse
            .Parse(daemon.Call(DaemonProtocol.Methods.ListPending))
            .ResultAs<PendingList>();
        Assert.NotNull(afterUndo);
        Assert.Contains(afterUndo!.Pending, entry => entry.EntryId == entryId);
    }
}
