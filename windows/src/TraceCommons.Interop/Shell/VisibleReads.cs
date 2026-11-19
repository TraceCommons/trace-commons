using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Whether a tab's reads may follow the daemon: only while its tab is chosen
/// AND the window can be seen (not hidden to the tray, not minimised). The
/// Mac's <c>.task</c> runs while the view is on screen and starts again when
/// it reappears (InferenceViews.swift:48-56), so a return to view reads
/// everything afresh, and nothing reads while it cannot be seen (one-pass 13).
/// <see cref="Changed"/> fires on each start (true) and stop (false), once.
/// </summary>
public sealed class VisibleReads
{
    private bool _tabShown;
    private bool _windowShown = true;

    public bool Reading => _tabShown && _windowShown;

    public event Action<bool>? Changed;

    public void SetTabShown(bool shown) => Update(() => _tabShown = shown);

    public void SetWindowShown(bool shown) => Update(() => _windowShown = shown);

    private void Update(Action change)
    {
        bool before = Reading;
        change();
        if (Reading != before) Changed?.Invoke(Reading);
    }
}
