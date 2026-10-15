using System;
using System.ComponentModel;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The passkey step of the legacy invite move: the NEAR AI browser sign-in
/// the move needs a session from. It starts the same ceremony Private AI
/// does, opens the browser on it and polls the daemon until the session is
/// kept or the attempt is over.
/// </summary>
/// <remarks>
/// No words and no policy of its own: the sign-in states are the daemon's
/// (<c>near_ai_credential_status</c>), and what a finished or failed step
/// says is the Rust's (<see cref="LegacyMigrationOfferCopy"/>). Transport and
/// browser are injected so the loop runs off Windows.
/// </remarks>
public sealed class NearAiPasskeyStep
{
    /// <summary>The ceremony's waiting word (<c>nearai_credential::ceremony</c>).</summary>
    public const string WaitingForBrowser = "waiting_for_browser";

    /// <summary>How long the step waits on the browser before it gives up.</summary>
    public static readonly TimeSpan DefaultLimit = TimeSpan.FromMinutes(10);

    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<Uri, Task<bool>> _open;
    private readonly Func<TimeSpan, CancellationToken, Task> _delay;
    private readonly TimeSpan _interval;
    private readonly TimeSpan _limit;
    private string? _attemptId;

    public NearAiPasskeyStep(
        Func<string, string, Task<DaemonResponse>> call,
        Func<Uri, Task<bool>> open,
        Func<TimeSpan, CancellationToken, Task>? delay = null,
        TimeSpan? interval = null,
        TimeSpan? limit = null)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
        _open = open ?? throw new ArgumentNullException(nameof(open));
        _delay = delay ?? ((span, token) => Task.Delay(span, token));
        _interval = interval ?? TimeSpan.FromSeconds(1);
        _limit = limit ?? DefaultLimit;
    }

    /// <summary>
    /// True when the daemon holds a session when the step ends. False when it
    /// failed, ran out of time or was cancelled.
    /// </summary>
    public async Task<bool> RunAsync(CancellationToken cancellation)
    {
        try
        {
            if (await ReadAsync(null).ConfigureAwait(true) is { } first && IsSignedIn(first))
            {
                return true;
            }

            if (cancellation.IsCancellationRequested)
            {
                return false;
            }

            DaemonResponse started = await _call(DaemonProtocol.Methods.NearAiCredentialStart, "{}")
                .ConfigureAwait(true);
            NearAiCredentialAttempt? attempt = started.IsError || started.Result is null
                ? null
                : NearAiCredentialSurface.ParseStart(started.Result.Value.GetRawText());
            if (attempt is not { } begun || !Uri.TryCreate(begun.BrowserUrl, UriKind.Absolute, out Uri? url))
            {
                return false;
            }

            _attemptId = begun.AttemptId;
            if (cancellation.IsCancellationRequested)
            {
                await CancelAsync().ConfigureAwait(true);
                return false;
            }

            bool opened;
            try
            {
                opened = await _open(url).ConfigureAwait(true);
            }
            catch
            {
                // Browser-launch errors can include the credential-bearing URL.
                opened = false;
            }

            if (!opened)
            {
                await CancelAsync().ConfigureAwait(true);
                return false;
            }

            DateTime deadline = DateTime.UtcNow + _limit;
            while (DateTime.UtcNow < deadline && !cancellation.IsCancellationRequested)
            {
                try
                {
                    await _delay(_interval, cancellation).ConfigureAwait(true);
                }
                catch (OperationCanceledException)
                {
                    return false;
                }

                if (cancellation.IsCancellationRequested)
                {
                    return false;
                }

                NearAiCredentialStatus? status = await ReadAsync(_attemptId).ConfigureAwait(true);
                if (status is { } read)
                {
                    if (IsSignedIn(read))
                    {
                        return true;
                    }

                    if (read.AttemptStatus is { } word && word != WaitingForBrowser)
                    {
                        return false;
                    }
                }
            }

            // Out of time, or cancelled: end the browser step in the daemon.
            await CancelAsync().ConfigureAwait(true);
            return false;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            System.Diagnostics.Trace.TraceWarning(nameof(NearAiPasskeyStep));
            return false;
        }
    }

    /// <summary>Ends the browser step: the attempt this step began, else whatever sign-in runs.</summary>
    public async Task CancelAsync()
    {
        try
        {
            await _call(
                    DaemonProtocol.Methods.NearAiCredentialCancel,
                    NearAiCredentialSurface.SerializeCancel(_attemptId))
                .ConfigureAwait(true);
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(NearAiPasskeyStep));
        }
    }

    private async Task<NearAiCredentialStatus?> ReadAsync(string? attemptId)
    {
        DaemonResponse response = await _call(
                DaemonProtocol.Methods.NearAiCredentialStatus,
                NearAiCredentialSurface.SerializeStatus(attemptId))
            .ConfigureAwait(true);
        return response.IsError || response.Result is null
            ? null
            : NearAiCredentialSurface.ParseStatus(response.Result.Value.GetRawText());
    }

    private static bool IsSignedIn(NearAiCredentialStatus status) =>
        status.SessionState == NearAiEnrollSurface.CredentialStatePresent;
}

