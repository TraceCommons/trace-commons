using System;
using System.Diagnostics;
using Microsoft.Win32;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// Run at login, per user, with no administrator rights.
/// </summary>
/// <remarks>
/// <para>
/// The Windows counterpart of <c>macos/LoginItemManager.swift</c> (which uses
/// <c>SMAppService</c>) and of the GTK shell's <c>autostart.rs</c> (which
/// writes an XDG <c>.desktop</c> entry). All three exist for the same reason:
/// this is a background watcher, and a watcher that only runs when someone
/// remembers to start it is not watching.
/// </para>
/// <para>
/// <b>Why HKCU\...\Run and nothing else.</b> The app ships unpackaged
/// (<c>WindowsPackageType=None</c>), self-contained, and installs per user.
/// Every other run-at-login mechanism on Windows costs elevation or a
/// package: a Scheduled Task with a logon trigger needs administrator rights
/// for anything but the plainest task and puts the app in a list contributors
/// do not think to audit; a service is wrong for something with a UI and
/// needs an installer running as SYSTEM; the MSIX
/// <c>windows.startupTask</c> extension needs a package, and the packaged
/// flavour is opt-in and is not what ships. HKCU\Run needs none of that --
/// it is the current user's own hive, writable by that user without a
/// prompt -- and it
/// surfaces in Task Manager's Startup tab and in Settings -> Apps -> Startup,
/// which are exactly where someone goes to see what starts with their
/// machine. Anything that runs at login without appearing there reads as
/// malware, correctly. This follows <see cref="UrlSchemeRegistration"/>,
/// which reaches for the same hive for the same reasons.
/// </para>
/// <para>
/// <b>Windows can veto it.</b> Settings -> Apps -> Startup lets a
/// contributor disable a startup entry without deleting it; Windows records
/// that in a separate <c>StartupApproved</c> key and simply does not run the
/// command. This class does not read or write that key, and must not: it is
/// the user's override of the app's request, and an app that reverses it is
/// doing the thing that makes startup entries distrusted in the first place.
/// The consequence is that <see cref="IsEnabled"/> answers "this app asked
/// to run at login", not "it will". <c>LoginItemManager</c> has the same
/// honest gap on macOS, where it is called <c>requiresApproval</c>.
/// </para>
/// <para>
/// <b>Failure is swallowed.</b> Same stance as
/// <see cref="UrlSchemeRegistration"/>: a contributor who cannot write their
/// own HKCU is in an unusual state, but the app still works. Losing
/// run-at-login costs a manual start, not a broken product, and refusing to
/// launch over it would trade a small loss for a total one.
/// </para>
/// <para>
/// <b>Inert under MSIX.</b> The packaged flavour
/// (<c>TcPackaged=true</c>) disables registry virtualization, so this write
/// would be real and would record a path inside <c>WindowsApps</c> -- the
/// same reason <c>UrlSchemeRegistration</c> skips its own registration when
/// packaged. A packaged build declares startup with a
/// <c>windows.startupTask</c> extension in the manifest instead. That
/// extension is not there yet, so a packaged build simply has no
/// run-at-login, and <see cref="IsSupported"/> says so rather than offering a
/// toggle that writes something the package model would not honour. Two
/// mechanisms at once is how a contributor ends up with two copies starting,
/// which is the failure mode <c>autostart.rs</c> documents at length for
/// Linux.
/// </para>
/// </remarks>
public static class RunAtLogin
{
    /// <summary>
    /// Whether this build can offer run-at-login at all.
    /// </summary>
    /// <remarks>
    /// False under MSIX package identity, where the mechanism belongs to the
    /// manifest rather than to this class. Callers should hide the control
    /// rather than show one that cannot work.
    /// </remarks>
    public static bool IsSupported => !PackageIdentity.IsPackaged();

    /// <summary>
    /// Whether this executable has an entry asking Windows to start it at
    /// login.
    /// </summary>
    /// <remarks>
    /// Read fresh from the registry every call rather than cached. The
    /// contributor can delete the entry from outside this app at any time,
    /// and a cached bool would then be a toggle that lies about its own
    /// state -- the same reasoning as <c>LoginItemManager.currentState</c>.
    /// </remarks>
    public static bool IsEnabled
    {
        get
        {
            string? executable = Environment.ProcessPath;
            if (!IsSupported || string.IsNullOrEmpty(executable))
            {
                return false;
            }

            try
            {
                using RegistryKey? key = Registry.CurrentUser.OpenSubKey(
                    AutostartCommand.RunKeyPath);

                return AutostartCommand.PointsAt(
                    key?.GetValue(AutostartCommand.ValueName) as string,
                    executable);
            }
            catch (Exception e) when (e is UnauthorizedAccessException or System.Security.SecurityException)
            {
                return false;
            }
        }
    }

    /// <summary>
    /// Asks Windows to start this app at login, or stops asking.
    /// </summary>
    /// <returns>
    /// The state afterwards, read back rather than assumed, so a caller that
    /// renders a checkmark renders what the registry says and not what it
    /// hoped.
    /// </returns>
    public static bool Set(bool enabled)
    {
        string? executable = Environment.ProcessPath;
        if (!IsSupported || string.IsNullOrEmpty(executable))
        {
            return false;
        }

        try
        {
            using RegistryKey key = Registry.CurrentUser.CreateSubKey(
                AutostartCommand.RunKeyPath);

            if (enabled)
            {
                // Rewritten rather than left alone when it already exists:
                // an unpackaged app lives in a folder its owner may move or
                // rename, and a stale path is an entry that starts nothing.
                key.SetValue(
                    AutostartCommand.ValueName,
                    AutostartCommand.For(executable),
                    RegistryValueKind.String);
            }
            else
            {
                // throwOnMissingValue: false -- turning off something that
                // was already off is not an error.
                key.DeleteValue(AutostartCommand.ValueName, throwOnMissingValue: false);
            }
        }
        catch (Exception e) when (e is UnauthorizedAccessException or System.Security.SecurityException)
        {
            Debug.WriteLine("tracecommons run-at-login change skipped: no HKCU write access");
        }

        return IsEnabled;
    }
}
