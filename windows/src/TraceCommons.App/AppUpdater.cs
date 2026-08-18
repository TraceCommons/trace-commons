using System;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Windows.ApplicationModel;
using Windows.Management.Deployment;

namespace TraceCommons.App;

/// <summary>
/// The app's half of the MSIX update flow.
///
/// The governing rule is that whoever installed the binary owns replacing it,
/// and on Windows desktop that is the deployment service. This class never
/// touches the install directory: it asks whether an update exists, it makes
/// sure nothing is mid-upload, and it hands off. App Installer performs the
/// swap and restarts the app.
///
/// Every call here needs package identity, which is why the project is
/// packaged. An unpackaged build reports
/// <see cref="TcUpdateAvailability.Unknown"/> rather than throwing, so a
/// developer running an unpackaged build sees "updates are not managed for
/// this installation" instead of a crash.
/// </summary>
public sealed class AppUpdater
{
    private readonly DaemonHost _host;

    public AppUpdater(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
    }

    /// <summary>
    /// The feed the deployment service polls and this class hands back to.
    ///
    /// Hard-coded rather than configurable. A configurable update source is
    /// a configurable place to be handed a different app, and the signature
    /// check that would have to defend it happens inside Windows, against
    /// whatever URI it is given.
    /// </summary>
    public static Uri FeedUri { get; } =
        new Uri("https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller");

    /// <summary>
    /// Asks the deployment service whether the feed offers something newer.
    ///
    /// The package is looked up through <see cref="PackageManager"/> rather
    /// than used straight off <c>Package.Current</c>: calling
    /// <c>CheckUpdateAvailabilityAsync</c> on the object
    /// <c>Package.Current</c> returns fails with access denied, which is a
    /// documented known issue and not something to discover at runtime.
    /// </summary>
    public async Task<TcUpdateAvailability> CheckAsync()
    {
        try
        {
            var manager = new PackageManager();
            Package current = manager.FindPackageForUser(
                string.Empty, Package.Current.Id.FullName);

            PackageUpdateAvailabilityResult result =
                await current.CheckUpdateAvailabilityAsync().AsTask().ConfigureAwait(true);

            return result.Availability switch
            {
                PackageUpdateAvailability.Available => TcUpdateAvailability.Available,
                PackageUpdateAvailability.Required => TcUpdateAvailability.Required,
                PackageUpdateAvailability.NoUpdates => TcUpdateAvailability.NoUpdates,
                PackageUpdateAvailability.Unknown => TcUpdateAvailability.Unknown,
                _ => TcUpdateAvailability.Error,
            };
        }
        catch (Exception ex) when (
            ex is InvalidOperationException      // no package identity
            or UnauthorizedAccessException
            or COMException)
        {
            // Deliberately not logging the exception. Its message can carry
            // a package full name and a path, and this is the one class that
            // sits between the deployment service and a log file.
            return TcUpdateAvailability.Unknown;
        }
    }

    /// <summary>
    /// Asks the in-process daemon to drain in-flight uploads and park the
    /// queue, bounded by <paramref name="timeoutSeconds"/>.
    ///
    /// The daemon is hosted IN THIS PROCESS, so this is not the CLI's
    /// separate-process problem: the update terminates this process and takes
    /// the daemon with it. That makes the drain the whole safety property.
    /// A refusal is honoured -- the app does not hand off, and the scheduled
    /// OnLaunch check installs the update at a calmer moment instead.
    /// </summary>
    public async Task<QuiesceOutcome> QuiesceAsync(int timeoutSeconds = 60)
    {
        string paramsJson = string.Format(
            CultureInfo.InvariantCulture,
            "{{\"timeout_secs\":{0}}}",
            timeoutSeconds);

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Quiesce, paramsJson)
            .ConfigureAwait(true);

        return UpdateProtocol.ReadQuiesce(response);
    }

    /// <summary>
    /// Hands the update to the deployment service.
    ///
    /// <c>ForceTargetAppShutdown</c> because the package being replaced is
    /// the one running this code; without it registration cannot proceed
    /// past a live process. The caller must therefore have quiesced and torn
    /// the daemon down first, because control does not reliably come back:
    /// on the success path this process is terminated part-way through the
    /// await.
    ///
    /// Returns false rather than throwing when the request is refused, so a
    /// contributor gets a sentence instead of a crash. The commonest refusal
    /// is a policy that blocks non-Store deployment, and there is nothing the
    /// app can do about it except say so.
    /// </summary>
    public async Task<bool> ApplyAsync()
    {
        try
        {
            var manager = new PackageManager();
            DeploymentResult result = await manager
                .RequestAddPackageByAppInstallerFileAsync(
                    FeedUri,
                    AddPackageByAppInstallerOptions.ForceTargetAppShutdown,
                    null!)
                .AsTask()
                .ConfigureAwait(true);

            return result.ExtendedErrorCode is null;
        }
        catch (Exception ex) when (
            ex is UnauthorizedAccessException or COMException or InvalidOperationException)
        {
            return false;
        }
    }
}
