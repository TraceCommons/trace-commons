using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop.Map;

/// <summary>
/// A Traces folder row's Automatic switch (TracesViews.swift:438-451 and
/// :685-691, TracesStore.swift:1039-1061). Turning it on arms the folder with
/// the arming offer's own write, <c>set_project_mode</c> <c>auto_upload</c>
/// (<see cref="ArmingOfferFlow"/>); off sets Ask. No confirmation box
/// (owner, 2026-11-11): the daemon refuses an arm its consent does not
/// cover, and the refusal is the core's <c>project_mode_failed</c> line,
/// kept beside the folder until its next write. One write per folder at a
/// time; a confirmed write re-reads, a refused one re-reads nothing, as the
/// offer card does.
/// </summary>
public sealed class FolderAutomatic
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<Task> _refresh;
    private readonly Func<string> _refusalLine;
    private readonly HashSet<string> _writing = new(StringComparer.Ordinal);
    private readonly Dictionary<string, string> _refused = new(StringComparer.Ordinal);
    private long _generation;

    /// <param name="call">The daemon call: method, params JSON.</param>
    /// <param name="refresh">Re-reads the projects and the tree.</param>
    /// <param name="refusalLine">The core's <c>settings.project_mode_failed</c>.</param>
    public FolderAutomatic(Func<string, string, Task<DaemonResponse>> call, Func<Task> refresh, Func<string> refusalLine)
    {
        _call = call;
        _refresh = refresh;
        _refusalLine = refusalLine;
    }

    /// <summary>Raised when a write starts or ends.</summary>
    public event Action? Changed;

    /// <summary>The folders with a write in flight: their rows turn their controls off.</summary>
    public IReadOnlySet<string> Writing => _writing;

    /// <summary>The refusal line for <paramref name="projectId"/>'s last write, or null.</summary>
    public string? Refusal(string projectId) => _refused.TryGetValue(projectId, out string? line) ? line : null;

    /// <summary>
    /// The switch: on at Automatic, off at Ask, when the core offers the
    /// other; null at Never or with no mode. The bucket of traces with no
    /// project has one that stays off.
    /// </summary>
    public static bool? Switch(TracesTree.FolderNode folder)
    {
        if (folder.IsBucket)
        {
            return folder.Mode is null ? null : false;
        }

        return folder.Mode switch
        {
            MapModes.AutoUpload => folder.OfferableModes.Contains(MapModes.Ask) ? true : null,
            MapModes.Ask => folder.OfferableModes.Contains(MapModes.AutoUpload) ? false : null,
            _ => null,
        };
    }

    /// <summary>
    /// A sign-out, unenroll or another account: every write in flight is
    /// forgotten, and its reply, when it lands, changes nothing
    /// (<see cref="ArmingOfferFlow.AccountBoundary"/>).
    /// </summary>
    public void AccountBoundary()
    {
        _generation++;
        _writing.Clear();
        _refused.Clear();
        Changed?.Invoke();
    }

    /// <summary>The switch moved to <paramref name="on"/>.</summary>
    public async Task SetAsync(TracesTree.FolderNode folder, bool on)
    {
        if (folder.IsBucket || Switch(folder) is not bool now || now == on || _writing.Contains(folder.Id))
        {
            return;
        }

        string mode = on ? MapModes.AutoUpload : MapModes.Ask;
        long generation = _generation;
        _writing.Add(folder.Id);
        _refused.Remove(folder.Id);
        Changed?.Invoke();
        bool ok;
        try
        {
            DaemonResponse reply = await _call(
                DaemonProtocol.Methods.SetProjectMode,
                JsonSerializer.Serialize(new Dictionary<string, string> { ["project_id"] = folder.Id, ["mode"] = mode })).ConfigureAwait(true);
            ok = !reply.IsError;
        }
        catch (Exception e) when (e is TcException or InvalidOperationException or System.IO.IOException or JsonException)
        {
            ok = false;
        }

        if (generation != _generation)
        {
            return;
        }

        if (!ok)
        {
            _refused[folder.Id] = _refusalLine();
            _writing.Remove(folder.Id);
            Changed?.Invoke();
            return;
        }

        try
        {
            await _refresh().ConfigureAwait(true);
        }
        finally
        {
            if (generation == _generation)
            {
                _writing.Remove(folder.Id);
                Changed?.Invoke();
            }
        }
    }
}
