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
    // Line endings normalized: a CI checkout can be CRLF where a clone is LF, and
    // these scans match on text, not on the platform's newline.
    private static string Shell(string file) => File.ReadAllText(Path.Combine(
        AppContext.BaseDirectory, "shell-source", "TraceCommons.App", file + ".txt")).Replace("\r\n", "\n");

    /// <summary>The text from <paramref name="start"/> up to <paramref name="end"/>, failing with the missing marker named.</summary>
    private static string Between(string source, string start, string end)
    {
        int from = source.IndexOf(start, StringComparison.Ordinal);
        Assert.True(from >= 0, "start marker not found: " + start);
        int to = source.IndexOf(end, from, StringComparison.Ordinal);
        Assert.True(to >= 0, "end marker not found after the start: " + end);
        return source[from..to];
    }

    private static string Code(string source) =>
        string.Join('\n', source.Split('\n').Where(l => !l.TrimStart().StartsWith("//", StringComparison.Ordinal)));

    /// <summary>A click during the startup joins it instead of being answered "false".</summary>
    [Fact]
    public void TheContributionStartupJoinsACallerInsteadOfRefusingIt()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.DoesNotContain("_contributionStartupBusy", window);
        Assert.Contains("private readonly SingleFlight<bool> _contributionStartup = new();", window);
        Assert.Matches(@"private Task<bool> StartContributionsCoreAsync\(\) => _contributionStartup\.Run\(RunContributionStartupAsync\);", window);
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

    /// <summary>
    /// The update check is a network call; the startup a click can join does not
    /// wait for it, and a failure of it is logged, not thrown into the UI.
    /// </summary>
    [Fact]
    public void TheUpdateCheckRunsAfterTheStartupAndIsNotWaitedFor()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        string body = Between(window, "private async Task ContinueStartupAsync()", "private async Task ShowSessionRootsAsync()");
        Assert.DoesNotContain("await ViewModel.CheckForUpdateAsync()", body);
        Assert.Matches(@"OfferNextRedirectedInvite\(\);\s*_ = Observed\.Run\(ViewModel\.CheckForUpdateAsync\(\), ""update-check""\);\s*\}", body);
    }

    /// <summary>
    /// The automatic start at first launch never navigates: a failed one moved
    /// the person off Insights onto a blank Traces pane. A click's start still
    /// shows Queue, whose header says why.
    /// </summary>
    [Fact]
    public void OnlyAClicksStartNavigatesToQueueOnFailure()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        string auto = Between(window, "private async Task OpenFirstRunOnFirstLaunchAsync", "private bool _contributionStartupAttempted");
        Assert.Contains("await StartContributionsCoreAsync();", auto);
        Assert.DoesNotContain("StartContributionsAsync", auto);
        // The shared body does not navigate; the clicked wrapper does, once.
        string body = Between(window, "private async Task<bool> RunContributionStartupAsync", "private async Task ContinueStartupAsync");
        Assert.DoesNotContain("ShowQueue", body);
        Assert.Matches(@"private async Task<bool> StartContributionsAsync\(\)\s*\{\s*bool started = await StartContributionsCoreAsync\(\);\s*if \(!started && !ViewModel\.NeedsSessionRoots && !_closed\)\s*\{\s*ViewModel\.ShowQueue\(\);", window);
    }

    /// <summary>CoalescingRefresh rethrows a failed refresh; the daemon-event handlers catch it.</summary>
    [Theory]
    [InlineData("OnQueueChanged", "queue-changed")]
    [InlineData("OnStatusChanged", "status-changed")]
    public void TheDaemonEventHandlersCatchARefreshThatFailed(string handler, string label)
    {
        Assert.Matches(
            @"private async void " + handler + @"\(\)\s*\{\s*(try\s*\{\s*await RefreshAsync\(\)\.ConfigureAwait\(true\);\s*\}\s*catch \(Exception exception\)\s*\{\s*CrashGuard\.Log\.Record\(""" + label + @""", exception\);\s*\})",
            Code(Shell("ViewModels/MainViewModel.cs")));
    }

    /// <summary>A quit queued from the tray can run after the window closed.</summary>
    [Fact]
    public void ATrayQuitAfterTheWindowClosedDoesNothing()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.Matches(@"private async Task ConfirmQuitAsync\(\)\s*\{\s*if \(_quitDialogOpen \|\| _closed\)\s*\{\s*return;\s*\}", window);
        Assert.Matches(@"if \(OpenRoot is not XamlRoot quitRoot\)\s*\{\s*return;\s*\}\s*BringForward\(\);\s*var dialog = new Microsoft\.UI\.Xaml\.Controls\.ContentDialog\s*\{\s*XamlRoot = quitRoot,", window);
        Assert.Matches(@"_host\.Dispatcher\.TryEnqueue\(async \(\) =>\s*\{\s*try\s*\{\s*await ConfirmQuitAsync\(\);\s*\}\s*catch \(Exception exception\)\s*\{\s*CrashGuard\.Log\.Record\(""tray-quit"", exception\);", window);
    }

    /// <summary>A History view made anywhere is the wired one (rows open).</summary>
    [Fact]
    public void EveryHistoryViewIsTheWiredOne()
    {
        string window = Code(Shell("MainWindow.xaml.cs"));
        Assert.DoesNotContain("new HistoryView(", window);
        Assert.Equal(2, System.Text.RegularExpressions.Regex.Matches(window, @"HistoryPane\.Content \?\?= NewHistoryView\(\);").Count);
    }

    /// <summary>
    /// First run's own entry points survive a failed render themselves; they do
    /// not depend on the app-level handler, whose exception details XAML does
    /// not guarantee. The handlers are installed before InitializeComponent.
    /// </summary>
    [Fact]
    public void FirstRunRendersWithoutThrowingAndTheHandlersComeFirst()
    {
        string run = Code(Shell("FirstRunWindow.xaml.cs"));
        Assert.Matches(@"private void Render\(\)\s*\{\s*try\s*\{\s*RenderCore\(\);\s*\}\s*catch \(Exception exception\)\s*\{\s*CrashGuard\.Log\.Record\(""first-run-render"", exception\);\s*\}\s*\}", run);
        Assert.Matches(@"CrashGuard\.Install\(this\);\s*InitializeComponent\(\);", Code(Shell("App.xaml.cs")));
    }
}
