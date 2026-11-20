using System;
using TraceCommons.Interop;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace TraceCommons.App;

public static class Program
{
    private static readonly ActivationInbox<AppActivationArguments> Activations = new();

    [STAThread]
    private static int Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        // Subscribe before claiming the key: another process can redirect as
        // soon as registration succeeds, before the XAML window exists.
        AppInstance.GetCurrent().Activated += (_, activation) => Activations.Enqueue(activation);
        try
        {
            // A recorded-sample launch stays out of single-instancing: it
            // would hand itself to a running app and show that app's data,
            // or hold the key and take a real launch's activation.
            if (!IsSampleLaunch)
            {
                var holder = AppInstance.FindOrRegisterForKey("TraceCommons.Main");
                if (!holder.IsCurrent && Redirect(holder)) return 0;
            }
        }
        catch (Exception)
        {
            // SDK failure starts a local window, which may lose the daemon lock.
            // This fallback cannot deliver the invite to the existing instance;
            // the contributor must reopen it after recovering that window.
            // Activation credentials and exception messages are never logged.
        }
#if DEBUG
        // A sample launch's throwaway state (SampleDaemon.ThrowawayConfigDir)
        // goes with it; a force-stopped one's is removed by the next launch.
        _ = SampleDaemon.SweepThrowawayDirs(System.IO.Path.GetTempPath(), Environment.ProcessId, SampleDaemon.IsProcessRunning);
        AppDomain.CurrentDomain.ProcessExit += (_, _) => RemoveOwnThrowawayDir();
#endif
        Application.Start(initialization =>
        {
            SynchronizationContext.SetSynchronizationContext(
                new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App();
        });
#if DEBUG
        RemoveOwnThrowawayDir();
#endif
        return 0;
    }

#if DEBUG
    private static void RemoveOwnThrowawayDir() =>
        _ = SampleDaemon.RemoveThrowawayDir(System.IO.Path.GetTempPath(), Environment.ProcessId);
#endif

    /// <summary>
    /// Whether this launch reads recorded samples instead of a daemon
    /// (<c>TRACE_COMMONS_SAMPLE</c>, Debug builds only; always false in
    /// Release). Such a launch changes nothing on this machine outside the
    /// window: no single-instance key, URL scheme, run at login or witness
    /// files.
    /// </summary>
    internal static bool IsSampleLaunch =>
#if DEBUG
        SampleDaemon.IsRequested();
#else
        false;
#endif

    internal static void ReceiveActivations(Action<AppActivationArguments> receive) => Activations.Attach(receive);

    private static bool Redirect(AppInstance holder)
    {
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        var completed = new EventWaitHandle(false, EventResetMode.ManualReset);
        var redirect = Task.Run(async () =>
        {
            try { await holder.RedirectActivationToAsync(activation); return true; }
            catch (Exception) { return false; }
            finally { completed.Set(); }
        });
        try
        {
            // Pump COM on the STA thread while the MTA performs redirection.
            // A stale holder must not hang a launch indefinitely.
            // Timeout falls back to a local window, not confirmed delivery.
            uint result = Win32NativeMethods.CoWaitForMultipleObjects(0, 30000, 1,
                new[] { completed.SafeWaitHandle.DangerousGetHandle() }, out _);
            return result == 0 && redirect.GetAwaiter().GetResult();
        }
        finally
        {
            // On timeout the worker still owns the event until it signals.
            // Dispose only after both the native wait and worker are finished.
            _ = redirect.ContinueWith(_ => completed.Dispose(), TaskScheduler.Default);
        }
    }

}
