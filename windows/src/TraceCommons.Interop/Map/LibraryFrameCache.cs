using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The station's (or the orb's) drawn frames, one per frame of the turn and
/// lit state, for one look: style, size and theme (checklist O3). A frame
/// is drawn once and reused on every later turn. The lit state flips as
/// transfers run, so each state keeps its own frames; only a new look
/// throws them away. The caller owns nothing it has put here.
/// </summary>
public sealed class LibraryFrameCache<TFrame> where TFrame : class, IDisposable
{
    private readonly Dictionary<(bool Lit, int Index), TFrame> _frames = new();
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

    /// <summary>The frame drawn for this lit state and index, or null.</summary>
    public TFrame? Get(bool lit, int index) => _frames.TryGetValue((lit, index), out TFrame? frame) ? frame : null;

    /// <summary>Keeps <paramref name="frame"/>; this cache disposes it with its look.</summary>
    public void Put(bool lit, int index, TFrame frame) => _frames[(lit, index)] = frame;
}
