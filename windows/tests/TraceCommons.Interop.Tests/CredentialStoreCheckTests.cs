using System;
using System.Collections.Generic;
using System.IO;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W25-D2 (a): the Windows app answers the release pipeline's credential-store
/// check the way the Mac app does (<c>macos/Sources/TraceCommonsApp/SelfTest.swift:52-62</c>)
/// and the Tauri app does (<c>tauri-desktop/src-tauri/src/credential_store_check.rs</c>):
/// <c>TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT</c> names a file, the app
/// writes one line to it and exits before any window, daemon or
/// single-instance key.
/// </summary>
public class CredentialStoreCheckTests
{
    [Fact]
    public void TheVariableIsTheOneTheMacAndTauriAppsRead()
    {
        Assert.Equal("TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT", CredentialStoreCheck.Variable);
    }

    [Fact]
    public void ReportsMatchTheOtherShellsAndTheVerifier()
    {
        // The verifier passes only on a line starting `reachable`.
        Assert.Equal("reachable\n", CredentialStoreCheck.Report(0));
        Assert.Equal("unreachable code=1\n", CredentialStoreCheck.Report(1));
        Assert.Equal("unreachable code=2\n", CredentialStoreCheck.Report(2));
        Assert.False(CredentialStoreCheck.Report(2).StartsWith("reachable", StringComparison.Ordinal));
    }

    [Fact]
    public void AnUnsetOrEmptyVariableRunsNothing()
    {
        int checks = 0;
        var writes = new List<string>();
        Assert.False(CredentialStoreCheck.RunIfRequested(null, () => { checks++; return 0; }, (p, _) => writes.Add(p)));
        Assert.False(CredentialStoreCheck.RunIfRequested("", () => { checks++; return 0; }, (p, _) => writes.Add(p)));
        Assert.Equal(0, checks);
        Assert.Empty(writes);
    }

    [Fact]
    public void ARequestedCheckWritesItsLineToTheNamedFile()
    {
        string dir = Directory.CreateTempSubdirectory("tc-credcheck-").FullName;
        try
        {
            string path = Path.Combine(dir, "out");
            Assert.True(CredentialStoreCheck.RunIfRequested(path, () => 0));
            Assert.Equal("reachable\n", File.ReadAllText(path));

            Assert.True(CredentialStoreCheck.RunIfRequested(path, () => 2));
            Assert.Equal("unreachable code=2\n", File.ReadAllText(path));
        }
        finally
        {
            Directory.Delete(dir, recursive: true);
        }
    }

    [Fact]
    public void AFailedWriteStillEndsTheLaunch()
    {
        // The verifier reports an empty or missing file as a failure; the app
        // must still exit rather than go on to open its window.
        string path = Path.Combine(Path.GetTempPath(), "tc-credcheck-" + Guid.NewGuid().ToString("N"), "missing", "out");
        Assert.True(CredentialStoreCheck.RunIfRequested(path, () => 0));
        Assert.False(File.Exists(path));
    }

    [Fact]
    public void TheAppAnswersTheCheckBeforeAnythingElseInMain()
    {
        string program = ShellSource("TraceCommons.App/Program.cs");
        int check = program.IndexOf("CredentialStoreCheck.RunIfRequested(", StringComparison.Ordinal);
        Assert.True(check >= 0, "Program.Main does not answer the credential-store check");
        // Before COM wrappers, the activation subscription, the single-instance
        // key and Application.Start: the check opens no window, starts no
        // daemon and is never redirected to a running instance.
        foreach (string later in new[]
        {
            "InitializeComWrappers()", "AppInstance.GetCurrent()", "FindOrRegisterForKey(", "Application.Start(",
        })
        {
            int at = program.IndexOf(later, StringComparison.Ordinal);
            Assert.True(at > check, $"{later} runs before the credential-store check");
        }
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
