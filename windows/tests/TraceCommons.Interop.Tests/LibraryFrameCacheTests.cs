using System;
using System.Collections.Generic;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's station frames (checklist O3): each of a turn's frames is drawn
/// once per look and reused, and every frame of an old look, or of a lost
/// device, is disposed.
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

    /// <summary>
    /// The art is rasterised once per frame, into a bitmap the glow and the
    /// frame both read: a command list was rasterised twice, once for the
    /// glow's shadow and once for the image (box tc-win-az10, armedFolder,
    /// motion on: the first turn's CPU fell from 22.1 s to about 18 s).
    /// </summary>
    [Fact]
    public void TheStationsArtIsDrawnOncePerFrame()
    {
        string path = System.IO.Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/TracesMapView.cs.txt");
        string source = System.IO.File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
        int draw = source.IndexOf("private void DrawLibrary(", StringComparison.Ordinal);
        Assert.True(draw >= 0, "DrawLibrary is gone");
        string body = source[draw..source.IndexOf("\n    }\n", draw, StringComparison.Ordinal)];
        Assert.DoesNotContain("CanvasCommandList", body, StringComparison.Ordinal);
        Assert.Contains("var art = new CanvasRenderTarget(", body, StringComparison.Ordinal);
        Assert.Contains("_libraryFrames.Get(index)", body, StringComparison.Ordinal);
    }
}
