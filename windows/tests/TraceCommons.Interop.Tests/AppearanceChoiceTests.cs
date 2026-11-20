using System;
using System.IO;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > General's appearance (W17-D1 (a)) against the Mac's
/// <c>GlassAppearance</c> (GlassTheme.swift:98-138): System until one is
/// chosen, the choice applies at once and is kept for the next launch.
/// </summary>
public sealed class AppearanceChoiceTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "w16-appearance-" + Guid.NewGuid().ToString("N"));

    private string File_ => Path.Combine(_dir, "appearance.txt");

    public void Dispose()
    {
        try
        {
            Directory.Delete(_dir, recursive: true);
        }
        catch (IOException)
        {
        }
    }

    [Fact]
    public void NothingStoredIsSystem()
    {
        Assert.Equal(Appearance.System, new AppearanceChoice(File_).Current);
        Assert.Equal(Appearance.System, AppearanceChoice.Parse("sepia"));
    }

    [Fact]
    public void AChoiceAppliesAtOnceAndIsKeptForTheNextLaunch()
    {
        var choice = new AppearanceChoice(File_);
        Appearance? applied = null;
        choice.Changed += a => applied = a;
        choice.Choose(Appearance.Dark);
        Assert.Equal(Appearance.Dark, applied);
        // GlassAppearance's raw value, stored as the Mac stores it.
        Assert.Equal("dark", File.ReadAllText(File_));
        Assert.Equal(Appearance.Dark, new AppearanceChoice(File_).Current);
    }

    [Fact]
    public void ASampleLaunchWritesNothing()
    {
        var choice = new AppearanceChoice(File_) { ReadOnly = true };
        choice.Choose(Appearance.Light);
        Assert.Equal(Appearance.Light, choice.Current);
        Assert.False(File.Exists(File_));
    }
}
