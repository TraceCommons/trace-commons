using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// What the shell does with an exception nothing caught. XAML ends the process
/// with a stowed exception (0xc000027b) when one escapes a UI-thread handler
/// and the app has no <c>UnhandledException</c> handler; that tells nobody
/// which exception it was. So each is written to
/// <c>shell-crashes.log</c> in the state directory (type, HResult and frames,
/// never the message), once per type per run.
/// <para>
/// While a first-run window is open, a fault whose stack runs through the
/// first-run window or flow is also survived
/// (<see cref="UnhandledExceptionEventArgs.Handled"/>), so a screen that fails
/// to build leaves the person on Welcome to try again. Any other fault, the
/// main window's included, is logged and surfaces as it always did.
/// </para>
/// </summary>
internal static class CrashGuard
{
    internal static readonly CrashLog Log = new(DaemonHost.DefaultConfigDir());

    internal static readonly FirstRunWindowTracker FirstRun = new();

    internal static void Install(Application app)
    {
        app.UnhandledException += (_, e) =>
        {
            Log.Record("xaml", e.Exception);
            if (FirstRun.Survives(e.Exception))
            {
                e.Handled = true;
            }
        };
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            if (e.ExceptionObject is Exception exception)
            {
                Log.Record("appdomain", exception);
            }
        };
        TaskScheduler.UnobservedTaskException += (_, e) => Log.Record("task", e.Exception);
    }

    /// <summary>Counts <paramref name="window"/> as an open first-run window until it closes.</summary>
    internal static void TrackFirstRun(Window window)
    {
        FirstRun.Enter();
        bool left = false;
        window.Closed += (_, _) =>
        {
            if (!left)
            {
                left = true;
                FirstRun.Exit();
            }
        };
    }
}