/// <summary>
/// The Move on the legacy invite offer, passkey first: the daemon is asked to
/// move the identity, and when it has no NEAR AI session to provision the
/// account with the passkey step runs and the move is asked once more,
/// without a second press. A refusal changes nothing and is said in the
/// core's words; a passkey step that does not finish moves nothing.
/// </summary>
/// <remarks>
/// One instance serves the Settings section and the notices card, so the two
/// can never disagree about whether a move is running.
/// </remarks>
public sealed class LegacyMigrationMove : INotifyPropertyChanged
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly NearAiPasskeyStep _passkey;
    private readonly Func<string, string?> _refusalLine;
    private CancellationTokenSource? _passkeyCancel;
    private bool _busy;
    private bool _awaitingPasskey;
    private bool _passkeyFailed;
    private bool _startFailed;
    private bool _inviteNeeded;
    private string? _refusal;

    public LegacyMigrationMove(
        Func<string, string, Task<DaemonResponse>> call,
        NearAiPasskeyStep passkey,
        Func<string, string?>? refusalLine = null)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
        _passkey = passkey ?? throw new ArgumentNullException(nameof(passkey));
        _refusalLine = refusalLine ?? LegacyMigrationOffers.RefusalLine;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Raised after a move succeeds, so the owner re-reads status.</summary>
    public event Action? Moved;

    /// <summary>Raised when a move attempt ends either way, so status is re-read.</summary>
    public event Action? Settled;

    public bool Busy => _busy;

    /// <summary>The move is waiting on the passkey step in the browser.</summary>
    public bool AwaitingPasskey => _awaitingPasskey;

    /// <summary>The passkey step ended without finishing; nothing was moved.</summary>
    public bool PasskeyFailed => _passkeyFailed;

    /// <summary>The move could not be started at all (no usable answer).</summary>
    public bool StartFailed => _startFailed;

    /// <summary>
    /// The daemon could not tell which invite this device joined with: the
    /// offer asks for it and the next Move sends it. Stays until a move succeeds.
    /// </summary>
    public bool InviteNeeded => _inviteNeeded;

    /// <summary>The core's sentence for the last refused move, or null.</summary>
    public string? Refusal => _refusal;

    /// <summary>Move, and on the passkey refusal run the passkey step and retry once.</summary>
    public async Task MoveAsync(string? invite)
    {
        if (_busy)
        {
            return;
        }

        string? sent = _inviteNeeded && !string.IsNullOrWhiteSpace(invite) ? invite : null;
        _refusal = null;
        _startFailed = false;
        _passkeyFailed = false;
        _busy = true;
        Changed();

        DaemonResponse outcome = await AskAsync(sent).ConfigureAwait(true);
        if (outcome.Error?.Message == LegacyMigrationOffers.PasskeyNeededLabel)
        {
            using var cancel = new CancellationTokenSource();
            _passkeyCancel = cancel;
            _awaitingPasskey = true;
            Changed();
            bool finished = await _passkey.RunAsync(cancel.Token).ConfigureAwait(true);
            bool cancelled = cancel.IsCancellationRequested;
            _passkeyCancel = null;
            _awaitingPasskey = false;
            if (!finished || cancelled)
            {
                _passkeyFailed = !cancelled;
                _busy = false;
                Changed();
                Settled?.Invoke();
                return;
            }

            outcome = await AskAsync(sent).ConfigureAwait(true);
        }

        _busy = false;
        if (!outcome.IsError)
        {
            _inviteNeeded = false;
            Changed();
            Moved?.Invoke();
        }
        else
        {
            // The message is a fixed legacy_migration_* label by contract.
            string label = outcome.Error?.Message ?? string.Empty;
            if (label == LegacyMigrationOffers.InviteNeededLabel)
            {
                _inviteNeeded = true;
            }

            _refusal = label.StartsWith("legacy_migration_", StringComparison.Ordinal)
                ? _refusalLine(label)
                : null;
            _startFailed = _refusal is null;
            Changed();
        }

        Settled?.Invoke();
    }

    /// <summary>
    /// Cancel while waiting on the passkey: stop waiting and end the browser
    /// step. Nothing is moved, and the offer stays.
    /// </summary>
    public async Task CancelPasskeyAsync()
    {
        CancellationTokenSource? cancel = _passkeyCancel;
        if (cancel is null)
        {
            return;
        }

        try
        {
            cancel.Cancel();
        }
        catch (ObjectDisposedException)
        {
            return;
        }

        await _passkey.CancelAsync().ConfigureAwait(true);
    }

    private async Task<DaemonResponse> AskAsync(string? invite)
    {
        try
        {
            return await _call(
                    DaemonProtocol.Methods.LegacyInviteMigrate,
                    LegacyMigrationOffers.MigrateParams(invite))
                .ConfigureAwait(true);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            System.Diagnostics.Trace.TraceWarning(nameof(LegacyMigrationMove));
            return new DaemonResponse { Error = new DaemonError { Code = "unavailable", Message = "move-not-started" } };
        }
    }

    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
}
