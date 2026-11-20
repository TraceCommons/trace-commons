using System;
using System.Collections.Generic;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's station frames (checklist O3): each of a turn's frames is drawn
/// once per look and reused. The station lights and dims as transfers run,
/// and before this every flip threw the whole turn away and drew it again,
/// so the map kept paying the first turn's cost (box tc-win-az10:
/// 4.0 s of CPU per 10 s after the first turn).
/// </summary>
public class LibraryFrameCacheTests
{
    private sealed class Frame : IDisposable
    {
        public bool Disposed;

        public void Dispose() => Disposed = true;
    }

    [Fact]
    public void ALitFlipKeepsBothStatesFrames()
    {
        var cache = new LibraryFrameCache<Frame>();
        var dim = new Frame();
        var lit = new Frame();
        cache.Look("launch|1|dark");
        cache.Put(false, 3, dim);
        cache.Put(true, 3, lit);

        cache.Look("launch|1|dark");
        Assert.Same(dim, cache.Get(false, 3));
        Assert.Same(lit, cache.Get(true, 3));
        Assert.False(dim.Disposed);
        Assert.False(lit.Disposed);
    }

    [Fact]
    public void ANewLookDisposesEveryFrame()
    {
        var cache = new LibraryFrameCache<Frame>();
        var frames = new List<Frame> { new(), new() };
        cache.Look("launch|1|dark");
        cache.Put(false, 0, frames[0]);
        cache.Put(true, 1, frames[1]);

        cache.Look("launch|1.25|dark");
        Assert.All(frames, f => Assert.True(f.Disposed));
        Assert.Null(cache.Get(false, 0));
        Assert.Null(cache.Get(true, 1));
    }

    [Fact]
    public void AnUnknownFrameIsNull()
    {
        var cache = new LibraryFrameCache<Frame>();
        cache.Look("orb|1|light");
        Assert.Null(cache.Get(true, 7));
    }

    [Fact]
    public void ClearingDisposesEveryFrameAndForgetsTheLook()
    {
        var cache = new LibraryFrameCache<Frame>();
        var frame = new Frame();
        cache.Look("launch|1|dark");
        cache.Put(true, 0, frame);
        cache.Clear();
        Assert.True(frame.Disposed);
        cache.Look("launch|1|dark");
        Assert.Null(cache.Get(true, 0));
    }
}
