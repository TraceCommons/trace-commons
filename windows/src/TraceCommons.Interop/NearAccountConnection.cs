using System;
using System.ComponentModel;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>Wallet ceremony lifecycle. Keys, PKCE and verification stay in the daemon.</summary>
public sealed class NearAccountConnection : INotifyPropertyChanged
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<Uri, Task<bool>> _open;
    private readonly Func<Task> _delay;
    private readonly Func<bool> _canBegin;
    private string _commons = "", _account = "";
    private string? _attempt;
    private bool _closed;
    public NearAccountConnection(Func<string, string, Task<DaemonResponse>> call,
        Func<Uri, Task<bool>> open, Func<bool> canBegin, Func<Task>? delay = null)
    { _call = call; _open = open; _canBegin = canBegin; _delay = delay ?? (() => Task.Delay(2000)); }
    public event PropertyChangedEventHandler? PropertyChanged;
    public event Action? Completed;
    public bool Supported { get; private set; }
    public bool Ready { get; private set; }
    public bool Busy { get; private set; }
    public bool CanEdit => !Busy;
    public bool CanCheck => Supported && !Busy && !string.IsNullOrWhiteSpace(Commons);
    public bool CanStart => Ready && !Busy && !string.IsNullOrWhiteSpace(Account);
    public bool CanCancel => _attempt is not null;
    public string Message { get; private set; } = "";
    public string Commons { get => _commons; set { if (Busy) return; _commons = value; Ready = false; Changed(); } }
    public string Account { get => _account; set { if (Busy) return; _account = value; Changed(); } }
    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
    private static JsonElement? Result(DaemonResponse response) => response.IsError ? null : response.Result;
    private static string? Text(JsonElement? value, string key) => value is { ValueKind: JsonValueKind.Object } obj
        && obj.TryGetProperty(key, out var field) && field.ValueKind == JsonValueKind.String ? field.GetString() : null;
    public async Task InitializeAsync()
    {
        try {
            var result = Result(await _call("hello", "{}"));
            Supported = result is { ValueKind: JsonValueKind.Object } obj
                && obj.TryGetProperty("methods", out var methods) && methods.ValueKind == JsonValueKind.Array
                && new[] { "near_account_capabilities", "near_account_start", "near_account_status", "near_account_cancel" }
                    .All(required => methods.EnumerateArray().Any(m => m.ValueKind == JsonValueKind.String && m.GetString() == required));
        } catch { Supported = false; }
        Changed();
    }
    public async Task CheckAsync()
    {
        if (!CanCheck || !_canBegin() || _closed) return;
        Busy = true; Ready = false; Message = ""; Changed();
        try {
            var result = Result(await _call("near_account_capabilities", JsonSerializer.Serialize(new { ingest_url = Commons })));
            Ready = result is { ValueKind: JsonValueKind.Object } obj && obj.TryGetProperty("ready", out var ready) && ready.ValueKind == JsonValueKind.True;
        } catch { Ready = false; }
        Message = Ready ? "This commons supports wallet signup." : "Wallet signup is unavailable for this commons. You can still use an invite.";
        Busy = false; Changed();
    }
    /// <summary>Validate the exact returned destination, including HTTPS port and credentials.</summary>
    public static Uri? BrowserDestination(string commons, string? browser)
    {
        if (!Uri.TryCreate(commons, UriKind.Absolute, out var origin) || !Uri.TryCreate(browser, UriKind.Absolute, out var target)
            || origin.Scheme != "https" || target.Scheme != "https" || origin.UserInfo.Length != 0 || target.UserInfo.Length != 0
            || !string.Equals(origin.IdnHost, target.IdnHost, StringComparison.OrdinalIgnoreCase) || origin.Port != target.Port)
            return null;
        return target;
    }
    public async Task StartAsync()
    {
        if (!CanStart || !_canBegin() || _closed) return;
        Busy = true; Message = "Opening a wallet connection…"; Changed();
        try {
            var result = Result(await _call("near_account_start", JsonSerializer.Serialize(new { ingest_url = Commons, account_id = Account.Trim() })));
            _attempt = Text(result, "attempt_id");
            var browser = BrowserDestination(Commons, Text(result, "browser_url"));
            if (_closed || string.IsNullOrEmpty(_attempt) || Text(result, "status") != "waiting_for_wallet" || browser is null) {
                await CancelAsync(); Message = "The connection could not start. Check availability and try again."; return;
            }
            Message = "Finish signing in your wallet. Keep this window open."; Changed();
            if (!await _open(browser)) { await CancelAsync(); return; }
            var id = _attempt;
            while (!_closed && id == _attempt) {
                var progress = Result(await _call("near_account_status", JsonSerializer.Serialize(new { attempt_id = id })));
                if (_closed || id != _attempt) return;
                switch (Text(progress, "status")) {
                    case "complete":
                        _attempt = null; _account = ""; Busy = false; Changed(); Completed?.Invoke(); return;
                    case "failed": case "cancelled": case "expired":
                        _attempt = null; Message = "The wallet connection did not complete. You can try again."; return;
                    case "starting": case "waiting_for_wallet": await _delay(); break;
                    default: Message = "The connection status is unavailable. Cancel and try again."; return;
                }
            }
        } catch { Message = "The connection status is unavailable. Cancel and try again."; }
        finally { Busy = _attempt is not null; Changed(); }
    }
    public async Task CancelAsync()
    {
        var id = _attempt; _attempt = null;
        if (id is not null) { try { await _call("near_account_cancel", JsonSerializer.Serialize(new { attempt_id = id })); } catch { } }
        Busy = false; Message = "Connection cancelled."; Changed();
    }
    public async Task CloseAsync() { _closed = true; await CancelAsync(); }
}
