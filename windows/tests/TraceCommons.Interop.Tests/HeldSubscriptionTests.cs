using System;
using System.IO;
using System.Runtime.CompilerServices;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A handler on a static event, held by an element between Loaded and
/// Unloaded. WinUI can raise Loaded twice without an Unloaded between (an
/// element re-parented, a template applied again), so a handler added on
/// each Loaded doubles, and the one Unloaded removes only one copy: the
/// static event then keeps the element alive for the life of the process.
/// The hold is idempotent instead.
/// </summary>
public class HeldSubscriptionTests
{
    private static event Action? StaticChanged;

    private sealed class Element
    {
        public int Updates;
        public readonly HeldSubscription Motion;

        public Element()
        {
            Motion = new HeldSubscription(() => StaticChanged += Update, () => StaticChanged -= Update);
        }

        private void Update() => Updates++;
    }

    private static int Handlers() => StaticChanged?.GetInvocationList().Length ?? 0;

    [Fact]
    public void LoadedTwiceHoldsTheHandlerOnce()
    {
        StaticChanged = null;
        var element = new Element();

        element.Motion.Hold();
        element.Motion.Hold();
        Assert.Equal(1, Handlers());

        StaticChanged?.Invoke();
        Assert.Equal(1, element.Updates);

        element.Motion.Release();
        Assert.Equal(0, Handlers());
        Assert.False(element.Motion.Held);
    }

    [Fact]
    public void ReleaseWithoutHoldDoesNothing()
    {
        StaticChanged = null;
        var element = new Element();

        element.Motion.Release();
        element.Motion.Hold();
        Assert.Equal(1, Handlers());
        element.Motion.Release();
        element.Motion.Release();
        Assert.Equal(0, Handlers());
    }

    [Fact]
    public void AnUnloadedElementLoadedTwiceIsCollected()
    {
        StaticChanged = null;
        WeakReference alive = LoadTwiceThenUnload();

        for (int i = 0; i < 3 && alive.IsAlive; i++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
        }

        Assert.False(alive.IsAlive, "the static event still holds the element");
        Assert.Equal(0, Handlers());
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static WeakReference LoadTwiceThenUnload()
    {
        var element = new Element();
        element.Motion.Hold();
        element.Motion.Hold();
        element.Motion.Release();
        return new WeakReference(element);
    }

    /// <summary>
    /// Every element that listens to the motion setting on Loaded holds it
    /// through <see cref="HeldSubscription"/>, never with a bare
    /// <c>+=</c> in a Loaded handler.
    /// </summary>
    [Theory]
    [InlineData("TraceCommons.App/Look/GlassClock.cs")]
    [InlineData("TraceCommons.App/Look/GlassSkeleton.cs")]
    public void LoadedHandlersHoldTheMotionSettingIdempotently(string relativePath)
    {
        string source = ShellSource(relativePath);
        Assert.DoesNotContain("GlassMotionSettings.Changed += Update;", source, StringComparison.Ordinal);
        Assert.Contains("new HeldSubscription(", source, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
