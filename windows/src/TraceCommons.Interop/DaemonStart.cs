using System;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// One daemon start at a time on one state directory. A second start while
/// one is under way joins it rather than racing it: the daemon holds a lock
/// on its state directory, so a second start would be refused for the lock
/// the first one holds, and would be reported as another instance running.
/// </summary>
public sealed class DaemonStartGate
{
    private Task? _inFlight;

    /// <summary>Whether a start is under way.</summary>
    public bool Starting => _inFlight is not null;

    /// <summary>
    /// Runs <paramref name="start"/> unless the daemon is already
    /// <paramref name="running"/>. A start already under way is awaited
    /// first: if it brought the daemon up, nothing more runs; if it failed,
    /// its failure is this call's too, unless <paramref name="retryAfterFailure"/>
    /// (a start that carries the contributor's new settings, which can clear
    /// the refusal the persisted settings earned), which then runs its own.
    /// Call on one thread, as the window's dispatcher does.
    /// </summary>
    public async Task RunAsync(Func<Task> start, Func<bool> running, bool retryAfterFailure = false)
    {
        if (_inFlight is { } current)
        {
            try
            {
                await current.ConfigureAwait(true);
            }
            catch (Exception) when (retryAfterFailure)
            {
            }
        }

        if (running()) return;
        Task task = start();
        _inFlight = task;
        try
        {
            await task.ConfigureAwait(true);
        }
        finally
        {
            if (ReferenceEquals(_inFlight, task)) _inFlight = null;
        }
    }
}

/// <summary>
/// The Mac starts its daemon when the app launches (<c>model.start()</c>,
/// TraceCommonsAppMain.swift:476, reaching <c>startDaemon</c>,
/// AppModel.swift:1782) and reads its feature flags as soon as the daemon is
/// up (<c>refreshAll</c>, AppModel.swift:2085). The Windows window opens on
/// Insights, which needs no daemon, so this starts one there for the same
/// reads. Quietly: a refusal (session sources not declared, another instance
/// holding the state directory) leaves no daemon and every flag off, as on
/// the Mac, and puts nothing on screen; the explicit contribution start still
/// asks for the sources and reports its own failure.
/// </summary>
public static class LaunchDaemonStart
{
    /// <summary>Runs <paramref name="start"/>; true when it returned, false when it refused.</summary>
    public static async Task<bool> StartQuietlyAsync(Func<Task> start)
    {
        try
        {
            await start().ConfigureAwait(true);
            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }
}
