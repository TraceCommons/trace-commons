using System;
using System.ComponentModel;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;
namespace TraceCommons.Interop;

/// <summary>Transport and browser adapter. All wallet transitions and cadence belong to Rust.</summary>
public sealed class NearAccountConnection : INotifyPropertyChanged
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<Uri, Task<bool>> _open;
    private readonly Func<bool> _canBegin;
    private WalletView _view = new();
    private bool _pending, _closed;
    private string _commons = "", _account = "";
    public NearAccountConnection(Func<string, string, Task<DaemonResponse>> call, Func<Uri, Task<bool>> open,
        Func<bool> canBegin) { _call = call; _open = open; _canBegin = canBegin; }
    public WalletCopy? Copy { get; } = WitnessSurface.Copy()?.Wallet;
    public event PropertyChangedEventHandler? PropertyChanged;
    public event Action? Completed;
    public bool Supported => Copy is not null && _view.State != "Unsupported";
    public bool Ready => _view.CanStart;
    public bool Busy => _pending || _view.Busy;
    public bool CanEdit => !Busy && _view.CanEdit;
    public bool CanCheck => !_pending && _view.CanCheck;
    public bool CanStart => !_pending && _view.CanStart;
    public bool CanCancel => _view.CanCancel;
    public string Message => _view.Message;
    public string Glyph => _view.Glyph;
    public bool Refused => _view.Tone == "refused";
    public string RefusalMessage => Refused ? Message : "";
    public string NeutralMessage => Refused ? "" : Message;
    public string Commons { get => _commons; set { if (!Busy) { _commons = value; Changed(); } } }
    public string Account { get => _account; set { if (!Busy) { _account = value; Changed(); } } }
    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
    private async Task Command(string action)
    {
        var response = await _call("native_wallet_flow", JsonSerializer.Serialize(new {
            action, flow_id = _view.FlowId, ingest_url = Commons, account_id = Account
        }));
        _view = response.ResultAs<WalletView>() ?? throw new InvalidOperationException();
        Changed();
    }
    public async Task InitializeAsync()
    {
        try { await Command("open"); if (_closed) await Command("cancel"); } catch { }
    }
    public async Task CheckAsync()
    {
        if (!CanCheck || !_canBegin() || _closed) return;
        _pending = true; Changed();
        try { await Command("check"); }
        catch { TransportFailed(); }
        finally { _pending = false; Changed(); }
    }
    public async Task StartAsync()
    {
        if (!CanStart || !_canBegin() || _closed) return;
        _pending = true; Changed();
        try {
            await Command("start");
            if (_closed) { await Command("cancel"); return; }
            if (_view.BrowserUrl is { } browser) {
                bool opened;
                try { opened = await _open(new Uri(browser)); } catch { opened = false; }
                if (!opened) { await Command("cancel"); return; }
            }
            while (!_closed && _view.Wait) await Command("wait");
            if (!_closed && _view.State == "Complete") { _account = ""; Completed?.Invoke(); }
        } catch { TransportFailed(); }
        finally { _pending = false; Changed(); }
    }
    private void TransportFailed() { _view = _view with { Message = Copy?.Failed ?? "", Tone = "refused", Glyph = Copy?.RefusedGlyph ?? "" }; Changed(); }
    public async Task CancelAsync() { try { await Command("cancel"); } catch { TransportFailed(); } }
    public async Task CloseAsync() { _closed = true; await CancelAsync(); }
}
public sealed record WalletView
{
    [JsonPropertyName("flow_id")] public string FlowId { get; init; } = "";
    [JsonPropertyName("state")] public string State { get; init; } = "Unsupported";
    [JsonPropertyName("busy")] public bool Busy { get; init; }
    [JsonPropertyName("can_edit")] public bool CanEdit { get; init; }
    [JsonPropertyName("can_check")] public bool CanCheck { get; init; }
    [JsonPropertyName("can_start")] public bool CanStart { get; init; }
    [JsonPropertyName("can_cancel")] public bool CanCancel { get; init; }
    [JsonPropertyName("wait")] public bool Wait { get; init; }
    [JsonPropertyName("message")] public string Message { get; init; } = "";
    [JsonPropertyName("tone")] public string Tone { get; init; } = "neutral";
    [JsonPropertyName("glyph")] public string Glyph { get; init; } = "";
    [JsonPropertyName("browser_url")] public string? BrowserUrl { get; init; }
}
