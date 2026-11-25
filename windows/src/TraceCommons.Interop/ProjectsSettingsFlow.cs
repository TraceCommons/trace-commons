using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop;

public sealed record ProjectsBulkConfirmation(string Title, string Body, string Button, string Keep, string? Help, bool Destructive);

public sealed record ProjectsBulkPlan(
    string Outcome,
    [property: JsonPropertyName("turns_off_automatic")] int TurnsOffAutomatic,
    int Pending,
    ProjectsBulkConfirmation? Confirmation,
    [property: JsonPropertyName("bulk_modes")] string[]? BulkModes);

public sealed record ProjectsBulkResult(string[] Changed, int Total);

public sealed class ProjectsSettingsFlow(Func<string, string?, Task<DaemonResponse>> call)
{
    public static IReadOnlyList<string> Modes { get; } = [MapModes.Ask, MapModes.AutoUpload, MapModes.Ignore];

    public static string Mode(ProjectSetting project) => project.Mode == "ask" ? MapModes.Ask : project.Mode;

    public static string Label(ProjectSetting project) =>
        project.IsUnresolvedBucket ? UnresolvedBucketCopy.Label : project.ProjectLabel;

    public static IEnumerable<string> Options(ProjectSetting project) => Modes.Where(mode =>
        (mode != MapModes.AutoUpload || !project.IsUnresolvedBucket)
        && (project.OfferableModes is null || project.OfferableModes.Contains(mode)));

    public static string[] SelectedIds(IEnumerable<ProjectSetting> shown, IReadOnlySet<string> selection) =>
        shown.Select(row => row.ProjectId).Where(selection.Contains).ToArray();

    public async Task<IReadOnlyList<ProjectSetting>?> ReadAsync() =>
        (await call(DaemonProtocol.Methods.ListProjects, null)).ResultAs<ProjectSettingsPayload>()?.Projects;

    public async Task<(bool Saved, IReadOnlyList<ProjectSetting>? Projects)> SetProjectAsync(ProjectSetting project, string mode)
    {
        if (!Options(project).Contains(mode) || mode == Mode(project) || mode == MapModes.Ignore)
        {
            return (false, null);
        }

        DaemonResponse reply = await call(DaemonProtocol.Methods.SetProjectMode,
            JsonSerializer.Serialize(new { project_id = project.ProjectId, mode }));
        IReadOnlyList<ProjectSetting>? projects = await ReadAsync();
        return (!reply.IsError && projects?.Any(row => row.ProjectId == project.ProjectId && Mode(row) == mode) == true,
            projects);
    }

    public async Task<ProjectsBulkPlan?> PlanAsync(IReadOnlyList<string> projectIds, string mode) =>
        (await call("plan_project_modes", JsonSerializer.Serialize(new { project_ids = projectIds, mode })))
        .ResultAs<ProjectsBulkPlan>();

    public async Task<(ProjectsBulkResult? Result, bool Stale)> ApplyAsync(
        IReadOnlyList<string> projectIds, string mode, ProjectsBulkPlan? confirmed)
    {
        var agreed = confirmed is null ? null
            : new { turns_off_automatic = confirmed.TurnsOffAutomatic, pending = confirmed.Pending };
        DaemonResponse reply = await call("set_project_modes", JsonSerializer.Serialize(new { project_ids = projectIds, mode, agreed }));
        return (reply.ResultAs<ProjectsBulkResult>(), reply.Error?.Message == "bulk-confirmation-stale");
    }
}

public static class ProjectsSettingsWords
{
    private static readonly Lazy<string?> Words = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_shell_words_copy_json()));

    public static string Text(string key) => MonitorWords.WordAt(Words.Value, "projects_manager", key);

    public static string Empty => MonitorShellCopy.ProjectsEmpty;
}
