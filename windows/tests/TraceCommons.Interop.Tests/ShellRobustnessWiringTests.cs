using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// What the shell's source must keep doing so an ordinary failure is not an
/// exception into an async void handler (which ends the process), a dropped
/// click, or a window left running against a torn-down daemon. Source scans, the
/// same kind the window tests use: the code itself needs a WinUI host.
/// </summary>
public sealed class ShellRobustnessWiringTests
{
    private static string Shell(string file) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "shell-source", "TraceCommons.App", file + ".txt"));

    private static string Code(string source) =>
        string.Join('\n', source.Split('\n').Where(l => !l.TrimStart().StartsWith("//", StringComparison.Ordinal)));

    /// <summary>A click during the startup joins it instead of being answered "false".</summary>
    [Fact]
    public void TheContributionStartupJoinsACallerInsteadOfRefusingIt()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.DoesNotContain("_contributionStartupBusy", window);
        Assert.Contains("private readonly SingleFlight<bool> _contributionStartup = new();", window);
        Assert.Matches(@"private Task<bool> StartContributionsAsync\(\) => _contributionStartup\.Run\(RunContributionStartupAsync\);", window);
    }

    /// <summary>
    /// The daemon can fail to start for more than its own refusal: a state
    /// directory that cannot be made, a core library that cannot be loaded. Each
    /// start site catches them.
    /// </summary>
    [Theory]
    [InlineData("ViewModels/MainViewModel.cs", @"catch \(TcException\)\s*\{[\s\S]*?\}\s*catch \(Exception exception\)\s*\{[\s\S]*?CrashGuard\.Log\.Record\(""daemon-start"", exception\);")]
    [InlineData("ViewModels/SessionRootsViewModel.cs", @"catch \(Exception exception\)\s*\{[\s\S]*?CrashGuard\.Log\.Record\(""daemon-start"", exception\);")]
    [InlineData("FirstRunWindow.xaml.cs", @"catch \(TcException\)\s*\{\s*return false;\s*\}\s*catch \(Exception exception\)\s*\{[\s\S]*?CrashGuard\.Log\.Record\(""daemon-start"", exception\);\s*return false;")]
    public void EveryDaemonStartSiteCatchesWhatIsNotTheDaemonsRefusal(string file, string pattern)
    {
        Assert.Matches(pattern, Code(Shell(file)));
    }

    /// <summary>
    /// A shared clipboard another process holds open throws; the only place that
    /// calls it is the helper that catches.
    /// </summary>
    [Fact]
    public void NothingSetsTheClipboardExceptThroughTheCatchingHelper()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        var offenders = Directory.GetFiles(root, "*.cs.txt", SearchOption.AllDirectories)
            .Where(f => !f.EndsWith("ClipboardCopy.cs.txt", StringComparison.Ordinal))
            .Where(f => Code(File.ReadAllText(f)).Contains("Clipboard.SetContent", StringComparison.Ordinal))
            .Select(Path.GetFileName)
            .ToList();
        Assert.True(offenders.Count == 0, string.Join(", ", offenders));
        string helper = Code(Shell("ClipboardCopy.cs"));
        Assert.Matches(@"catch \(COMException exception\)", helper);
        Assert.Contains("Thread.Sleep(PauseMilliseconds)", helper);
    }

    /// <summary>Quitting closes first run and the roots window, and nothing starts on a disposed host.</summary>
    [Fact]
    public void QuittingClosesTheWindowsItOwnsAndADisposedHostStartsNothing()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.Matches(@"_aboutWindow\?\.Close\(\);\s*_onboarding\?\.Close\(\);\s*_sessionRoots\?\.Close\(\);", window);
        Assert.Matches(@"onboarding\.Closed \+= async \(_, _\) =>\s*\{\s*if \(_closed\) return;", window);
        string host = Code(Shell("DaemonHost.cs"));
        Assert.Matches(@"if \(_disposed\)\s*\{\s*throw new ObjectDisposedException\(nameof\(DaemonHost\)\);\s*\}", host);
        Assert.Matches(@"public async ValueTask DisposeAsync\(\)\s*\{\s*_disposed = true;", host);
    }

    /// <summary>
    /// Work that is not an event handler is a Task someone observes: an async
    /// void that throws ends the process, and a discarded faulted Task is lost.
    /// </summary>
    [Fact]
    public void WorkThatIsNotAHandlerReturnsATaskThatIsObserved()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.DoesNotMatch(@"async void (StartInTray|ShowInsightsPane)\b", window);
        Assert.Contains("internal async Task StartInTrayAsync()", window);
        Assert.Contains("private async Task ShowInsightsPaneAsync()", window);
        Assert.Contains("_ = Observed.Run(window.StartInTrayAsync(), \"tray-start\");", Code(Shell("App.xaml.cs")));
        Assert.Contains("_ = Observed.Run(ShowInsightsPaneAsync(), \"insights-pane\");", Code(Shell("MainWindow.Shell.cs")));
        string observed = Code(Shell("Observed.cs"));
        Assert.Matches(@"catch \(Exception exception\)\s*\{\s*CrashGuard\.Log\.Record\(where, exception\);", observed);
    }

    /// <summary>An async void handler whose body can throw has a catch around it.</summary>
    [Fact]
    public void TheRootsDeclaredHandlerCatchesWhatItsAwaitsThrow()
    {
        Assert.Matches(
            @"private async void OnSessionRootsDeclared\(\)\s*\{\s*try\s*\{[\s\S]*?await ContinueStartupAsync\(\);\s*\}\s*catch \(Exception exception\)\s*\{\s*CrashGuard\.Log\.Record\(""roots-declared"", exception\);",
            Code(Shell("MainWindow.xaml.cs")));
    }

    /// <summary>
    /// A new settings view model is built each time Settings opens, so the one
    /// from a closed Settings window stops listening to the daemon.
    /// </summary>
    [Fact]
    public void ASettingsViewModelStopsListeningWhenItsWindowCloses()
    {
        string model = Code(Shell("ViewModels/ContributorSettingsViewModel.cs"));
        Assert.Contains("public sealed class ContributorSettingsViewModel : INotifyPropertyChanged, IDisposable", model);
        Assert.Matches(@"public void Dispose\(\)\s*\{\s*if \(_disposed\)\s*\{\s*return;\s*\}\s*_disposed = true;\s*_host\.StatusChanged -= OnDaemonStatusChanged;", model);
        Assert.Matches(@"private async void OnDaemonStatusChanged\(\)\s*\{\s*if \(_disposed\)\s*\{\s*return;\s*\}", model);
        Assert.Matches(@"_settingsWindow\.Closed \+= async \(_, _\) =>\s*\{\s*settings\.Settings\.Dispose\(\);", Code(Shell("MainWindow.Shell.cs")));
    }
}
