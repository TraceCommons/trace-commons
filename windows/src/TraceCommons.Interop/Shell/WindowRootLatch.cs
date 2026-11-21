using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// A window's root, read only while the window is known to be open.
/// <c>Window.Close()</c> raises a last deactivation after the window has
/// closed, and reading <c>Window.Content</c> then throws across the ABI,
/// which ends the process with 0xc000027b. So a deactivation answers from
/// what is already known and never reads the window.
/// </summary>
public sealed class WindowRootLatch<TRoot> where TRoot : class
{
    private readonly Func<TRoot?> _read;
    private TRoot? _root;

    public WindowRootLatch(Func<TRoot?> read) => _read = read;

    /// <summary>The root as last read. Never touches the window.</summary>
    public TRoot? Known => _root;

    /// <summary>Reads the root if it is not known yet. Only for a window that is open.</summary>
    public TRoot? WhileOpen() => _root ??= _read();

    /// <summary>The root for an activation change: read when activating, never when deactivating.</summary>
    public TRoot? ForActivation(bool activating) => activating ? WhileOpen() : Known;
}
