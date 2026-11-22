using System;
using System.Threading.Tasks;

namespace TraceCommons.App;

/// <summary>
/// Work started without waiting for it, whose failure someone should still see.
/// A discarded <c>Task</c> that faults is never observed and is lost; an
/// <c>async void</c> that throws ends the process. This awaits the task and
/// records what it threw (the type, never the message) in the crash log. It
/// never throws itself.
/// </summary>
internal static class Observed
{
    internal static async Task Run(Task task, string where)
    {
        try
        {
            await task.ConfigureAwait(true);
        }
        catch (Exception exception)
        {
            CrashGuard.Log.Record(where, exception);
        }
    }
}
