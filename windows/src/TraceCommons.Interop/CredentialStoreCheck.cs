using System;
using System.IO;

namespace TraceCommons.Interop;

/// <summary>
/// The release pipeline's credential-store check, answered by the Windows app
/// (spec W25-D2 (a)).
/// </summary>
/// <remarks>
/// <para>
/// <c>scripts/windows/verify-credential-store.ps1</c> launches the built app
/// with <c>TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT</c> set and waits for the
/// file it names. The app answers through <c>tc_credential_store_self_check</c>,
/// the same FFI call the Mac app makes
/// (<c>macos/Sources/TraceCommonsApp/SelfTest.swift:52-62</c>), and writes the
/// same lines as the Mac and Tauri apps: <c>reachable</c> or
/// <c>unreachable code=N</c>. On Windows the store is the Credential Manager,
/// reached from the process that hosts the daemon, which is this app.
/// </para>
/// <para>
/// <c>Program.Main</c> runs this before anything else, so the check opens no
/// window, starts no daemon, takes no single-instance key and is never
/// redirected to a running instance. It reads a reference that was never
/// stored, writes nothing to the store, and the report carries only the code.
/// </para>
/// </remarks>
public static class CredentialStoreCheck
{
    /// <summary>The variable naming the report file, as on the Mac and in Tauri.</summary>
    public const string Variable = "TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT";

    /// <summary>The report line for a self-check code; the verifier passes only on <c>reachable</c>.</summary>
    public static string Report(int code) => code == 0 ? "reachable\n" : $"unreachable code={code}\n";

    /// <summary>
    /// If <paramref name="requestedPath"/> (the variable's value) names a
    /// file, run the self-check, write its report there and return true; the
    /// caller then exits without starting the app.
    /// </summary>
    public static bool RunIfRequested(string? requestedPath) =>
        RunIfRequested(requestedPath, NativeMethods.tc_credential_store_self_check);

    internal static bool RunIfRequested(string? requestedPath, Func<int> check) =>
        RunIfRequested(requestedPath, check, WriteReport);

    internal static bool RunIfRequested(string? requestedPath, Func<int> check, Action<string, string> write)
    {
        if (string.IsNullOrEmpty(requestedPath)) return false;
        write(requestedPath, Report(check()));
        return true;
    }

    private static void WriteReport(string path, string report)
    {
        // A failed write leaves the file missing or empty, which the verifier
        // reports as a failure; there is nothing more useful to do here.
        try { File.WriteAllText(path, report); }
        catch (Exception) { }
    }
}
