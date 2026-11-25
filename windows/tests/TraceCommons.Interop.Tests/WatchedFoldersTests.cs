using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

public class WatchedFoldersTests
{
    [Fact]
    public void OnlyAnUnambiguousOpenCodeDatabaseChangesTheChosenFolder()
    {
        string folder = Path.Combine(Path.GetTempPath(), "OpenCode");
        string database = Path.Combine(folder, "opencode.db");
        var databaseMatch = new SourceCandidate { Source = "opencode", Path = database };
        var exports = new SourceCandidate { Source = "trajectory", Path = folder };

        Assert.Equal(database, WatchedFoldersFlow.OpenCodePath(folder, JsonSerializer.Serialize(new[] { databaseMatch })));
        Assert.Equal(folder, WatchedFoldersFlow.OpenCodePath(folder, JsonSerializer.Serialize(new[] { databaseMatch, exports })));
    }

    [Fact]
    public async Task WritesOnlyTheChosenSourceAndAdoptsTheConfirmedModes()
    {
        var calls = new List<string>();
        var folders = new WatchedFoldersFlow((method, payload) =>
        {
            calls.Add(method);
            if (method == DaemonProtocol.Methods.GetSettings)
            {
                return Task.FromResult(Reply("""{"codex_source_mode":"unset"}"""));
            }

            using JsonDocument document = JsonDocument.Parse(payload!);
            JsonProperty source = Assert.Single(document.RootElement.EnumerateObject());
            Assert.Equal("codex_source", source.Name);
            Assert.Equal("watch", source.Value.GetProperty("mode").GetString());
            Assert.Equal(@"C:\traces", source.Value.GetProperty("path").GetString());
            return Task.FromResult(Reply("""{"codex_source_mode":"watch","trajectory_source_mode":"off"}"""));
        });

        await folders.ReadAsync();
        Assert.Equal("unset", folders.Mode("codex"));
        await folders.SaveAsync("codex", ToolChoice.Watch(@"C:\traces"));

        Assert.Equal(new[] { "get_settings", "set_settings" }, calls);
        Assert.Equal("watch", folders.Mode("codex"));
        Assert.Equal("off", folders.TrajectoryMode);
        Assert.False(folders.SaveFailed);
    }

    [Fact]
    public async Task ARefusedWriteReadsBackAndKeepsFailureVisibleUntilRetry()
    {
        int reads = 0;
        var folders = new WatchedFoldersFlow((method, _) =>
            Task.FromResult(method == DaemonProtocol.Methods.GetSettings
                ? Reply(++reads == 1 ? """{"opencode_source_mode":"watch"}""" : """{"opencode_source_mode":"off"}""")
                : DaemonResponse.Parse("""{"error":{"code":"unavailable","message":"disconnected"}}""")));

        await folders.ReadAsync();
        Assert.Equal("watch", folders.Mode("opencode"));
        await folders.SaveAsync("opencode", ToolChoice.Off);

        Assert.Equal(2, reads);
        Assert.Equal("off", folders.Mode("opencode"));
        Assert.True(folders.SaveFailed);
        await folders.ReadAsync();
        Assert.False(folders.SaveFailed);
    }

    private static DaemonResponse Reply(string snapshot) => DaemonResponse.Parse("{\"result\":" + snapshot + "}");
}
