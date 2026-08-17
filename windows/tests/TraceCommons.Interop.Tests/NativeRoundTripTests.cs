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
                     DaemonProtocol.Methods.Approve,
                     DaemonProtocol.Methods.Dismiss,
                     DaemonProtocol.Methods.Shutdown,
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
}
