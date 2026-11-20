using System;
using System.Collections.Generic;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's station frames (checklist O3): each of a turn's frames is drawn
/// once per look and reused. The station lights and dims as transfers run;
/// before this the lit state was part of the key and every flip threw the
/// whole turn away and drew it again (box tc-win-az10: 4.0 s of CPU per
/// 10 s after the first turn). The lit state is now composed at draw time.
/// </summary>
public class LibraryFrameCacheTests
{
    private sealed class Frame : IDisposable
    {
        public bool Disposed;

        public void Dispose() => Disposed = true;
    }

    [Fact]
    public void TheSameLookKeepsItsFrames()
    {
        var cache = new LibraryFrameCache<Frame>();
        var frame = new Frame();
        cache.Look("launch|1|dark");
        cache.Put(3, frame);

        cache.Look("launch|1|dark");
        Assert.Same(frame, cache.Get(3));
        Assert.False(frame.Disposed);
    }

    [Fact]
    public void ANewLookDisposesEveryFrame()
    {
        var cache = new LibraryFrameCache<Frame>();
        var frames = new List<Frame> { new(), new() };
        cache.Look("launch|1|dark");
        cache.Put(0, frames[0]);
        cache.Put(1, frames[1]);

        cache.Look("launch|1.25|dark");
        Assert.All(frames, f => Assert.True(f.Disposed));
        Assert.Null(cache.Get(0));
        Assert.Null(cache.Get(1));
    }

    [Fact]
    public void AnUnknownFrameIsNull()
    {
        var cache = new LibraryFrameCache<Frame>();
        cache.Look("orb|1|light");
        Assert.Null(cache.Get(7));
    }

    [Fact]
    public void ClearingDisposesEveryFrameAndForgetsTheLook()
    {
        var cache = new LibraryFrameCache<Frame>();
        var frame = new Frame();
        cache.Look("launch|1|dark");
        cache.Put(0, frame);
        cache.Clear();
        Assert.True(frame.Disposed);
        cache.Look("launch|1|dark");
        Assert.Null(cache.Get(0));
    }

    /// <summary>The map keys a frame by its index alone and draws the lit state over it.</summary>
    [Fact]
    public void TheMapComposesTheLitStateAtDrawTime()
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/TracesMapView.cs.txt");
        string source = System.IO.File.ReadAllText(path);
        int draw = source.IndexOf("private void DrawLibrary(", StringComparison.Ordinal);
        string body = source[draw..source.IndexOf("\n    }\n", draw, StringComparison.Ordinal)];
        Assert.Contains("_libraryFrames.Get(index)", body, StringComparison.Ordinal);
        Assert.DoesNotContain("library.Active}", body, StringComparison.Ordinal);
    }
}
