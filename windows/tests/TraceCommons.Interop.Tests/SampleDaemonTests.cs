using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The recorded samples, read in place of a daemon.
/// </summary>
/// <remarks>
/// <para>
/// The Windows half of the macOS app's <c>SampleDaemonClient</c> and of the
/// Linux shell's <c>sample.rs</c> (#2161): slice W1 of
/// <c>docs/superpowers/specs/2026-11-18-windows-mac-parity-design.md</c>.
/// </para>
/// <para>
/// The client is compiled out of a Release <c>TraceCommons.Interop</c>. CI
/// runs this suite in Release, so in that configuration the test assembly
/// compiles its own copy of the source (see the test project) and these
/// tests still run there; <see cref="TheClientIsNotInAReleaseBuild"/> checks
/// the shipped assembly itself in both configurations.
/// </para>
/// </remarks>
public class SampleDaemonTests
{
    private static readonly string Root = Path.Combine(AppContext.BaseDirectory, "RecordedSamples");

    private static JsonElement Recorded(string set, string method)
    {
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(Path.Combine(Root, set, method + ".json")));
        return doc.RootElement.Clone();
    }

    private static string Compact(JsonElement element) => JsonSerializer.Serialize(element);

    private static DaemonResponse Answer(SampleDaemon sample, string method) =>
        DaemonResponse.Parse(sample.Call(method));

    [Fact]
    public void TheChoiceFollowsTheMacApp()
    {
        Assert.Null(SampleDaemon.Choose(null));
        Assert.Equal(new SampleChoice("normalDay", false), SampleDaemon.Choose(""));
        foreach (string set in SampleDaemon.Sets)
        {
            Assert.Equal(new SampleChoice(set, false), SampleDaemon.Choose(set));
        }

        // Matched exactly, as Swift's rawValue is.
        Assert.Equal(new SampleChoice("normalDay", true), SampleDaemon.Choose("NormalDay"));
        Assert.Equal(new SampleChoice("normalDay", true), SampleDaemon.Choose("busy"));
    }

    [Fact]
    public void TheSetsAreTheMacAppsInItsOrder()
    {
        Assert.Equal(
            new[] { "empty", "normalDay", "busyQueue", "coreDown", "unknownCounts", "heldSessions", "armedFolder" },
            SampleDaemon.Sets);
        foreach (string set in SampleDaemon.Sets)
        {
            Assert.Equal(set != SampleDaemon.CoreDown, Directory.Exists(Path.Combine(Root, set)));
        }

        Assert.True(Directory.Exists(Path.Combine(Root, SampleDaemon.SharedDir)));
    }

    [Fact]
    public void UnsetIsTheDaemonAndLogsNothing()
    {
        var log = new List<string>();
        Assert.Null(SampleDaemon.FromValue(null, Root, log.Add));
        Assert.Empty(log);
    }

    [Fact]
    public void EmptyIsTheDefaultSetAndLogsNothing()
    {
        var log = new List<string>();
        SampleDaemon? sample = SampleDaemon.FromValue("", Root, log.Add);
        Assert.NotNull(sample);
        Assert.Equal("normalDay", sample!.Set);
        Assert.Empty(log);
    }

    [Fact]
    public void AnUnknownNameIsTheDefaultSetAndOneLogLineWithoutTheValue()
    {
        var log = new List<string>();
        SampleDaemon? sample = SampleDaemon.FromValue("my-secret-name", Root, log.Add);
        Assert.NotNull(sample);
        Assert.Equal("normalDay", sample!.Set);
        string line = Assert.Single(log);
        Assert.DoesNotContain("my-secret-name", line, StringComparison.Ordinal);
        Assert.Contains("fallback=normalDay", line, StringComparison.Ordinal);
    }

    [Fact]
    public void AReadAnswersWithTheRecordingForItsSet()
    {
        foreach (string set in new[] { "normalDay", "heldSessions", "empty" })
        {
            var sample = new SampleDaemon(set, Root);
            foreach (string method in new[] { "status", "list_pending", "list_history", "get_settings", "list_projects" })
            {
                DaemonResponse response = Answer(sample, method);
                Assert.False(response.IsError, $"{set}/{method}");
                Assert.Equal(Compact(Recorded(set, method)), Compact(response.Result!.Value));
            }
        }
    }

    [Fact]
    public void ARecordedStatusDecodesAsTheAppReadsIt()
    {
        DaemonResponse response = Answer(new SampleDaemon("normalDay", Root), DaemonProtocol.Methods.Status);
        Assert.NotNull(response.ResultAs<DaemonStatus>());
    }

    [Fact]
    public void ASetIndependentWriteAnswersFromShared()
    {
        var sample = new SampleDaemon("busyQueue", Root);
        foreach (string method in new[] { "approve", "keep", "undo_keep", "set_project_mode" })
        {
            DaemonResponse response = Answer(sample, method);
            Assert.False(response.IsError, method);
            Assert.Equal(Compact(Recorded(SampleDaemon.SharedDir, method)), Compact(response.Result!.Value));
        }
    }

    [Fact]
    public void EveryOtherMethodIsRefusedWithOneFixedLabel()
    {
        var sample = new SampleDaemon("normalDay", Root);
        foreach (string method in new[]
        {
            "set_settings", "enroll", "withdraw", "unenroll", "preview", "preview_body",
            "search_original", "account_devices_list", "",
            "../normalDay/status", "..\\normalDay\\status", "status.json", "Status", "status ",
        })
        {
            DaemonResponse response = Answer(sample, method);
            Assert.True(response.IsError, method);
            Assert.Equal(SampleDaemon.UnrecordedLabel, response.Error!.Message);
            Assert.Null(response.Result);
        }
    }

    [Fact]
    public void CoreDownFailsEveryCallAsAMissingDaemon()
    {
        var sample = new SampleDaemon(SampleDaemon.CoreDown, Root);
        foreach (string method in new[] { "status", "list_pending", "approve", "enroll" })
        {
            DaemonResponse response = Answer(sample, method);
            Assert.True(response.IsError, method);
            Assert.Equal("unavailable", response.Error!.Code);
            Assert.Equal(SampleDaemon.NotRunningLabel, response.Error.Message);
        }
    }

    /// <summary>
    /// Sample mode never opens the state directory, starts or attaches a
    /// daemon, or takes the lock.
    /// </summary>
    /// <remarks>
    /// Shown two ways. Its source names no ABI entry point, no file write and
    /// no state directory, so it cannot start or attach a daemon or take the
    /// lock. And the throwaway state directory it hands the host is not
    /// created by building the client or by any call it answers.
    /// </remarks>
    [Fact]
    public void TheSampleNeverStartsADaemonOrTouchesTheStateDirectory()
    {
        string source = ShellSource("TraceCommons.Interop/SampleDaemon.cs");
        foreach (string forbidden in new[]
        {
            "NativeMethods", "TcDaemon", "tc_", "DllImport", "Directory.Create", "File.Open",
            "File.Write", "File.Create", "FileStream", "DefaultConfigDir", "LocalApplicationData",
        })
        {
            Assert.DoesNotContain(forbidden, Code(source), StringComparison.Ordinal);
        }

        string throwaway = SampleDaemon.ThrowawayConfigDir();
        Assert.StartsWith(Path.GetTempPath(), throwaway, StringComparison.Ordinal);
        Assert.False(Directory.Exists(throwaway), throwaway);
        SampleDaemon sample = SampleDaemon.FromValue("armedFolder", Root, _ => { })!;
        foreach (string method in new[] { "status", "list_pending", "approve", "enroll", "set_settings" })
        {
            _ = sample.Call(method);
        }

        Assert.False(Directory.Exists(throwaway), throwaway);
    }

    /// <summary>
    /// The app chooses the sample before anything else is touched, and only
    /// in a debug build.
    /// </summary>
    /// <remarks>
    /// <c>DaemonHost</c> is WinUI and cannot be built here, so the order is
    /// asserted about its source: the sample branch returns before the state
    /// directory is created and before a daemon is started, every mention
    /// of the sample sits inside <c>#if DEBUG</c>, and the host's state
    /// directory is the throwaway one in sample mode.
    /// </remarks>
    [Fact]
    public void TheHostChoosesTheSampleFirstAndOnlyInDebug()
    {
        string host = ShellSource("TraceCommons.App/DaemonHost.cs");
        Assert.Contains("SampleDaemon.FromEnvironment()", host, StringComparison.Ordinal);
        Assert.Contains("SampleDaemon.ThrowawayConfigDir()", host, StringComparison.Ordinal);

        string start = Between(host, "public async Task StartAsync(", "public async Task<DaemonResponse> CallAsync(");
        int sampleReturn = start.IndexOf("if (_sample is not null)", StringComparison.Ordinal);
        Assert.True(sampleReturn >= 0, "StartAsync has no sample branch");
        Assert.True(sampleReturn < start.IndexOf("Directory.CreateDirectory(_configDir)", StringComparison.Ordinal));
        Assert.True(sampleReturn < start.IndexOf("new TcDaemon(", StringComparison.Ordinal));

        string call = Between(host, "public async Task<DaemonResponse> CallAsync(", "public async Task<IReadOnlyList<QueueEntry>> ListPendingAsync(");
        int sampleCall = call.IndexOf("_sample is SampleDaemon", StringComparison.Ordinal);
        Assert.True(sampleCall >= 0, "CallAsync has no sample branch");
        Assert.True(sampleCall < call.IndexOf("TcDaemon? daemon = _daemon;", StringComparison.Ordinal));

        foreach (string file in new[] { "TraceCommons.App/DaemonHost.cs", "TraceCommons.App/MainWindow.xaml.cs" })
        {
            string text = ShellSource(file);
            AssertOnlyInsideDebug(file, text, new[] { "_sample", "IsSample", "SampleDaemon" });
        }

        // Program.IsSampleLaunch is a constant false outside Debug (see
        // ASampleLaunchWritesNothingOutsideItsWindow); only its Debug body
        // names the client.
        AssertOnlyInsideDebug("TraceCommons.App/Program.cs", ShellSource("TraceCommons.App/Program.cs"), new[] { "SampleDaemon" });

        // Nothing else in either shell project names the sample client.
        foreach (string path in Directory.EnumerateFiles(ShellSourceRoot, "*.txt", SearchOption.AllDirectories))
        {
            string rel = Path.GetRelativePath(ShellSourceRoot, path).Replace('\\', '/');
            if (rel is "TraceCommons.Interop/SampleDaemon.cs.txt"
                or "TraceCommons.App/DaemonHost.cs.txt"
                or "TraceCommons.App/MainWindow.xaml.cs.txt"
                or "TraceCommons.App/Program.cs.txt")
            {
                continue;
            }

            Assert.DoesNotContain("SampleDaemon", File.ReadAllText(path), StringComparison.Ordinal);
        }
    }

    [Fact]
    public void ASampleIsRequestedExactlyWhenTheVariableIsSet()
    {
        Assert.False(SampleDaemon.IsRequested((string?)null));
        foreach (string value in new[] { "", "normalDay", "coreDown", "NormalDay", "anything" })
        {
            Assert.True(SampleDaemon.IsRequested(value), value);
            Assert.NotNull(SampleDaemon.FromValue(value, Root, _ => { }));
        }
    }

    /// <summary>
    /// A sample launch changes nothing on this machine outside its window:
    /// it never claims the single-instance key (so it never redirects into a
    /// running app), never points the URL scheme at itself, never offers or
    /// writes run at login, and never writes the witness files.
    /// </summary>
    /// <remarks>
    /// These sites are in the WinUI project, which cannot be built or run
    /// here, so this checks the wiring in the source: each write is reached
    /// only past a <c>Program.IsSampleLaunch</c> check, and that check is a
    /// constant false outside Debug. The behaviour is checked on the Windows
    /// test box by <c>win-capture-samples.ps1</c>, which snapshots the URL
    /// scheme, the Run value and the state directories around every sample
    /// launch and fails on any change, and launches one set twice to see two
    /// windows rather than a redirect.
    /// </remarks>
    [Fact]
    public void ASampleLaunchWritesNothingOutsideItsWindow()
    {
        string program = ShellSource("TraceCommons.App/Program.cs").Replace("\r\n", "\n", StringComparison.Ordinal);
        string flag = Between(program, "internal static bool IsSampleLaunch =>", ";\n#endif");
        Assert.Contains("#if DEBUG", flag, StringComparison.Ordinal);
        Assert.Contains("SampleDaemon.IsRequested();", flag, StringComparison.Ordinal);
        Assert.EndsWith("#else\n        false", flag.TrimEnd(), StringComparison.Ordinal);

        AssertGuarded(program, "private static int Main(", "AppInstance.FindOrRegisterForKey(", "if (!IsSampleLaunch)");
        AssertGuarded(
            ShellSource("TraceCommons.App/App.xaml.cs"),
            "protected override void OnLaunched(", "UrlSchemeRegistration.EnsureRegistered();", "if (!Program.IsSampleLaunch)");

        string startup = ShellSource("TraceCommons.App/StartupRegistration.cs");
        AssertGuarded(startup, "GetStateAsync()", "RunAtLogin.IsEnabled", "if (Program.IsSampleLaunch)");
        AssertGuarded(startup, "GetStateAsync()", "StartupTask.GetAsync(", "if (Program.IsSampleLaunch)");
        AssertGuarded(startup, "SetEnabledAsync(bool enabled)", "RunAtLogin.Set(", "if (Program.IsSampleLaunch)");
        AssertGuarded(startup, "SetEnabledAsync(bool enabled)", "StartupTask.GetAsync(", "if (Program.IsSampleLaunch)");

        string settings = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        AssertGuarded(settings, "public async Task ConfigureWitnessAsync()", "WitnessSurface.Configure(", "|| Program.IsSampleLaunch)");
        AssertGuarded(settings, "public async Task ClearWitnessAsync()", "WitnessSurface.Clear(", "|| Program.IsSampleLaunch)");

        // No other caller reaches those writes around the guards.
        foreach (string path in Directory.EnumerateFiles(Path.Combine(ShellSourceRoot, "TraceCommons.App"), "*.txt", SearchOption.AllDirectories))
        {
            string rel = Path.GetRelativePath(ShellSourceRoot, path).Replace('\\', '/');
            string text = Code(File.ReadAllText(path));
            foreach ((string write, string owner) in new[]
            {
                ("FindOrRegisterForKey(", "TraceCommons.App/Program.cs.txt"),
                ("UrlSchemeRegistration.EnsureRegistered(", "TraceCommons.App/App.xaml.cs.txt"),
                ("RunAtLogin.Set(", "TraceCommons.App/StartupRegistration.cs.txt"),
                ("WitnessSurface.Configure(", "TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs.txt"),
                ("WitnessSurface.Clear(", "TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs.txt"),
            })
            {
                if (rel != owner)
                {
                    Assert.DoesNotContain(write, text, StringComparison.Ordinal);
                }
            }
        }
    }

    /// <summary>
    /// <paramref name="guard"/> appears in the member starting at
    /// <paramref name="member"/>, before <paramref name="write"/>.
    /// </summary>
    private static void AssertGuarded(string source, string member, string write, string guard)
    {
        string code = Code(source);
        int start = code.IndexOf(member, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{member} not found");
        int writeAt = code.IndexOf(write, start, StringComparison.Ordinal);
        Assert.True(writeAt > start, $"{member} no longer reaches {write}");
        int guardAt = code.IndexOf(guard, start, StringComparison.Ordinal);
        Assert.True(guardAt > start && guardAt < writeAt, $"{member} reaches {write} without {guard}");
    }

    /// <summary>
    /// The sample client is not in a Release <c>TraceCommons.Interop</c>,
    /// and is in a Debug one.
    /// </summary>
    [Fact]
    public void TheClientIsNotInAReleaseBuild()
    {
        Type? shipped = typeof(TcDaemon).Assembly.GetType("TraceCommons.Interop.SampleDaemon");
#if DEBUG
        Assert.NotNull(shipped);
#else
        Assert.Null(shipped);
#endif

        string source = ShellSource("TraceCommons.Interop/SampleDaemon.cs");
        string[] directives = source.Split('\n')
            .Select(line => line.Trim())
            .Where(line => line.StartsWith("#", StringComparison.Ordinal))
            .ToArray();
        Assert.Equal(new[] { "#if DEBUG || TC_SAMPLE_TESTS", "#endif" }, directives);
        Assert.StartsWith("#if DEBUG || TC_SAMPLE_TESTS", Code(source).TrimStart(), StringComparison.Ordinal);
        Assert.EndsWith("#endif", source.TrimEnd(), StringComparison.Ordinal);

        // The recordings are copied next to a Debug build only.
        string project = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "interop-project", "TraceCommons.Interop.csproj.txt"));
        Assert.Matches(
            new Regex(@"<ItemGroup Condition=""'\$\(Configuration\)' == 'Debug'"">\s*<None Include=""[^""]*RecordedSamples/\*\*/\*\.json"""),
            project);
    }

    /// <summary>
    /// The Windows capture driver names its files with the gallery's grammar,
    /// for the Mac app's sets, so Mac and Windows captures pair.
    /// </summary>
    [Fact]
    public void CaptureNamesFollowTheGalleryGrammar()
    {
        string gallery = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "parity", "gallery.py.txt"));
        Match pattern = Regex.Match(gallery, @"NAME = re\.compile\(\s*r""([^""]*)""\s*r""([^""]*)""\s*\)");
        Assert.True(pattern.Success, "gallery.py's NAME pattern moved");
        var name = new Regex(pattern.Groups[1].Value.Replace("(?P<", "(?<", StringComparison.Ordinal)
            + pattern.Groups[2].Value.Replace("(?P<", "(?<", StringComparison.Ordinal));

        string driver = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "parity", "win-capture-samples.ps1.txt"));
        string[] sets = PowerShellList(driver, "Sets");
        string[] tabs = PowerShellList(driver, "Tabs");
        string[] panes = PowerShellList(driver, "Panes");
        string[] appearances = PowerShellList(driver, "Appearances");
        Assert.Equal(SampleDaemon.Sets, sets);
        Assert.Equal(new[] { "home", "inference", "traces", "insights" }, tabs);
        Assert.Equal(new[] { "main", "main+map", "main+inspector", "main+map+inspector" }, panes);
        Assert.Equal(new[] { "light", "dark" }, appearances);
        Assert.Contains("\"$Set-$Tab-$Panes-$Appearance.png\"", driver, StringComparison.Ordinal);

        foreach (string set in sets)
        foreach (string tab in tabs)
        foreach (string pane in panes)
        foreach (string appearance in appearances)
        {
            string file = $"{set}-{tab}-{pane}-{appearance}.png";
            Match parsed = name.Match(file);
            Assert.True(parsed.Success, file);
            Assert.Equal(set, parsed.Groups["set"].Value);
            Assert.Equal(tab, parsed.Groups["tab"].Value);
            Assert.Equal(pane, parsed.Groups["panes"].Value);
            Assert.Equal(appearance, parsed.Groups["appearance"].Value);
        }

        // The view the window opens on is one of the grammar's.
        Assert.Contains(PowerShellScalar(driver, "OpeningTab"), tabs);
        Assert.Contains(PowerShellScalar(driver, "OpeningPanes"), panes);
    }

    private static string[] PowerShellList(string script, string variable)
    {
        Match match = Regex.Match(script, @"(?m)^\$" + variable + @"\s*=\s*@\(([^)]*)\)");
        Assert.True(match.Success, $"${variable} = @(...) not found");
        return Regex.Matches(match.Groups[1].Value, "'([^']*)'").Select(m => m.Groups[1].Value).ToArray();
    }

    private static string PowerShellScalar(string script, string variable)
    {
        Match match = Regex.Match(script, @"(?m)^\$" + variable + @"\s*=\s*'([^']*)'");
        Assert.True(match.Success, $"${variable} = '...' not found");
        return match.Groups[1].Value;
    }

    private static void AssertOnlyInsideDebug(string file, string text, string[] needles)
    {
        int depth = 0;
        bool inDebug = false;
        int line = 0;
        foreach (string raw in text.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n'))
        {
            line++;
            string trimmed = raw.Trim();
            if (trimmed.StartsWith("#if DEBUG", StringComparison.Ordinal))
            {
                Assert.False(inDebug, $"{file}:{line}: nested #if DEBUG");
                inDebug = true;
                depth = 0;
                continue;
            }

            if (inDebug && trimmed.StartsWith("#if", StringComparison.Ordinal))
            {
                depth++;
                continue;
            }

            if (inDebug && trimmed.StartsWith("#else", StringComparison.Ordinal) && depth == 0)
            {
                inDebug = false;
                continue;
            }

            if (inDebug && trimmed.StartsWith("#endif", StringComparison.Ordinal))
            {
                if (depth == 0)
                {
                    inDebug = false;
                }
                else
                {
                    depth--;
                }

                continue;
            }

            if (inDebug || trimmed.StartsWith("//", StringComparison.Ordinal) || trimmed.StartsWith("///", StringComparison.Ordinal))
            {
                continue;
            }

            foreach (string needle in needles)
            {
                Assert.False(
                    trimmed.Contains(needle, StringComparison.Ordinal),
                    $"{file}:{line}: {needle} outside #if DEBUG: {trimmed}");
            }
        }

        Assert.False(inDebug, $"{file}: unterminated #if DEBUG");
    }

    private static string Between(string text, string from, string to)
    {
        int start = text.IndexOf(from, StringComparison.Ordinal);
        Assert.True(start >= 0, from);
        int end = text.IndexOf(to, start, StringComparison.Ordinal);
        Assert.True(end > start, to);
        return text[start..end];
    }

    /// <summary>The source with comment lines removed.</summary>
    private static string Code(string source) => string.Join(
        "\n",
        source.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n')
            .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

    private static string ShellSourceRoot => Path.Combine(AppContext.BaseDirectory, "shell-source");

    private static string ShellSource(string relativePath) =>
        File.ReadAllText(Path.Combine(ShellSourceRoot, relativePath + ".txt"));
}
