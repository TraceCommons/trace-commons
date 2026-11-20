using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// A system setting read once and again only when the system says it changed
/// (WM_SETTINGCHANGE): the tray and taskbar frames draw from it many times a
/// second, and a registry read a frame is work with an event to replace it
/// (one-pass checklist O3).
/// </summary>
public sealed class CachedReading<T>
{
    private readonly Func<T> _read;
    private T _value;

    public CachedReading(Func<T> read)
    {
        _read = read;
        _value = read();
    }

    /// <summary>The value as last read.</summary>
    public T Value => _value;

    /// <summary>Reads the setting again; true when it changed.</summary>
    public bool Refresh()
    {
        T now = _read();
        if (EqualityComparer<T>.Default.Equals(now, _value))
        {
            return false;
        }

        _value = now;
        return true;
    }
}
