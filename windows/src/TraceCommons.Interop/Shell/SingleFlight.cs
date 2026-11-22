using System;
using System.Threading.Tasks;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// One piece of async work at a time, for a single thread (the UI thread): a
/// caller that arrives while it is running joins it and gets its result,
/// instead of being refused. The contribution startup used a busy flag that
/// answered "false" to everything that came in meanwhile, so a click on Queue,
/// History or Settings during the first launch's startup (which includes a
/// network update check) did nothing at all.
/// <para>
/// Not thread-safe by design: callers run on the UI thread and the bookkeeping
/// relies on it. Work that finishes before <see cref="Run"/> returns is never
/// kept, so a start that completes synchronously cannot wedge the next one.
/// </para>
/// </summary>
public sealed class SingleFlight<T>
{
    private Task<T>? _running;

    /// <summary>True while work is in flight.</summary>
    public bool IsRunning => _running is not null;

    /// <summary>
    /// Starts <paramref name="work"/> unless some is already running, in which
    /// case that is returned. When it completes, faulted or not, the next call
    /// starts new work.
    /// </summary>
    public Task<T> Run(Func<Task<T>> work)
    {
        if (_running is { } joined)
        {
            return joined;
        }

        Task<T> task = Guarded(work);
        if (!task.IsCompleted)
        {
            _running = task;
        }

        return task;
    }

    private async Task<T> Guarded(Func<Task<T>> work)
    {
        try
        {
            return await work().ConfigureAwait(true);
        }
        finally
        {
            _running = null;
        }
    }
}
