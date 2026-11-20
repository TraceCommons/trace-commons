using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The Compute section's shared words (<c>ComputeCopy</c>, ComputeSurface.swift),
/// from <c>tc_compute_copy_json</c> or a snapshot's <c>copy</c>. The optional
/// lines are null when the core left them out.
/// </summary>
public sealed record ComputeCopy(
    string Destination,
    string? Subtitle,
    string? Retry,
    string? Unavailable,
    string Introduction,
    string AllowanceLabel,
    string AllowanceDetail,
    string Enable,
    string Resume,
    string Pause,
    string Disable,
    string? QuitRefused)
{
    public static ComputeCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return From(doc.RootElement);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    internal static ComputeCopy? From(JsonElement e)
    {
        if (e.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        string? T(string key) => DaemonCalls.Text(e, key);
        return T("destination") is { } destination && T("introduction") is { } intro
            && T("allowance_label") is { } label && T("allowance_detail") is { } detail
            && T("enable") is { } enable && T("resume") is { } resume && T("pause") is { } pause
            && T("disable") is { } disable
            ? new ComputeCopy(destination, T("subtitle"), T("retry"), T("unavailable"), intro, label, detail,
                enable, resume, pause, disable, T("quit_refused"))
            : null;
    }

    /// <summary>The handle-free words, read once (<c>TCCompute.copyJSON</c>).</summary>
    public static ComputeCopy? Live => LiveCopy.Value;

    private static readonly Lazy<ComputeCopy?> LiveCopy = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_compute_copy_json())));
}

