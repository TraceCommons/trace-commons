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

    public async Task RunAsync(Func<Task> refresh)
    {
        ArgumentNullException.ThrowIfNull(refresh);
        lock (_gate)
        {
            if (_running)
            {
                _requested = true;
                return;
            }
            _running = true;
        }

        ExceptionDispatchInfo? failure = null;
        while (true)
        {
            try
            {
                await refresh().ConfigureAwait(true);
            }
            catch (Exception exception)
            {
                // Preserve the caller's error while still servicing an event
                // that arrived during the failed snapshot. No event, no retry.
                failure ??= ExceptionDispatchInfo.Capture(exception);
            }

            lock (_gate)
            {
                if (_requested)
                {
                    _requested = false;
                    continue;
                }
                _running = false;
                break;
            }
        }
        failure?.Throw();
    }
}
