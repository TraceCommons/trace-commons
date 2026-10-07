using System;
using System.IO;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The quit prompt is the core's (<c>quit_copy.rs</c>, exported as
/// <c>tc_quit_prompt_json</c>), chosen for the role the handle actually has.
/// </summary>
/// <remarks>
/// The window used to hand-type the hosting sentence. That sentence is the
/// true one only while this process runs the watcher; with no daemon started
/// the core says something else, and a hand-typed copy cannot follow it.
/// </remarks>
public sealed class QuitPromptTests : IDisposable
{
    private readonly string _configDir;
    private readonly string _settingsJson;

    public QuitPromptTests()
    {
        // Short for the same reason NativeRoundTripTests.ShortTempDir gives:
        // sun_path is capped at 104 bytes on macOS.
        _configDir = Path.Combine(
            Path.GetTempPath(), "tc-" + Guid.NewGuid().ToString("n").Substring(0, 8));
        Directory.CreateDirectory(_configDir);
        string claudeRoot = Path.Combine(_configDir, "claude");
        string codexRoot = Path.Combine(_configDir, "codex");
        Directory.CreateDirectory(claudeRoot);
        Directory.CreateDirectory(codexRoot);
        _settingsJson = JsonSerializer.Serialize(new
        {
            claude_root = claudeRoot,
            codex_root = codexRoot,
        });
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_configDir, recursive: true);
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    [Fact]
    public void ARunningInProcessDaemonGetsTheHostingPrompt()
    {
        using var daemon = new TcDaemon(_configDir, _settingsJson);

        QuitPrompt? prompt = daemon.QuitPrompt();

        Assert.NotNull(prompt);
        Assert.Equal("hosting", prompt!.Role);
        Assert.StartsWith("Quitting stops Trace Commons watching", prompt.Body, StringComparison.Ordinal);
        Assert.False(string.IsNullOrWhiteSpace(prompt.Title));
        Assert.False(string.IsNullOrWhiteSpace(prompt.Confirm));
        Assert.False(string.IsNullOrWhiteSpace(prompt.Cancel));
    }

    [Fact]
    public void NoDaemonGetsTheUnavailablePromptWhichClaimsNothingStops()
    {
        QuitPrompt? prompt = QuitPrompt.WithoutWatcher();

        Assert.NotNull(prompt);
        Assert.Equal("unavailable", prompt!.Role);
        Assert.DoesNotContain("Quitting stops", prompt.Body, StringComparison.Ordinal);
    }

    [Fact]
    public void AStoppedDaemonGetsTheUnavailablePrompt()
    {
        var daemon = new TcDaemon(_configDir, _settingsJson);
        daemon.Shutdown();

        QuitPrompt? prompt = daemon.QuitPrompt();

        Assert.NotNull(prompt);
        Assert.Equal("unavailable", prompt!.Role);
    }

    [Fact]
    public void TheMainWindowAsksTheCoreRatherThanTypingTheSentence()
    {
        string source = File.ReadAllText(Path.Combine(
            AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.cs.txt"));

        Assert.DoesNotContain("Quitting stops", source, StringComparison.Ordinal);
        Assert.DoesNotContain("\"Quit Trace Commons?\"", source, StringComparison.Ordinal);
        Assert.Contains("QuitPrompt()", source, StringComparison.Ordinal);
    }
}