/// <summary>
/// One controller snapshot (<c>ComputeSnapshot</c>, ComputeSurface.swift).
/// Refused unless its schema is <c>trace_commons.compute_status.v1</c> and
/// its state is one this build knows.
/// </summary>
public sealed record ComputeSnapshot(
    string State,
    string Reason,
    string Title,
    string Detail,
    bool ConsentGranted,
    ulong? RamAllowanceGib,
    bool Available,
    bool CanEnable,
    bool CanResume,
    bool CanPause,
    bool? CommandPending,
    bool? WorkerStopped,
    ComputeCopy Copy)
{
    public const string Schema = "trace_commons.compute_status.v1";

    public static readonly IReadOnlyList<string> States = new[]
    {
        "disabled", "unavailable", "starting", "waiting", "training", "serving",
        "draining", "paused", "stale", "error",
    };

    public static ComputeSnapshot? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement e = doc.RootElement;
            if (e.ValueKind != JsonValueKind.Object || DaemonCalls.Text(e, "schema") != Schema
                || DaemonCalls.Text(e, "state") is not { } state || !States.Contains(state)
                || DaemonCalls.Text(e, "reason") is not { } reason || DaemonCalls.Text(e, "title") is not { } title
                || DaemonCalls.Text(e, "detail") is not { } detail
                || Bool(e, "consent_granted") is not { } consent || Bool(e, "available") is not { } available
                || Bool(e, "can_enable") is not { } canEnable || Bool(e, "can_resume") is not { } canResume
                || Bool(e, "can_pause") is not { } canPause
                || !e.TryGetProperty("copy", out JsonElement copyElement) || ComputeCopy.From(copyElement) is not { } copy)
            {
                return null;
            }

            ulong? allowance = null;
            if (e.TryGetProperty("ram_allowance_gib", out JsonElement a) && a.ValueKind != JsonValueKind.Null)
            {
                if (!a.TryGetUInt64(out ulong gib))
                {
                    return null;
                }

                allowance = gib;
            }

            return new ComputeSnapshot(state, reason, title, detail, consent, allowance, available, canEnable,
                canResume, canPause, Bool(e, "command_pending"), Bool(e, "worker_stopped"), copy);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static bool? Bool(JsonElement e, string key) =>
        e.TryGetProperty(key, out JsonElement v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? v.ValueKind == JsonValueKind.True
            : null;
}

/// <summary>
/// What the allowance field accepts: a whole number above zero
/// (<c>ComputeAllowance.parse</c>, ComputeView.swift:20-27).
/// </summary>
public static class ComputeAllowance
{
    public static ulong? Parse(string text) =>
        // Swift's UInt64(String) takes ASCII digits with an optional '+'.
        text.Length > 0 && (text[0] == '+' ? text.Length > 1 && text.Skip(1).All(char.IsAsciiDigit) : text.All(char.IsAsciiDigit))
            && ulong.TryParse(text, System.Globalization.NumberStyles.AllowLeadingSign, System.Globalization.CultureInfo.InvariantCulture, out ulong value)
            && value > 0
            ? value
            : null;
}

/// <summary>A compute command (<c>TCCompute.Command</c>).</summary>
public abstract record ComputeCommand
{
    public sealed record Enable(ulong RamAllowanceGib) : ComputeCommand;
    public sealed record Resume : ComputeCommand;
    public sealed record Pause : ComputeCommand;
    public sealed record Disable : ComputeCommand;

    public string Json() => this switch
    {
        Enable e => JsonSerializer.Serialize(new Dictionary<string, object> { ["command"] = "enable", ["ram_allowance_gib"] = e.RamAllowanceGib }),
        Resume => """{"command":"resume"}""",
        Pause => """{"command":"pause"}""",
        _ => """{"command":"disable"}""",
    };
}

/// <summary>A refused compute call: the label <c>tc_last_error</c> named.</summary>
public sealed class ComputeRefusedException : Exception
{
    public ComputeRefusedException(string label)
        : base(label)
    {
        Label = label;
    }

    public string Label { get; }
}

/// <summary>
/// The app-owned compute controller (<c>TCCompute</c>, TCCompute.swift). It
/// needs neither a trace daemon nor enrollment. Calls do settings I/O, so run
/// them off the UI thread. The handle is freed only when the controller says
/// its worker stopped and no command is pending, as the Mac's
/// <c>close()</c> does; otherwise it is kept so a stop can be retried.
/// </summary>
public sealed class TcCompute : IDisposable
{
    private readonly object _lock = new();
    private IntPtr _handle;
    private int _activeCalls;

    public TcCompute(string configDirectory)
    {
        if (configDirectory.Contains('\0', StringComparison.Ordinal))
        {
            throw new ComputeRefusedException("invalid-input");
        }

        IntPtr opened = NativeMethods.tc_compute_open(configDirectory, out IntPtr err);
        string? label = NativeMethods.TakeOwnedString(err);
        if (opened == IntPtr.Zero)
        {
            throw new ComputeRefusedException(label ?? "panic");
        }

        _handle = opened;
    }

    /// <summary>Whether the handle has been freed.</summary>
    public bool IsClosed
    {
        get
        {
            lock (_lock)
            {
                return _handle == IntPtr.Zero;
            }
        }
    }

    public string StatusJson() => WithHandle(h => Result(NativeMethods.tc_compute_status_json(h)));

    public string CommandJson(ComputeCommand command)
    {
        string json = command.Json();
        return WithHandle(h => Result(NativeMethods.tc_compute_command_json(h, json)));
    }

    public string ShutdownJson(ulong timeoutMilliseconds) =>
        WithHandle(h => Result(NativeMethods.tc_compute_shutdown(h, timeoutMilliseconds)));

    /// <summary>
    /// Frees the handle once the controller proves its worker stopped and no
    /// command is pending. Idempotent; never races another handle call.
    /// </summary>
    public bool Close()
    {
        lock (_lock)
        {
            if (_handle == IntPtr.Zero)
            {
                return true;
            }

            if (_activeCalls != 0)
            {
                return false;
            }

            string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_compute_status_json(_handle));
            if (!StoppedEvidence(json))
            {
                return false;
            }

            IntPtr owned = _handle;
            _handle = IntPtr.Zero;
            NativeMethods.tc_compute_free(owned);
            return true;
        }
    }

    public void Dispose() => Close();

    /// <summary><c>worker_stopped</c> true and <c>command_pending</c> false (<c>CloseEvidence</c>).</summary>
    internal static bool StoppedEvidence(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return false;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement e = doc.RootElement;
            return e.ValueKind == JsonValueKind.Object
                && e.TryGetProperty("worker_stopped", out JsonElement stopped) && stopped.ValueKind == JsonValueKind.True
                && e.TryGetProperty("command_pending", out JsonElement pending) && pending.ValueKind == JsonValueKind.False;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private T WithHandle<T>(Func<IntPtr, T> operation)
    {
        IntPtr handle;
        lock (_lock)
        {
            if (_handle == IntPtr.Zero)
            {
                throw new ComputeRefusedException("closed");
            }

            handle = _handle;
            _activeCalls++;
        }

        try
        {
            return operation(handle);
        }
        finally
        {
            lock (_lock)
            {
                _activeCalls--;
            }
        }
    }

    private static string Result(IntPtr pointer)
    {
        string? value = NativeMethods.TakeOwnedString(pointer);
        if (value is not null)
        {
            return value;
        }

        // Borrowed thread-local storage: copy at once, never free it.
        IntPtr error = NativeMethods.tc_last_error();
        throw new ComputeRefusedException(error == IntPtr.Zero ? "panic" : Marshal.PtrToStringUTF8(error) ?? "panic");
    }
}

/// <summary>
/// The Compute section's state (<c>ComputeModel</c>, ComputeModel.swift):
/// opened once against this app's state directory, a command only while the
/// snapshot allows it, a status read each second while the section is
/// shown, and a failure said with the core's words. Drive it from the UI
/// thread; the handle's calls run on the thread pool.
/// </summary>
public sealed class ComputeModel
{
    private readonly Func<string, TcCompute> _open;
    private TcCompute? _handle;
    private bool _started;
    private bool _closed;
    private string? _directory;

    public ComputeModel(Func<string, TcCompute>? open = null)
    {
        _open = open ?? (dir => new TcCompute(dir));
    }

    public event Action? Changed;

    public ComputeSnapshot? Snapshot { get; private set; }
    public string? FailureLabel { get; private set; }
    public bool Busy { get; private set; }

    /// <summary>The handle-free words, for the failure line and Retry.</summary>
    public ComputeCopy? Copy { get; } = ComputeCopy.Live;

    /// <summary>Controls wait while a call or a command is out (<c>controlsBusy</c>).</summary>
    public bool ControlsBusy => Busy || Snapshot?.CommandPending == true;

    public async Task StartAsync(string configDirectory)
    {
        if (_started || _closed)
        {
            return;
        }

        _directory = configDirectory;
        _started = true;
        Busy = true;
        Changed?.Invoke();
        try
        {
            string json = await Task.Run(() =>
            {
                _handle ??= _open(configDirectory);
                return _handle.StatusJson();
            }).ConfigureAwait(true);
            Publish(json);
        }
        catch (ComputeRefusedException e)
        {
            Fail(e.Label);
        }
        finally
        {
            Busy = false;
            Changed?.Invoke();
        }
    }

    /// <summary>Opens again after a failure (<c>retryOpen</c>).</summary>
    public async Task RetryAsync()
    {
        if (ControlsBusy || _closed || _directory is null)
        {
            return;
        }

        _started = false;
        await StartAsync(_directory).ConfigureAwait(true);
    }

    /// <summary>Records a failure the shell decided before opening (sample data, no directory).</summary>
    public void Refuse(string label)
    {
        Fail(label);
        Changed?.Invoke();
    }

    public async Task PerformAsync(ComputeCommand command)
    {
        if (_closed || ControlsBusy || Snapshot is not { } s || _handle is not { } handle)
        {
            return;
        }

        bool allowed = command switch
        {
            ComputeCommand.Enable => s.Available && s.CanEnable,
            ComputeCommand.Resume => s.Available && s.CanResume,
            ComputeCommand.Pause => s.CanPause,
            _ => s.ConsentGranted,
        };
        if (!allowed)
        {
            return;
        }

        Busy = true;
        Changed?.Invoke();
        try
        {
            Publish(await Task.Run(() => handle.CommandJson(command)).ConfigureAwait(true));
        }
        catch (ComputeRefusedException e)
        {
            Fail(e.Label);
        }
        finally
        {
            Busy = false;
            Changed?.Invoke();
        }
    }

    /// <summary>One status read; the section's clock calls it each second while shown.</summary>
    public async Task RefreshAsync()
    {
        if (!_started || Busy || _closed || _handle is not { } handle || Snapshot is null)
        {
            return;
        }

        try
        {
            Publish(await Task.Run(handle.StatusJson).ConfigureAwait(true));
        }
        catch (ComputeRefusedException e)
        {
            Fail(e.Label);
        }

        Changed?.Invoke();
    }

    /// <summary>
    /// At quit: stops the controller within <paramref name="timeoutMilliseconds"/>
    /// and frees the handle once its worker is proved stopped, as the Mac's
    /// quit does (AppDelegate.swift:105-108), so no worker outlives the app.
    /// False when the worker could not be proved stopped; the handle is then
    /// kept. A model never opened has nothing to stop.
    /// </summary>
    public async Task<bool> ShutdownAsync(ulong timeoutMilliseconds)
    {
        if (_handle is { } handle)
        {
            try
            {
                Publish(await Task.Run(() => handle.ShutdownJson(timeoutMilliseconds)).ConfigureAwait(true));
            }
            catch (ComputeRefusedException e)
            {
                Fail(e.Label);
                Changed?.Invoke();
                return false;
            }
        }

        return Close();
    }

    /// <summary>Frees the handle when its worker is proved stopped; false keeps it for a retry.</summary>
    public bool Close()
    {
        if (_handle is { } handle && !handle.Close())
        {
            return false;
        }

        _handle = null;
        _closed = true;
        Snapshot = null;
        Changed?.Invoke();
        return true;
    }

    private void Publish(string json)
    {
        if (ComputeSnapshot.Decode(json) is not { } value)
        {
            Snapshot = null;
            FailureLabel = "compute-status-invalid";
            return;
        }

        Snapshot = value;
        FailureLabel = null;
    }

    private void Fail(string label)
    {
        Snapshot = null;
        FailureLabel = label;
    }
}
