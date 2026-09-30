using System;
using System.ComponentModel;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>Public organization context only; never a credential or payment.</summary>
public sealed record FundingDestination(string OrganizationId, string ConnectionRevision, Uri BrowserUri)
{
    public string Parameters => JsonSerializer.Serialize(new
    {
        expected_organization_id = OrganizationId,
        expected_connection_revision = ConnectionRevision,
    });
}

public sealed record FundingStatus(string Message, FundingDestination? Destination)
{
    public static FundingStatus? Parse(JsonElement value)
    {
        try
        {
            string? state = value.GetProperty("state").GetString();
            string? message = value.GetProperty("view").GetProperty("message").GetString();
            if (string.IsNullOrWhiteSpace(state) || string.IsNullOrWhiteSpace(message)
                || message.Length > 1024) return null;
            if (state != "ready") return new(message, null);
            string? organization = value.GetProperty("organization_id").GetString();
            string? revision = value.GetProperty("connection_revision").GetString();
            string? supplied = value.GetProperty("browser_url").GetString();
            if (string.IsNullOrEmpty(organization) || organization.Length > 128
                || !organization.All(c => char.IsAsciiLetterOrDigit(c) || c is '-' or '_')
                || revision is null || revision.Length != 64
                || !revision.All(c => c is >= '0' and <= '9' or >= 'a' and <= 'f')) return null;
            string url = "https://cloud.near.ai/dashboard/organizations/" + organization + "/credits";
            if (supplied != url) return null;
            return new(message, new(organization, revision, new Uri(url, UriKind.Absolute)));
        }
        catch (Exception error) when (error is JsonException or InvalidOperationException or System.Collections.Generic.KeyNotFoundException)
        {
            return null;
        }
    }
}

/// <summary>One visible funding row and its in-flight organization check.</summary>
public sealed class NearAiFunding : INotifyPropertyChanged
{
    public const string Method = "near_ai_funding";

    private readonly PrivateInferenceCopy? _copy;
    private readonly Func<string, Task<DaemonResponse>> _call;
    private readonly Func<bool> _isRunning;
    private readonly Func<Uri, Task<bool>> _open;
    private readonly Func<long> _connectionGeneration;
    private long _generation;
    private bool _active;
    private bool _pending;
    private bool _credentialBusy;
    private string _credentialState = string.Empty;
    private FundingStatus? _status;

    public NearAiFunding(PrivateInferenceCopy? copy, Func<string, Task<DaemonResponse>> call,
        Func<bool> isRunning, Func<Uri, Task<bool>> open, Func<long> connectionGeneration)
    {
        _copy = copy; _call = call; _isRunning = isRunning; _open = open;
        _connectionGeneration = connectionGeneration;
    }

    public event PropertyChangedEventHandler? PropertyChanged;
    public string Title => _copy?.FundingTitle ?? string.Empty;
    public string What => _copy?.FundingWhat ?? string.Empty;
    public string Message => _status?.Message ?? _copy?.FundingUnavailable ?? string.Empty;
    public string Action => (_status?.Destination is null ? _copy?.FundingRefresh : _copy?.FundingManage) ?? string.Empty;
    public bool Enabled => _active && !_pending && !_credentialBusy && _copy is not null && _isRunning();

    public async Task ActivateAsync()
    {
        _active = true;
        await ReadAsync(null).ConfigureAwait(true);
    }

    public void Deactivate()
    {
        _active = false;
        Invalidate();
    }

    public void CredentialChanged(string state, bool busy)
    {
        if (_credentialState == state && _credentialBusy == busy) return;
        _credentialState = state;
        _credentialBusy = busy;
        Invalidate();
    }

    private void Invalidate()
    {
        _generation++;
        _pending = false;
        _status = null;
        Raise();
    }

    public Task PressAsync() => ReadAsync(_status?.Destination);

    private async Task ReadAsync(FundingDestination? expected)
    {
        if (!Enabled) return;
        long generation = _generation;
        long connection = _connectionGeneration();
        _pending = true;
        Raise();
        try
        {
            DaemonResponse response = await _call(expected?.Parameters ?? "{}").ConfigureAwait(true);
            if (!Current(generation, connection)) return;
            FundingStatus? result = response.IsError || response.Result is null
                ? null : FundingStatus.Parse(response.Result.Value);
            if (expected is not null && result?.Destination != expected)
            {
                _status = null;
                return;
            }
            _status = result;
            if (expected is not null && Current(generation, connection))
            {
                bool opened = await _open(expected.BrowserUri).ConfigureAwait(true);
                if (Current(generation, connection) && !opened) _status = null;
            }
        }
        catch
        {
            if (Current(generation, connection)) _status = null;
        }
        finally
        {
            if (generation == _generation)
            {
                _pending = false;
                if (!_isRunning() || connection != _connectionGeneration()) _status = null;
                Raise();
            }
        }
    }

    private bool Current(long generation, long connection) =>
        generation == _generation && connection == _connectionGeneration()
        && _active && !_credentialBusy && _isRunning();

    private void Raise()
    {
        PropertyChanged?.Invoke(this, new(nameof(Message)));
        PropertyChanged?.Invoke(this, new(nameof(Action)));
        PropertyChanged?.Invoke(this, new(nameof(Enabled)));
    }
}
