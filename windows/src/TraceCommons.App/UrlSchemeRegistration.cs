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
/// MSIX would declare this in a manifest instead and the OS would own it.
/// When packaging happens this class should go away rather than be kept in
/// parallel.
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
}
