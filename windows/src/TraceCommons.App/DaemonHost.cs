using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// Owns the daemon's lifetime for the app and is the single seam between the
/// C ABI and anything with a UI thread.
///
/// The division of labour, mirroring <c>DaemonHost.swift</c>: everything below
/// this class is thread-agnostic and knows nothing about WinUI; everything
/// above it only ever runs on the UI thread. This class is where the hop
/// happens, and it is the only place a <see cref="DispatcherQueue"/> and a
/// <see cref="TcDaemon"/> are both in scope.
/// </summary>
public sealed class DaemonHost : IAsyncDisposable
{
    private readonly DispatcherQueue _dispatcher;
    private readonly string _configDir;
    private TcDaemon? _daemon;
    private TcSubscription? _subscription;

    /// <summary>
    /// Raised on the UI thread when the queue may have changed. Carries no
    /// payload deliberately: the event frame's own data is a delta, and the
    /// app refetches rather than patching, so there is nothing worth passing.
    /// </summary>
    public event Action? QueueChanged;

    /// <summary>Raised on the UI thread when daemon status may have changed.</summary>
    public event Action? StatusChanged;

    /// <summary>
    /// Raised on the UI thread when the daemon says a digest is due, carrying
    /// the pending count it decided on.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The daemon owns the decision, because the batching policy is shared by
    /// every application that attaches to it: <c>daemon/notify.rs</c> refuses
    /// on an empty queue and otherwise fires once per
    /// <c>digest_interval_secs</c>, persisting the stamp so the spacing
    /// survives a restart. This event is delivery, not policy, and the shell
    /// must not invent a timer of its own to supplement it.
    /// </para>
    /// <para>
    /// The daemon's own <c>text</c> field is deliberately ignored by the
    /// subscriber. It phrases the digest for a shell-less daemon; the shared
    /// spec's wording for an application is what
    /// <c>TraceCommons.Interop.DigestText</c> produces, and all three shells
    /// use that.
    /// </para>
    /// </remarks>
    public event Action<int>? DigestDue;

    /// <summary>
    /// Raised on the UI thread when the ABI reported dropped events. The view
    /// model treats this as "your local picture is stale, refetch everything",
    /// which is the only correct response to an unknown number of missed
    /// deltas.
    /// </summary>
    public event Action<int>? Lagged;

    /// <summary>
    /// Raised on the UI thread when the daemon's preview scheduler finishes
    /// and delivers one card's result -- the only way a card queued or
    /// running when it was requested ever gets filled in, since a build that
    /// was not answered from cache publishes no other signal.
    /// </summary>
    public event Action<PreviewCardOutcome>? PreviewReady;

    public DaemonHost(DispatcherQueue dispatcher, string? configDir = null)
    {
        _dispatcher = dispatcher ?? throw new ArgumentNullException(nameof(dispatcher));
        _configDir = configDir ?? DefaultConfigDir();
    }

    /// <summary>
    /// The per-user state directory, matching where the contributor CLI keeps
    /// its own. Under %LOCALAPPDATA% on Windows; the fallback keeps this class
    /// constructible in a non-Windows test host.
    /// <para>
    /// The CLI's half of this agreement is <c>platform_default_dir</c> in
    /// <c>crates/trace-commons-contributor/src/config.rs</c>, which special-cases
    /// Windows for the same reason: <c>dirs::config_dir()</c> is roaming AppData,
    /// which roaming profiles copy between machines, and this directory holds a
    /// machine-bound device key. The two disagreed until then -- enrolling in one
    /// left the other reporting no enrollment on the same machine -- so changing
    /// this path means changing that one.
    /// </para>
    /// </summary>
    public static string DefaultConfigDir()
    {
        string root = Environment.GetFolderPath(
            Environment.SpecialFolder.LocalApplicationData,
            Environment.SpecialFolderOption.DoNotVerify);

        if (string.IsNullOrEmpty(root))
        {
            root = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".config");
        }

