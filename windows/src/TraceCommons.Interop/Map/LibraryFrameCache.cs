using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The station's (or the orb's) drawn frames, one per frame of the turn,
/// for one look: style, lit state, size and theme. A frame is drawn once
/// and reused on every later turn; a new look disposes them all, and so
/// does a lost device (<see cref="Clear"/>). The caller owns nothing it has
/// put here.
/// </summary>
public sealed class LibraryFrameCache<TFrame> where TFrame : class, IDisposable
{
    private readonly Dictionary<int, TFrame> _frames = new();
    private string? _look;

    /// <summary>The look the next frames are for; a different one disposes every frame held.</summary>
    public void Look(string look)
    {
        if (look == _look)
        {
            return;
        }

        Clear();
        _look = look;
    }

    /// <summary>Disposes every frame held and forgets the look (the device was lost).</summary>
    public void Clear()
    {
        foreach (TFrame frame in _frames.Values)
        {
            frame.Dispose();
        }

        _frames.Clear();
        _look = null;
    }

    /// <summary>The frame drawn for this index of the turn, or null.</summary>
    public TFrame? Get(int index) => _frames.TryGetValue(index, out TFrame? frame) ? frame : null;

    /// <summary>Keeps <paramref name="frame"/>; this cache disposes it with its look.</summary>
    public void Put(int index, TFrame frame) => _frames[index] = frame;
}
