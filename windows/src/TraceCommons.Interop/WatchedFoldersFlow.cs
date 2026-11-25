using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;

namespace TraceCommons.Interop;

/// <summary>Keep the daemon's confirmed modes. A request's path is never a displayed setting.</summary>
public sealed class WatchedFoldersFlow(Func<string, string?, Task<DaemonResponse>> call)
{
    private DaemonSettingsSnapshot? _settings;

    public bool Available => _settings is not null;
    public bool Busy { get; private set; }
    public bool SaveFailed { get; private set; }
    public string? TrajectoryMode => _settings?.TrajectorySourceMode;

    public string? Mode(string source) => source switch
    {
        "claude-code" => _settings?.ClaudeSourceMode,
        "codex" => _settings?.CodexSourceMode,
        "gemini-cli" => _settings?.GeminiSourceMode,
        "cline" => _settings?.ClineSourceMode,
        "opencode" => _settings?.OpenCodeSourceMode,
        "antigravity" => _settings?.AntigravitySourceMode,
        _ => null,
    };

    public async Task ReadAsync()
    {
        if (Busy)
        {
            return;
        }

        Busy = true;
        try
        {
            if (await ReadSnapshotAsync())
            {
                SaveFailed = false;
            }
        }
        finally
        {
            Busy = false;
        }
    }

    public Task SaveAsync(string source, ToolChoice choice) =>
        FirstRunRoots.SettingsKeys.TryGetValue(source, out string? key)
            ? WriteAsync(key, choice)
            : Task.CompletedTask;

    public Task TurnOffTrajectoryAsync() => TrajectoryMode == "watch"
        ? WriteAsync(FirstRunRoots.TrajectoryKey, ToolChoice.Off)
        : Task.CompletedTask;

    private async Task WriteAsync(string key, ToolChoice choice)
    {
        if (Busy || !Available || choice.Declaration is not { } declaration)
        {
            return;
        }

        Busy = true;
        try
        {
            SaveFailed = false;
            string payload = JsonSerializer.Serialize(new Dictionary<string, SortedDictionary<string, string>>
            {
                [key] = declaration,
            });
            DaemonResponse reply = await call(DaemonProtocol.Methods.SetSettings, payload);
            if (reply.ResultAs<DaemonSettingsSnapshot>() is { } settings)
            {
                _settings = settings;
            }
            else
            {
                SaveFailed = true;
                await ReadSnapshotAsync();
            }
        }
        finally
        {
            Busy = false;
        }
    }

    private async Task<bool> ReadSnapshotAsync()
    {
        DaemonResponse reply = await call(DaemonProtocol.Methods.GetSettings, null);
        if (reply.ResultAs<DaemonSettingsSnapshot>() is { } settings)
        {
            _settings = settings;
            return true;
        }

        return false;
    }

    /// <summary>Resolve a newly selected OpenCode database only when the core identifies it unambiguously.</summary>
    public static string OpenCodePath(string folder, string? described)
    {
        if (described is null)
        {
            return folder;
        }

        IReadOnlyList<SourceCandidate> matches = SourceDiscovery.Parse(described);
        if (matches is [{ Source: "opencode", Path.Length: > 0 } match]
            && string.Equals(Path.GetFullPath(match.Path), Path.GetFullPath(Path.Combine(folder, "opencode.db")),
                StringComparison.OrdinalIgnoreCase))
        {
            return match.Path;
        }

        return folder;
    }
}
