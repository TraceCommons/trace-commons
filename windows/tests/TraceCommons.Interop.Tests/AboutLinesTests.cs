using System.Collections.Generic;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The About window (spec W24). Each expected line was printed by a Swift
/// probe that compiled <c>AboutLines.line</c> out of
/// macos/Sources/TraceCommonsApp/Views/AboutWindowView.swift:59-63 at the
/// branch's base.
/// </summary>
public sealed class AboutLinesTests
{
    [Theory]
    [InlineData("1.0.10", "42", "1.0.10 (42)")]
    [InlineData("1.0.10", null, "1.0.10")]
    [InlineData("1.0.10", "", "1.0.10")]
    [InlineData("1.0.10", "1.0.10", "1.0.10")]
    [InlineData(null, "42", "42")]
    [InlineData("", "42", "42")]
    [InlineData(null, null, "")]
    [InlineData("0.12.0", "0.12.0.0", "0.12.0 (0.12.0.0)")]
    public void TheVersionLineIsTheMacs(string? shortVersion, string? build, string expected)
    {
        Assert.Equal(expected, AboutLines.Line(shortVersion, build));
    }

    // Copy puts exactly the shown line on the clipboard
    // (AboutLines.copyVersion, AboutWindowView.swift:65-68).
    [Fact]
    public void CopyPutsExactlyTheShownLineOnTheClipboard()
    {
        var copied = new List<string>();
        var model = new AboutModel("1.0.10", "42", copyright: null, copied.Add);
        model.CopyVersion();
        Assert.Equal(new[] { "1.0.10 (42)" }, copied);
        Assert.Equal(model.Version, copied[0]);
    }

    // The Mac's Info.plist copyright (macos/scripts/info-plist.sh:93) is shown
    // only when there is one (AboutWindowView.swift:28-32).
    [Fact]
    public void AMissingCopyrightIsNotShown()
    {
        Assert.False(new AboutModel("1.0.10", null, null, _ => { }).ShowsCopyright);
        Assert.False(new AboutModel("1.0.10", null, "", _ => { }).ShowsCopyright);
        Assert.True(new AboutModel("1.0.10", null, "K&Z Partners LLC", _ => { }).ShowsCopyright);
    }

    // Every word is the core's: the window's title is the app menu's
    // "About <app>" (preview_copy.rs about_app), the name is brand.rs's, and
    // the button is the Settings table's copy (SettingsLegacyWords.copy).
    [Fact]
    public void TheWordsAreTheCores()
    {
        string name = MonitorWords.AppName;
        Assert.False(string.IsNullOrEmpty(name));
        Assert.Equal("About " + name, AboutLines.Title);
        Assert.Equal("Copy", AboutLines.CopyWord);
        Assert.Equal(name, AboutLines.Name);
    }

    // The informational version an SDK build stamps is "<version>+<revision>"
    // when source revisions are included; the window shows the version only.
    [Theory]
    [InlineData("1.0.10", "1.0.10")]
    [InlineData("1.0.10+0123abcd", "1.0.10")]
    [InlineData(null, "")]
    public void TheAssemblyVersionIsTheShortVersion(string? informational, string expected)
    {
        Assert.Equal(expected, AboutLines.ShortVersionFrom(informational));
    }

    // A packaged build reads the package's version; make-msix.ps1 stamps
    // "<version>.0", which is the version alone.
    [Theory]
    [InlineData(1, 0, 10, 0, "1.0.10")]
    [InlineData(1, 0, 10, 3, "1.0.10 (1.0.10.3)")]
    public void APackageVersionIsTheVersionAndItsBuild(int major, int minor, int build, int revision, string expected)
    {
        (string version, string? number) = AboutLines.FromPackageVersion((ushort)major, (ushort)minor, (ushort)build, (ushort)revision);
        Assert.Equal(expected, AboutLines.Line(version, number));
    }
}
