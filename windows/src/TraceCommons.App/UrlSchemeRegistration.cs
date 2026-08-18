using System;
using System.Diagnostics;
using Microsoft.Win32;

namespace TraceCommons.App;

/// <summary>
/// Registers <c>tracecommons://</c> so an invite clicked in mail opens this
/// app.
/// </summary>
/// <remarks>
/// <para>
/// Under HKEY_CURRENT_USER, never HKEY_LOCAL_MACHINE. This app ships
/// unpackaged and self contained, installs per user without administrator
/// rights, and a machine wide registration would both need elevation and
/// outlive the user who installed it.
/// </para>
/// <para>
/// Under MSIX the OS owns this instead: the scheme is declared as a
/// <c>windows.protocol</c> extension in
/// <c>windows/packaging/Package.appxmanifest</c>, and the runtime registry
/// write is the wrong mechanism. <see cref="EnsureRegistered"/> returns
/// early when the process has package identity, so exactly one of the two
/// paths is ever live. The class is not deleted because the unpackaged zip
/// remains the shipping artifact.
/// </para>
/// <para>
/// <b>The argv exposure.</b> A registered scheme handler receives the URL as
/// a command line argument, and command lines are readable by other
/// processes on the machine. macOS does not have this problem: it delivers
/// URL events instead. It matters here because invites are not single use,
/// <c>max_uses</c> is a <c>u32</c> on the registry entry and live invites
/// are issued in the thousands, so an invite captured from a process listing
/// stays usable. The exposure is inherent to scheme handlers and cannot be
/// removed by this class. What is done about it: the URL is never logged,
/// and pasting stays the recommended path for an invite with a large
/// <c>max_uses</c>.
/// </para>
/// </remarks>
public static class UrlSchemeRegistration
{
    private const string Scheme = "tracecommons";

    /// <summary>
    /// Registers the scheme for the current user, pointing at this
    /// executable.
    /// </summary>
    /// <remarks>
    /// Idempotent, and silent on failure. A contributor who cannot write
    /// their own HKCU is in an unusual state, but the app still works: the
    /// paste field does not depend on this, and losing a convenience is not
    /// a reason to refuse to start.
    /// </remarks>
    public static void EnsureRegistered()
    {
        // A packaged build declares the scheme in its manifest. Writing HKCU
        // as well would be at best redundant and at worst a second, divergent
        // registration -- the manifest disables registry virtualization, so
        // this write would be REAL and would point at a path inside
        // WindowsApps that the package model does not want anyone launching
        // directly.
        if (IsPackaged())
        {
            return;
        }

        try
        {
            string? executable = Environment.ProcessPath;
            if (string.IsNullOrEmpty(executable))
            {
                return;
            }

            using RegistryKey key = Registry.CurrentUser.CreateSubKey(
                $@"Software\Classes\{Scheme}");

            key.SetValue(string.Empty, "URL:Trace Commons Invite");

            // The marker that tells the shell this key is a protocol handler.
            // Its presence is what matters; the value is empty by convention.
            key.SetValue("URL Protocol", string.Empty);

            using RegistryKey command = key.CreateSubKey(@"shell\open\command");

            // %1 quoted: an invite URL is a single argument and must survive
            // a path or a query containing spaces.
            command.SetValue(string.Empty, $"\"{executable}\" \"%1\"");
        }
        catch (Exception e) when (e is UnauthorizedAccessException or System.Security.SecurityException)
        {
            Debug.WriteLine("tracecommons scheme registration skipped: no HKCU write access");
        }
    }

    /// <summary>
    /// Whether this process is running with MSIX package identity.
    /// </summary>
    /// <remarks>
    /// P/Invoke rather than <c>Windows.ApplicationModel.Package.Current</c>:
    /// that property THROWS when there is no package, so using it here would
    /// mean an exception on every startup of the build we actually ship.
    /// <c>GetCurrentPackageFullName</c> returns APPMODEL_ERROR_NO_PACKAGE
    /// (15700) instead, which is an answer rather than an incident.
    /// </remarks>
    private static bool IsPackaged()
    {
        const int AppModelErrorNoPackage = 15700;

        int length = 0;
        int result = GetCurrentPackageFullName(ref length, null);
        return result != AppModelErrorNoPackage;
    }

    [System.Runtime.InteropServices.DllImport(
        "kernel32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetCurrentPackageFullName(ref int packageFullNameLength, char[]? packageFullName);
}