        return Path.Combine(root, "trace-commons");
    }

    /// <summary>Whether a daemon is currently running in this process.</summary>
    public bool IsRunning => _daemon is not null;

    /// <summary>
    /// The UI thread's queue, for the one caller that needs a timer on it.
    ///
    /// Exposed rather than duplicated: the undo bar counts down against the
    /// daemon's own hold deadline, and a second dispatcher obtained elsewhere
    /// would be the same queue reached by a route that no longer says so.
    /// </summary>
    public DispatcherQueue Dispatcher => _dispatcher;

    /// <summary>
    /// Starts the daemon and subscribes to its events.
    ///
    /// Runs the start on a background thread: tc_daemon_start builds a runtime
    /// and performs the first filesystem scan, and blocking the UI thread on
    /// that is a visible hang on a large session history.
    /// </summary>
    /// <param name="settingsJson">
    /// Settings applied and persisted BEFORE the watcher's first tick, or null
    /// to use whatever is already persisted.
    ///
    /// This is how the roots screen's answer reaches the daemon. The refusal
    /// is evaluated after these settings are applied, so a declaration passed
    /// here clears the refusal that the persisted file alone would have
    /// earned -- which is the difference between a contributor who just
    /// answered and one who has to restart the app to be believed.
    /// </param>
    /// <exception cref="TcException">
    /// The daemon could not start -- because the session sources are
    /// undeclared (<see cref="TcException.IsRootsNotDeclared"/>, which the
    /// caller should route to the roots screen rather than report as a
    /// fault), or most commonly because another instance of this app, or the
    /// contributor CLI's daemon, already holds the lock for this state
    /// directory.
    /// </exception>
    public async Task StartAsync(
        string? settingsJson = null,
        CancellationToken cancellationToken = default)
    {
        if (_daemon is not null)
        {
            return;
        }

        Directory.CreateDirectory(_configDir);

        // Nothing is watched that the contributor did not name. With no
        // settings passed, the daemon reads what is persisted and refuses to
        // start when that does not declare both sources -- it does NOT fall
        // back to the real ~/.claude and ~/.codex. That fallback is what this
        // client used to get, and it meant a contributor's whole session
        // history was scanned on the strength of never having been asked.
        TcDaemon daemon = await Task
            .Run(() => new TcDaemon(_configDir, settingsJson), cancellationToken)
            .ConfigureAwait(true);

        _daemon = daemon;
        _subscription = daemon.Subscribe(OnDaemonEvent);
    }

    /// <summary>
    /// Calls a daemon method off the UI thread and returns the parsed
    /// response.
    ///
    /// Every call is dispatched to the thread pool. tc_call is synchronous and
    /// its duration is the daemon's, not ours; the counted gate inside
    /// <see cref="TcDaemon"/> is what makes running several concurrently safe.
    /// </summary>
    public async Task<DaemonResponse> CallAsync(
        string method,
        string paramsJson = "{}",
        CancellationToken cancellationToken = default)
    {
        TcDaemon? daemon = _daemon;
        if (daemon is null)
        {
            return DaemonResponse.Parse(
                "{\"error\":{\"code\":\"unavailable\",\"message\":\"daemon-not-started\"}}");
        }

        string raw = await Task
            .Run(() => daemon.Call(method, paramsJson), cancellationToken)
            .ConfigureAwait(true);

        return DaemonResponse.Parse(raw);
    }

    /// <summary>Fetches the pending queue, or an empty list on error.</summary>
    public async Task<IReadOnlyList<QueueEntry>> ListPendingAsync(
        CancellationToken cancellationToken = default)
    {
        DaemonResponse response = await CallAsync(
                DaemonProtocol.Methods.ListPending,
                cancellationToken: cancellationToken)
            .ConfigureAwait(true);

        return response.ResultAs<PendingList>()?.Pending ?? (IReadOnlyList<QueueEntry>)Array.Empty<QueueEntry>();
    }

    /// <summary>
    /// Opens the in-process preview for an entry, off the UI thread.
    ///
    /// Off-thread because <c>tc_preview_open</c> reads the session file and
    /// runs the whole redaction pass synchronously: on a 169 KB trace that is
    /// a visible freeze if it happens on the UI thread, and this is the one
    /// screen where a freeze reads as the app struggling with the very bytes
    /// it is about to send.
    ///
    /// The returned preview is the caller's to dispose. It holds native memory
    /// and every borrowed pointer it handed out dies with it.
    /// </summary>
    /// <exception cref="TcException">
    /// The entry is unknown, or the daemon handle is already gone.
    /// </exception>
    public async Task<TcPreview> OpenPreviewAsync(
        string entryId,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(entryId);

        TcDaemon daemon = _daemon
            ?? throw new TcException("daemon-not-started");

        return await Task.Run(() => daemon.OpenPreview(entryId), cancellationToken)
            .ConfigureAwait(true);
    }

    /// <summary>
    /// The subscription callback. Runs on a RUST BACKGROUND THREAD, so it does
    /// the minimum possible work and hops to the UI thread for everything
    /// else. Nothing here may touch observable state directly.
    /// </summary>
    private void OnDaemonEvent(string json)
    {
        DaemonEvent? evt = DaemonEvent.Parse(json);
        if (evt is null)
        {
            return;
        }

        // TryEnqueue rather than an assert: during shutdown the dispatcher can
        // already be gone, and an event arriving then is expected -- the ABI
        // explicitly permits a callback after tc_daemon_stop returns.
        _dispatcher.TryEnqueue(() =>
        {
            switch (evt.Event)
            {
                case DaemonProtocol.Events.QueueChanged:
                case DaemonProtocol.Events.Snapshot:
                    QueueChanged?.Invoke();
                    break;

                case DaemonProtocol.Events.StatusChanged:
                    StatusChanged?.Invoke();
                    break;

                case DaemonProtocol.Events.DigestDue:
                    DigestDue?.Invoke(evt.PendingCount);
                    break;

                case DaemonProtocol.Events.Lagged:
                    // An unknown number of deltas was missed, so the local
                    // picture is unreliable. Both handlers fire: the correct
                    // response is a full refetch of everything, not a targeted
                    // patch.
                    Lagged?.Invoke(evt.SkippedCount);
                    QueueChanged?.Invoke();
                    StatusChanged?.Invoke();
                    break;

                case DaemonProtocol.Events.ResyncRequired:
                    QueueChanged?.Invoke();
                    StatusChanged?.Invoke();
                    break;

                case DaemonProtocol.Events.PreviewReady:
                    if (evt.PreviewOutcome is { } outcome)
                    {
                        PreviewReady?.Invoke(outcome);
                    }

                    break;
            }
        });
    }

    /// <summary>
    /// Ordered teardown, off the UI thread.
    ///
    /// Off-thread for two reasons the ABI states directly: teardown blocks
    /// until in-flight calls drain and until the unsubscribe barrier is
    /// confirmed, and tc_unsubscribe must run on a plain thread. A UI thread
    /// pumping a message loop is a poor place for either.
    /// </summary>
    public async ValueTask DisposeAsync()
    {
        TcDaemon? daemon = _daemon;
        TcSubscription? subscription = _subscription;
        _daemon = null;
        _subscription = null;

        if (daemon is null)
        {
            return;
        }

        await Task.Run(() =>
        {
            // A Leaked outcome is a deliberate, safe result, not a failure to
            // handle: the handle stays alive precisely because something might
            // still touch it. The process is exiting either way.
            daemon.Shutdown(subscription);
        }).ConfigureAwait(false);
    }
}
