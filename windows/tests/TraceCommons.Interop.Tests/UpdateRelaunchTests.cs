using System;
using System.IO;
using System.Linq;
using System.Text;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The helper that starts the app again after "Update and restart".
///
/// <para>
/// The deployment ends the app's own process and installs the new version,
/// and nothing starts it again: on tc-win-az3 (2026-11-20) the app stayed
/// closed after every in-app update, with or without
/// <c>RegisterApplicationRestart</c>. A process the app starts survives that
/// ending (the same box, same day), so the app starts a hidden Windows
/// PowerShell helper just before the hand-off. It waits for this package
/// family to be registered at a different version, then starts the app
/// through its AUMID, and gives up after a bounded wait if the update never
/// lands.
/// </para>
/// </summary>
public class UpdateRelaunchTests
{
    private const string Name = "Iqlusion.TraceCommons";
    private const string Family = "Iqlusion.TraceCommons_v13bsb4pn17ym";
    private const string Version = "1.0.20.0";

    [Fact]
    public void TheHelperStartsTheAppOnlyOnceAnotherVersionIsRegistered()
    {
        string script = UpdateRelaunch.Script(Name, Family, Version);

        int poll = script.IndexOf("Get-AppxPackage -Name 'Iqlusion.TraceCommons'", StringComparison.Ordinal);
        int compare = script.IndexOf("$p.Version.ToString() -ne '1.0.20.0'", StringComparison.Ordinal);
        int launch = script.IndexOf("Start-Process 'shell:AppsFolder\\Iqlusion.TraceCommons_v13bsb4pn17ym!App'", StringComparison.Ordinal);
        Assert.True(poll >= 0, script);
        Assert.True(compare > poll, script);
        Assert.True(launch > compare, script);
        Assert.Contains("$_.PackageFamilyName -eq 'Iqlusion.TraceCommons_v13bsb4pn17ym'", script, StringComparison.Ordinal);
        Assert.Equal(1, CountOf(script, "Start-Process"));
    }

    [Fact]
    public void TheHelperGivesUpAfterABoundedWait()
    {
        string script = UpdateRelaunch.Script(Name, Family, Version);

        Assert.Contains($"AddSeconds({UpdateRelaunch.WaitSeconds})", script, StringComparison.Ordinal);
        Assert.Contains("while ((Get-Date) -lt $deadline)", script, StringComparison.Ordinal);
        Assert.InRange(UpdateRelaunch.WaitSeconds, 60, 600);
    }

    [Fact]
    public void TheArgumentsCarryTheScriptHiddenAndEncoded()
    {
        string script = UpdateRelaunch.Script(Name, Family, Version);
        string arguments = UpdateRelaunch.Arguments(script);

        Assert.StartsWith("-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand ", arguments, StringComparison.Ordinal);
        string encoded = arguments.Split(' ').Last();
        Assert.Equal(script, Encoding.Unicode.GetString(Convert.FromBase64String(encoded)));
    }

    [Theory]
    [InlineData("Iqlusion.TraceCommons'; Remove-Item C:\\", Family, Version)]
    [InlineData(Name, "Iqlusion.TraceCommons_v13 bsb4pn17ym", Version)]
    [InlineData(Name, Family, "1.0.20")]
    [InlineData(Name, Family, "1.0.20.0'")]
    [InlineData("", Family, Version)]
    public void IdentityThatCouldEscapeTheScriptIsRefused(string name, string family, string version)
    {
        Assert.Throws<ArgumentException>(() => UpdateRelaunch.Script(name, family, version));
    }

    [Fact]
    public void ApplyStartsTheHelperBeforeTheHandOffAndStopsItOnARefusal()
    {
        string source = ShellSource("TraceCommons.App/AppUpdater.cs");

        int helper = source.IndexOf("StartRelaunchHelper()", StringComparison.Ordinal);
        int handOff = source.IndexOf(".RequestAddPackageByAppInstallerFileAsync(", StringComparison.Ordinal);
        Assert.True(helper >= 0, "ApplyAsync must start the relaunch helper");
        Assert.True(handOff > helper, "the helper must be running before the deployment ends the app");
        Assert.Contains("UpdateRelaunch.Script(", source, StringComparison.Ordinal);
        Assert.Contains("UpdateRelaunch.Arguments(", source, StringComparison.Ordinal);
        Assert.Contains("helper?.Kill()", source, StringComparison.Ordinal);
    }

    private static int CountOf(string haystack, string needle)
    {
        int count = 0;
        for (int at = haystack.IndexOf(needle, StringComparison.Ordinal); at >= 0; at = haystack.IndexOf(needle, at + 1, StringComparison.Ordinal))
        {
            count++;
        }

        return count;
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
