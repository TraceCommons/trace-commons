using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
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
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private string _statusText = "Starting…";
    private bool _isBusy;

    public MainViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
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
    /// Says that another copy of the app owns the daemon, and what to do
    /// about it.
    /// </summary>
    /// <remarks>
    /// Split into two sentences by whether an invite was on the command
    /// line, because the two situations need different next actions. Someone
    /// who double-clicked the app just needs to find the window they already
    /// have. Someone who clicked an invite link in mail is holding something
    /// they were trying to use, and needs to be told where to use it --
    /// otherwise the link looks broken and the invite looks dead, which is
    /// the impression this whole path exists to avoid giving.
    /// </remarks>
    public void ReportAlreadyRunning(bool withInvite)
    {
        StatusText = withInvite
            ? "Trace Commons is already running. Open that window and paste your invite there."
            : "Trace Commons is already running. Use the window that is already open.";
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
