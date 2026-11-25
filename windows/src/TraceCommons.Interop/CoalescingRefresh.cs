using System;
using System.Runtime.ExceptionServices;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// Serializes refreshes and coalesces events during each snapshot into one
/// follow-up read. The callback retains its caller's synchronization context.
/// </summary>
public sealed class CoalescingRefresh
{
    private readonly object _gate = new();
    private bool _running;
    private bool _requested;
    private TaskCompletionSource? _nextSnapshot;

    public Task RunAsync(Func<Task> refresh) => RunAsync(refresh, waitForRefresh: false);

    /// <summary>When requested, wait for the snapshot scheduled by this call even if another read is in flight.</summary>
    public async Task RunAsync(Func<Task> refresh, bool waitForRefresh)
    {
        ArgumentNullException.ThrowIfNull(refresh);
        Task? queued = null;
        lock (_gate)
        {
            if (_running)
            {
                _requested = true;
                if (!waitForRefresh)
                {
                    return;
                }

                queued = (_nextSnapshot ??= new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously)).Task;
            }
            else
            {
                _running = true;
            }
        }

        if (queued is not null)
        {
            await queued.ConfigureAwait(true);
            return;
        }

        ExceptionDispatchInfo? failure = null;
        TaskCompletionSource? snapshotCompletion = null;
        while (true)
        {
            try
            {
                await refresh().ConfigureAwait(true);
                snapshotCompletion?.SetResult();
            }
            catch (Exception exception)
            {
                // Preserve the caller's error while still servicing an event
                // that arrived during the failed snapshot. No event, no retry.
                failure ??= ExceptionDispatchInfo.Capture(exception);
                snapshotCompletion?.SetException(exception);
            }

            lock (_gate)
            {
                if (_requested)
                {
                    _requested = false;
                    snapshotCompletion = _nextSnapshot;
                    _nextSnapshot = null;
                    continue;
                }
                _running = false;
                break;
            }
        }
        failure?.Throw();
    }
}
