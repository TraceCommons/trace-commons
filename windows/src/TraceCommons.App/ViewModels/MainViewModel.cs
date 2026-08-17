using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The main window's state: the pending queue, a status line, and the refresh
/// path that keeps them current.
///
/// Every member here is UI-thread-affine. <see cref="DaemonHost"/> guarantees
/// that by hopping before it raises anything, so nothing in this class needs
/// its own synchronization.
/// </summary>
public sealed class MainViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;
    private readonly AppUpdater? _updater;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private string _statusText = "Starting…";
    private string _updateStatusText = string.Empty;
    private bool _isBusy;
    private bool _isUpdateBannerVisible;
    private bool _isUpdateApplyEnabled;

    /// <summary>
    /// <paramref name="updater"/> is optional so the view model stays
    /// constructible without package identity. An unpackaged developer build
    /// then simply never shows the banner, rather than throwing at launch.
    /// </summary>
    public MainViewModel(DaemonHost host, AppUpdater? updater = null)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _updater = updater;
        _host.QueueChanged += OnQueueChanged;
        _host.StatusChanged += OnStatusChanged;
        _host.Lagged += OnLagged;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>The pending queue, newest state as the daemon reports it.</summary>
    public ObservableCollection<QueueEntryViewModel> Pending { get; } = new();

    /// <summary>
    /// A short, human-readable status line. Fixed labels only -- everything
    /// the daemon hands us is already a label rather than a path or a token,
    /// and nothing here should be the first place that stops being true.
    /// </summary>
    public string StatusText
    {
        get => _statusText;
        private set => Set(ref _statusText, value);
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (_isBusy == value)
            {
                return;
            }

            _isBusy = value;
            Raise(nameof(IsBusy));

            // Raised explicitly rather than bound through a value converter in
            // XAML. One inverted bool does not justify a converter class, and a
            // converter would also have to be registered in App.xaml resources
            // to be reachable from a DataTemplate.
            Raise(nameof(IsNotBusy));
        }
    }

    /// <summary>The inverse of <see cref="IsBusy"/>, for enabling controls.</summary>
    public bool IsNotBusy => !_isBusy;

    /// <summary>True when there is nothing pending, for an empty-state view.</summary>
    public bool IsEmpty => Pending.Count == 0;

    /// <summary>
    /// Whether the update banner is on screen. Only ever true for a
    /// confirmed offer -- see <c>UpdateProtocol.ShouldOfferUpdate</c>.
    /// </summary>
    public bool IsUpdateBannerVisible
    {
        get => _isUpdateBannerVisible;
        private set => Set(ref _isUpdateBannerVisible, value);
    }

    /// <summary>
    /// Whether the banner's action button is live. Goes false for the
    /// duration of an apply so a second click cannot start a second
    /// handoff.
    /// </summary>
    public bool IsUpdateApplyEnabled
    {
        get => _isUpdateApplyEnabled;
        private set => Set(ref _isUpdateApplyEnabled, value);
    }

    /// <summary>
    /// The banner's message. Fixed labels only, from
    /// <c>UpdateProtocol</c> -- nothing the deployment service or the daemon
    /// said reaches this string.
    /// </summary>
    public string UpdateStatusText
    {
        get => _updateStatusText;
        private set => Set(ref _updateStatusText, value);
    }

    /// <summary>
    /// Starts the daemon and loads the first queue snapshot.
    ///
    /// A start failure is shown rather than thrown: the overwhelmingly likely
    /// cause is another instance already holding the state directory's lock,
    /// which is a thing to tell the contributor plainly, not a crash.
    /// </summary>
    public async Task InitializeAsync()
    {
        try
        {
            await _host.StartAsync().ConfigureAwait(true);
        }
        catch (TcException)
        {
            // Deliberately not interpolating the exception message. It is a
            // fixed ABI label, but the UI string is more useful for saying
            // what to do about it.
            StatusText = "Could not start. Another Trace Commons instance may already be running.";
            return;
        }

        await RefreshAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Refetches the queue and the status line.
    ///
    /// Serialized by a gate rather than allowed to overlap: events can arrive
    /// in bursts, and two refreshes racing to rewrite one ObservableCollection
    /// produces flicker at best. A refresh already in flight makes a second
    /// request redundant, since the later one would read the same daemon state
    /// anyway.
    /// </summary>
    public async Task RefreshAsync()
    {
        if (!await _refreshGate.WaitAsync(0).ConfigureAwait(true))
        {
            return;
        }

        try
        {
            IsBusy = true;

            IReadOnlyList<QueueEntry> pending = await _host.ListPendingAsync().ConfigureAwait(true);
            ReplacePending(pending);

            DaemonResponse status = await _host
                .CallAsync(DaemonProtocol.Methods.Status)
                .ConfigureAwait(true);

            StatusText = status.IsError
                ? $"Daemon unavailable ({status.Error!.Code})"
                : DescribeQueue(Pending.Count);
        }
        finally
        {
            IsBusy = false;
            _refreshGate.Release();
        }
    }

    /// <summary>
    /// Asks the deployment service whether the feed offers something newer,
    /// and raises the banner if it does.
    ///
    /// Never surfaces a failed check. Windows checks the feed on its own
    /// schedule regardless of what this call returns, so a check that could
    /// not complete costs a contributor nothing and telling them about it
    /// buys nothing either.
    /// </summary>
    public async Task CheckForUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        TcUpdateAvailability availability = await _updater.CheckAsync().ConfigureAwait(true);
        if (!UpdateProtocol.ShouldOfferUpdate(availability))
        {
            IsUpdateBannerVisible = false;
            return;
        }

        UpdateStatusText = UpdateProtocol.DescribeAvailability(availability);
        IsUpdateApplyEnabled = true;
        IsUpdateBannerVisible = true;
    }

    /// <summary>
    /// Drains, tears the daemon down, and hands the update to Windows.
    ///
    /// The order is the whole point. Quiesce first, because App Installer
    /// terminates this process and a half-uploaded trace must never be the
    /// cost of an update. Then dispose the host, so the C ABI's ordered
    /// teardown runs while there is still a process to run it in. Only then
    /// hand off -- and on the success path control does not return from that
    /// call, because the process is gone.
    /// </summary>
    public async Task ApplyUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        IsUpdateApplyEnabled = false;
        UpdateStatusText = "Finishing any upload in progress…";

        QuiesceOutcome quiesce = await _updater.QuiesceAsync().ConfigureAwait(true);
        if (!quiesce.CanUpdate)
        {
            UpdateStatusText = UpdateProtocol.DescribeRefusal(quiesce.Outcome);
            return;
        }

        UpdateStatusText = "Installing the update…";
        await _host.DisposeAsync().ConfigureAwait(true);

        bool handedOff = await _updater.ApplyAsync().ConfigureAwait(true);
        if (!handedOff)
        {
            UpdateStatusText =
                "The update could not be installed. Windows will try again on its own schedule.";
        }
    }

    private static string DescribeQueue(int count) => count switch
    {
        0 => "No sessions waiting for review.",
        1 => "1 session waiting for review.",
        _ => $"{count} sessions waiting for review.",
    };

    /// <summary>
    /// Rewrites the collection in place.
    ///
    /// Clear-and-refill rather than a diff: the queue is small, the daemon is
    /// the sole authority on its contents, and a diff would introduce an
    /// opportunity for the local view to disagree with the daemon -- which is
    /// the exact class of bug a full refetch on every event exists to avoid.
    /// </summary>
    private void ReplacePending(IReadOnlyList<QueueEntry> entries)
    {
        Pending.Clear();
        foreach (QueueEntry entry in entries)
        {
            Pending.Add(new QueueEntryViewModel(entry));
        }

        Raise(nameof(IsEmpty));
    }

    private async void OnQueueChanged()
    {
        // async void because this is an event handler, which is the one place
        // it is correct. Exceptions are contained by RefreshAsync's own
        // handling of error frames; nothing it calls throws on a daemon error.
        await RefreshAsync().ConfigureAwait(true);
    }

    private async void OnStatusChanged()
    {
        await RefreshAsync().ConfigureAwait(true);
    }

    private void OnLagged(int skipped)
    {
        // Surfaced rather than swallowed. A lag means the app missed events;
        // the refetch that follows corrects the data, but the contributor
        // deserves to know the view briefly was not live.
        StatusText = skipped > 0
            ? $"Reconnecting… ({skipped} updates missed)"
            : "Reconnecting…";
    }

    private void Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return;
        }

        field = value;
        Raise(name);
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
