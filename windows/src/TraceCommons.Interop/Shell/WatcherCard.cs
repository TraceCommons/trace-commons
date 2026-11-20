namespace TraceCommons.Interop.Shell;

/// <summary>
/// The Settings header's chip (<c>SettingsModal.chip</c>,
/// SettingsModal.swift:243-248): the core's Watching or Paused over the
/// daemon's status, with the on or off dot.
/// </summary>
public sealed record SettingsHeaderChip(string Word, bool On)
{
    /// <summary>
    /// The chip for <paramref name="paused"/>; null without words. The Mac
    /// passes the model's <c>status.paused</c>, whose placeholder is false,
    /// so the chip reads Watching until a status says otherwise
    /// (TraceCommonsAppMain.swift:190-192).
    /// </summary>
    public static SettingsHeaderChip? For(bool paused, string watching, string pausedWord)
    {
        string word = paused ? pausedWord : watching;
        return word.Length == 0 ? null : new SettingsHeaderChip(word, !paused);
    }

    /// <summary>The chip from the core's monitor table (<c>watching</c>, <c>paused</c>).</summary>
    public static SettingsHeaderChip? Live(bool paused) =>
        For(paused, MonitorShellCopy.Top("watching"), MonitorShellCopy.Top("paused"));
}

/// <summary>
/// The Watching section's watcher card (<c>WatchingSection</c>,
/// WatchingSection.swift): drawn once the status has answered, with the
/// core's Watching or Paused chip (muted while paused), what pausing does,
/// the queue limit, and Pause and Resume, each enabled only in the state it
/// changes and only while that state is current. Before the status answers
/// only the queue limit is drawn.
/// </summary>
public sealed record WatcherCard(bool Drawn, string Chip, bool ChipMuted, bool PauseEnabled, bool ResumeEnabled)
{
    /// <summary>
    /// Pause and Resume act on the state the chip shows, so they are armed
    /// only while the status answered, its last read did not fail and the
    /// core is running (<c>controlsArmed</c>, WatchingSection.swift:70-72).
    /// </summary>
    public static bool ControlsArmed(bool statusAnswered, bool lastReadFailed, bool running) =>
        statusAnswered && !lastReadFailed && running;

    /// <param name="statusAnswered">A status has answered (<c>statusRead == .answered</c>, :10).</param>
    /// <param name="paused">The last answered status's <c>paused</c>.</param>
    /// <param name="lastReadFailed">The latest status read failed.</param>
    /// <param name="running">The core is running.</param>
    public static WatcherCard Of(
        bool statusAnswered, bool paused, bool lastReadFailed, bool running, string watching, string pausedWord)
    {
        if (!statusAnswered)
        {
            return new WatcherCard(false, string.Empty, false, false, false);
        }

        bool armed = ControlsArmed(statusAnswered, lastReadFailed, running);
        // :45-46 the chip and its muting; :57 Pause off while paused or
        // unarmed; :60 Resume off while watching or unarmed.
        return new WatcherCard(
            true,
            paused ? pausedWord : watching,
            paused,
            !paused && armed,
            paused && armed);
    }
}

/// <summary>
/// The watcher card's reads and writes (<c>AppModel.pause(until: nil)</c>,
/// <c>resume()</c>, <c>refreshSettings()</c>): the status and the daemon's
/// settings, Pause until resumed (empty parameters, as
/// <see cref="PauseRequest"/> builds for <see cref="PauseDuration.UntilResumed"/>)
/// and Resume, each followed by a status read. Every error reads as a failed
/// read; nothing throws out. Continuations resume on the caller's context.
/// </summary>
public sealed class WatcherFlow
{
    private readonly System.Func<string, string, System.Threading.Tasks.Task<DaemonResponse>> _call;
    private long _statusTicket;

    public WatcherFlow(System.Func<string, string, System.Threading.Tasks.Task<DaemonResponse>> call)
    {
        _call = call;
    }

    /// <summary>A status has answered.</summary>
    public bool StatusAnswered { get; private set; }

    /// <summary>The last answered status's <c>paused</c>; false until one answers.</summary>
    public bool Paused { get; private set; }

    /// <summary>The latest status read failed.</summary>
    public bool StatusReadFailed { get; private set; }

    /// <summary>The daemon's <c>queue_ttl_days</c>, once settings answered.</summary>
    public long? QueueTtlDays { get; private set; }

    /// <summary>The latest settings read failed.</summary>
    public bool SettingsReadFailed { get; private set; }

    /// <summary>A Pause or Resume is out.</summary>
    public bool Busy { get; private set; }

    public WatcherCard Card(bool running) =>
        WatcherCard.Of(StatusAnswered, Paused, StatusReadFailed, running,
            MonitorShellCopy.Shell("watcher_watching"), MonitorShellCopy.Shell("watcher_paused"));

    /// <summary>Reads the status; a reply overtaken by a newer read is dropped.</summary>
    public async System.Threading.Tasks.Task ReadStatusAsync()
    {
        long ticket = ++_statusTicket;
        DaemonStatus? status = null;
        try
        {
            DaemonResponse response = await _call(DaemonProtocol.Methods.Status, "{}").ConfigureAwait(true);
            if (!response.IsError)
            {
                status = response.ResultAs<DaemonStatus>();
            }
        }
        catch (System.Exception)
        {
            status = null;
        }

        if (ticket != _statusTicket)
        {
            return;
        }

        StatusReadFailed = status is null;
        if (status is not null)
        {
            StatusAnswered = true;
            Paused = status.Paused;
        }
    }

    /// <summary>Reads the daemon's settings for the queue limit.</summary>
    public async System.Threading.Tasks.Task ReadSettingsAsync()
    {
        DaemonSettingsSnapshot? settings = null;
        try
        {
            DaemonResponse response = await _call(DaemonProtocol.Methods.GetSettings, "{}").ConfigureAwait(true);
            if (!response.IsError)
            {
                settings = response.ResultAs<DaemonSettingsSnapshot>();
            }
        }
        catch (System.Exception)
        {
            settings = null;
        }

        SettingsReadFailed = settings is null;
        if (settings is not null)
        {
            QueueTtlDays = settings.QueueTtlDays;
        }
    }

    /// <summary>Pauses until resumed, then reads the status again.</summary>
    public System.Threading.Tasks.Task PauseAsync() => WriteAsync(DaemonProtocol.Methods.Pause, PauseRequest.Serialize(PauseDuration.UntilResumed, System.DateTimeOffset.UtcNow));

    /// <summary>Resumes, then reads the status again.</summary>
    public System.Threading.Tasks.Task ResumeAsync() => WriteAsync(DaemonProtocol.Methods.Resume, "{}");

    private async System.Threading.Tasks.Task WriteAsync(string method, string parameters)
    {
        if (Busy)
        {
            return;
        }

        Busy = true;
        try
        {
            await _call(method, parameters).ConfigureAwait(true);
        }
        catch (System.Exception)
        {
            // The status read below says what state stands.
        }
        finally
        {
            Busy = false;
        }

        await ReadStatusAsync().ConfigureAwait(true);
    }
}
