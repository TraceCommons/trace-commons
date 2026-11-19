using System;

namespace TraceCommons.App;

/// <summary>
/// Whether the main window's clocks may run: false while it is hidden to the
/// tray or minimised. The main window sets it from its
/// <see cref="TraceCommons.Interop.Shell.WindowClockGate"/>; a control with a
/// timer of its own listens here and stops while it is false.
/// </summary>
internal static class WindowClocks
{
    public static bool Running { get; private set; } = true;

    public static event Action<bool>? Changed;

    public static void Set(bool running)
    {
        if (Running == running)
        {
            return;
        }

        Running = running;
        Changed?.Invoke(running);
    }
}
