using System;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Windows.ApplicationModel;

namespace TraceCommons.App;

/// <summary>
/// The single run-at-login surface for both Windows distribution flavours.
/// </summary>
/// <remarks>
/// The portable build owns an HKCU Run value through <see cref="RunAtLogin"/>.
/// The MSIX build owns the <c>TraceCommonsStartup</c> manifest task and must use
/// <see cref="StartupTask"/>. Keeping the choice here prevents either caller
/// from registering both mechanisms and launching two watchers at sign-in.
/// </remarks>
public static class StartupRegistration
{
    internal const string PackagedTaskId = "TraceCommonsStartup";

    public static async Task<StartupRegistrationState> GetStateAsync()
    {
        // A recorded-sample launch offers no switch, so it can never write
        // the Run value or the startup task.
        if (Program.IsSampleLaunch)
        {
            return StartupRegistrationState.SampleLaunch;
        }

        if (!PackageIdentity.IsPackaged())
        {
            return new StartupRegistrationState(
                IsSupported: RunAtLogin.IsSupported,
                IsEnabled: RunAtLogin.IsEnabled,
                Notice: string.Empty);
        }

        try
        {
            StartupTask task = await StartupTask.GetAsync(PackagedTaskId);
            return FromPackagedState(task.State);
        }
        catch (Exception e) when (
            e is ArgumentException or InvalidOperationException or UnauthorizedAccessException or COMException)
        {
            return StartupRegistrationState.Unavailable;
        }
    }

    public static async Task<StartupRegistrationState> SetEnabledAsync(bool enabled)
    {
        if (Program.IsSampleLaunch)
        {
            return StartupRegistrationState.SampleLaunch;
        }

        if (!PackageIdentity.IsPackaged())
        {
            bool actual = RunAtLogin.Set(enabled);
            return new StartupRegistrationState(
                IsSupported: RunAtLogin.IsSupported,
                IsEnabled: actual,
                Notice: actual == enabled
                    ? string.Empty
                    : "Windows couldn't change startup registration just now.");
        }

        try
        {
            StartupTask task = await StartupTask.GetAsync(PackagedTaskId);
            if (enabled)
            {
                StartupTaskState state = await task.RequestEnableAsync();
                return FromPackagedState(state);
            }

            task.Disable();
            return FromPackagedState(task.State);
        }
        catch (Exception e) when (
            e is ArgumentException or InvalidOperationException or UnauthorizedAccessException or COMException)
        {
            return StartupRegistrationState.Unavailable;
        }
    }

    private static StartupRegistrationState FromPackagedState(StartupTaskState state) => state switch
    {
        StartupTaskState.Enabled =>
            new StartupRegistrationState(true, true, string.Empty),
        StartupTaskState.EnabledByPolicy =>
            new StartupRegistrationState(true, true, string.Empty, CanChange: false),
        StartupTaskState.DisabledByUser =>
            new StartupRegistrationState(
                true,
                false,
                $"Startup was disabled in Windows. Turn {TraceCommons.Interop.Shell.MonitorWords.AppName} on in Settings > Apps > Startup.",
                CanChange: false),
        StartupTaskState.DisabledByPolicy =>
            new StartupRegistrationState(
                true,
                false,
                "Startup is disabled by your organization's Windows policy.",
                CanChange: false),
        _ => new StartupRegistrationState(true, false, string.Empty),
    };
}

public sealed record StartupRegistrationState(bool IsSupported, bool IsEnabled, string Notice, bool CanChange = true)
{
    public static StartupRegistrationState Unavailable { get; } = new(
        false,
        false,
        "Windows startup settings aren't available just now.");

    /// <summary>
    /// A recorded-sample launch (Debug only): no switch and no notice, so
    /// nothing is offered that would write the Run value or the task.
    /// </summary>
    internal static StartupRegistrationState SampleLaunch { get; } = new(false, false, string.Empty);
}
