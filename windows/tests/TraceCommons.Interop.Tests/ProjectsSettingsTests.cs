using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

public class ProjectsSettingsTests
{
    [Fact]
    public void BulkSelectionUsesOnlyProjectsTheSearchAndFilterShow()
    {
        ProjectSetting[] shown = [new() { ProjectId = "b" }, new() { ProjectId = "a" }];
        var selection = new HashSet<string> { "hidden", "a", "b" };

        Assert.Equal(new[] { "b", "a" }, ProjectsSettingsFlow.SelectedIds(shown, selection));
    }

    [Fact]
    public void SingleModesRespectTheDaemonsChoicesAndTheUnresolvedBucket()
    {
        var unresolved = new ProjectSetting
        {
            IsUnresolvedBucket = true,
            OfferableModes = ["ignore", "auto_upload"],
        };

        Assert.Equal(new[] { "ignore" }, ProjectsSettingsFlow.Options(unresolved));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task SingleChangeReadsTheStoredModeEvenWhenTheWriteFails(bool refused)
    {
        var project = new ProjectSetting { ProjectId = "project-a", Mode = "notify_only" };
        int reads = 0;
        var flow = new ProjectsSettingsFlow((method, _) =>
        {
            if (method == DaemonProtocol.Methods.SetProjectMode)
            {
                return Task.FromResult(refused
                    ? DaemonResponse.Parse("""{"error":{"code":"unavailable","message":"disconnected"}}""")
                    : DaemonResponse.Parse("""{"result":{}}"""));
            }

            Assert.Equal(DaemonProtocol.Methods.ListProjects, method);
            ++reads;
            return Task.FromResult(DaemonResponse.Parse("""
                {"result":{"projects":[{"project_id":"project-a","mode":"notify_only"}]}}
                """));
        });

        var result = await flow.SetProjectAsync(project, "auto_upload");

        Assert.False(result.Saved);
        Assert.NotNull(result.Projects);
        Assert.Equal("notify_only", Assert.Single(result.Projects).Mode);
        Assert.Equal(1, reads);
    }

    [Fact]
    public async Task BulkCommitIncludesOnlyTheCountsTheContributorConfirmed()
    {
        var flow = new ProjectsSettingsFlow((method, json) =>
        {
            using var payload = JsonDocument.Parse(json!);
            Assert.Equal("ignore", payload.RootElement.GetProperty("mode").GetString());
            Assert.Equal("project-a", payload.RootElement.GetProperty("project_ids")[0].GetString());
            if (method == "plan_project_modes")
            {
                Assert.False(payload.RootElement.TryGetProperty("agreed", out _));
                return Task.FromResult(DaemonResponse.Parse("""
                    {"result":{"outcome":"confirm","turns_off_automatic":1,"pending":2}}
                    """));
            }

            Assert.Equal("set_project_modes", method);
            JsonElement agreed = payload.RootElement.GetProperty("agreed");
            Assert.Equal(1, agreed.GetProperty("turns_off_automatic").GetInt32());
            Assert.Equal(2, agreed.GetProperty("pending").GetInt32());
            return Task.FromResult(DaemonResponse.Parse("""
                {"error":{"code":"bad_params","message":"bulk-confirmation-stale"}}
                """));
        });
        string[] ids = ["project-a"];
        var plan = await flow.PlanAsync(ids, "ignore");
        Assert.NotNull(plan);
        var result = await flow.ApplyAsync(ids, "ignore", plan);
        Assert.True(result.Stale);
        Assert.Null(result.Result);
    }
}
