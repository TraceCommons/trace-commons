using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

public sealed class MissionDraftsViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly ILocalMissionDrafts _service;
    private readonly SemaphoreSlim _actions = new(1, 1);
    private CancellationTokenSource _lifetime = new();
    private long _generation;
    private IReadOnlyDictionary<string,string> _copy = new Dictionary<string,string>();
    private StoredMissionDraft? _current;
    private string _status = string.Empty;
    private bool _busy;
    private bool _disposed;

    public MissionDraftsViewModel(ILocalMissionDrafts? service = null) => _service = service ?? new LocalMissionDrafts();
    public event PropertyChangedEventHandler? PropertyChanged;
    public ObservableCollection<MissionDraftSummary> Drafts { get; } = new();
    public StoredMissionDraft? Current { get => _current; private set { _current = value; Raise(); Raise(nameof(HasCurrent)); } }
    public bool HasCurrent => Current != null;
    public string Status { get => _status; private set { _status = value; Raise(); } }
    public bool IsBusy { get => _busy; private set { _busy = value; Raise(); Raise(nameof(CanAct)); } }
    public bool CanAct => !IsBusy;
    public bool IsEmpty => Drafts.Count == 0;
    public string this[string key] => _copy.TryGetValue(key, out string? value) ? value : string.Empty;

    public async Task LoadAsync()
    {
        if (!await EnterAsync()) return;
        long ticket = Begin();
        CancellationToken token = _lifetime.Token;
        try
        {
            var copy = MissionDraftDecoding.Copy(await _service.CallAsync(new { type = "copy" }, token));
            if (!CurrentTicket(ticket)) return;
            _copy = copy;
            RaiseCopy();
            var drafts = MissionDraftDecoding.List(await _service.CallAsync(new { type = "list" }, token));
            if (!CurrentTicket(ticket)) return;
            Replace(drafts);
            Status = Drafts.Count == 0 ? this["empty"] : this["refreshed"];
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (CurrentTicket(ticket)) Status = this["error"]; }
        finally { End(ticket); _actions.Release(); }
    }

    public async Task RefreshAsync()
    {
        if (!await EnterAsync()) return;
        long ticket = Begin();
        CancellationToken token = _lifetime.Token;
        try
        {
            var drafts = MissionDraftDecoding.List(await _service.CallAsync(new { type = "list" }, token));
            if (!CurrentTicket(ticket)) return;
            Replace(drafts);
            if (Current != null && !Contains(Current.Id)) Current = null;
            Status = Drafts.Count == 0 ? this["empty"] : this["refreshed"];
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (CurrentTicket(ticket)) Status = this["error"]; }
        finally { End(ticket); _actions.Release(); }
    }

    public async Task ImportAsync(string file)
    {
        if (!await EnterAsync()) return;
        long ticket = Begin();
        CancellationToken token = _lifetime.Token;
        try
        {
            var imported = MissionDraftDecoding.Import(await _service.CallAsync(new { type = "import", file }, token));
            if (!CurrentTicket(ticket)) return;
            Status = this[imported.Inserted ? "added" : "duplicate"];
            try
            {
                var drafts = MissionDraftDecoding.List(await _service.CallAsync(new { type = "list" }, token));
                if (CurrentTicket(ticket)) Replace(drafts);
            }
            catch (Exception) when (CurrentTicket(ticket)) { Status += "\n" + this["error"]; }
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (CurrentTicket(ticket)) Status = this["error"]; }
        finally { End(ticket); _actions.Release(); }
    }

    public async Task ShowAsync(string id)
    {
        if (!await EnterAsync()) return;
        long ticket = Begin();
        CancellationToken token = _lifetime.Token;
        try
        {
            var shown = MissionDraftDecoding.Show(await _service.CallAsync(new { type = "show", id }, token));
            if (!CurrentTicket(ticket) || shown.Id != id || !Contains(id)) return;
            Current = shown;
            Status = this["display_notice"];
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (CurrentTicket(ticket)) Status = this["error"]; }
        finally { End(ticket); _actions.Release(); }
    }

    public async Task DeleteAsync(string id)
    {
        if (!await EnterAsync()) return;
        long ticket = Begin();
        CancellationToken token = _lifetime.Token;
        try
        {
            var deleted = MissionDraftDecoding.Delete(await _service.CallAsync(new { type = "delete", id }, token));
            if (!CurrentTicket(ticket) || deleted.Id != id) return;
            for (int index = Drafts.Count - 1; index >= 0; index--) if (Drafts[index].Id == id) Drafts.RemoveAt(index);
            Raise(nameof(IsEmpty));
            if (Current?.Id == id) Current = null;
            Status = this["deleted"];
            try
            {
                var drafts = MissionDraftDecoding.List(await _service.CallAsync(new { type = "list" }, token));
                if (CurrentTicket(ticket)) Replace(drafts);
            }
            catch (Exception) when (CurrentTicket(ticket)) { Status += "\n" + this["error"]; }
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (CurrentTicket(ticket)) Status = this["error"]; }
        finally { End(ticket); _actions.Release(); }
    }

    public void ReportFileSelected() => Status = this["file_selected"];
    public void ReportError() => Status = this["error"];
    public void Deactivate() { Interlocked.Increment(ref _generation); _lifetime.Cancel(); _lifetime.Dispose(); _lifetime = new(); IsBusy = false; Current = null; }
    public void Dispose() { if (_disposed) return; _disposed = true; Interlocked.Increment(ref _generation); _lifetime.Cancel(); _lifetime.Dispose(); }

    private long Begin() { long ticket = Interlocked.Increment(ref _generation); IsBusy = true; Status = this["working"]; return ticket; }
    private async Task<bool> EnterAsync()
    {
        CancellationToken token = _lifetime.Token;
        long generation = Interlocked.Read(ref _generation);
        try
        {
            await _actions.WaitAsync(token);
            if (!_disposed && !token.IsCancellationRequested && generation == Interlocked.Read(ref _generation)) return true;
            _actions.Release();
            return false;
        }
        catch (OperationCanceledException) { return false; }
    }
    private bool CurrentTicket(long ticket) => !_disposed && ticket == Interlocked.Read(ref _generation);
    private void End(long ticket) { if (CurrentTicket(ticket)) IsBusy = false; }
    private bool Contains(string id) { foreach (var draft in Drafts) if (draft.Id == id) return true; return false; }
    private void Replace(IReadOnlyList<MissionDraftSummary> drafts) { Drafts.Clear(); foreach (var draft in drafts) Drafts.Add(draft); Raise(nameof(IsEmpty)); }
    private void RaiseCopy() { Raise("Item[]"); Raise(nameof(Status)); }
    private void Raise([CallerMemberName] string? name = null) => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
