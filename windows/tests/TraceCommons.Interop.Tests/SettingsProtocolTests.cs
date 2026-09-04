using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class SettingsProtocolTests
{
    [Fact]
    public void SettingsExposeBehaviorAndConfigurationFactsButNoPrivateValues()
    {
        DaemonSettingsSnapshot? settings = Response(
            """{"quiescence_secs":1800,"digest_interval_secs":14400,"approval_hold_secs":10,"queue_ttl_days":14,"local_notifications":false,"claude_root_configured":true,"codex_root_configured":false,"near_ai_configured":true}""")
            .ResultAs<DaemonSettingsSnapshot>();

        Assert.NotNull(settings);
        Assert.True(settings!.ClaudeRootConfigured);
        Assert.False(settings.CodexRootConfigured);
        Assert.True(settings.NearAiConfigured);
        Assert.Equal(1800UL, settings.QuiescenceSeconds);
        Assert.Equal(14400UL, settings.DigestIntervalSeconds);
        Assert.Equal(10UL, settings.ApprovalHoldSeconds);
        Assert.Equal(14, settings.QueueTtlDays);

        Assert.DoesNotContain(
            typeof(DaemonSettingsSnapshot).GetProperties(),
            property => property.Name.Contains("Path", StringComparison.Ordinal)
                        || property.Name.Contains("Token", StringComparison.Ordinal)
                        || property.Name.Contains("Credential", StringComparison.Ordinal));
    }

    [Theory]
    [InlineData(BehaviorSetting.QuiescenceMinutes, 30, "quiescence_secs", 1800)]
    [InlineData(BehaviorSetting.ApprovalHoldSeconds, 10, "approval_hold_secs", 10)]
    [InlineData(BehaviorSetting.DigestHours, 4, "digest_interval_secs", 14400)]
    public void BehaviorEditsSendExactlyOneSetting(
        BehaviorSetting setting,
        double displayed,
        string expectedKey,
        ulong expectedSeconds)
    {
        using JsonDocument json = JsonDocument.Parse(
            BehaviorSettingsRequest.Serialize(setting, displayed));

        JsonProperty property = Assert.Single(json.RootElement.EnumerateObject());
        Assert.Equal(expectedKey, property.Name);
        Assert.Equal(expectedSeconds, property.Value.GetUInt64());
    }

    [Fact]
    public void AuditRowsExposeFixedLabelsButNotDetail()
    {
        AuditSettingsPayload? payload = Response(
            """{"entries":[{"at":"2026-08-18T12:30:00Z","action":"consent-scopes-changed","project_label":null,"detail":"fixed-label"}]}""")
            .ResultAs<AuditSettingsPayload>();

        AuditSettingEntry entry = Assert.Single(payload!.Entries);
        Assert.Equal("consent-scopes-changed", entry.Action);
        Assert.Null(entry.ProjectLabel);
        Assert.DoesNotContain(
            typeof(AuditSettingEntry).GetProperties(),
            property => property.Name.Contains("Detail", StringComparison.Ordinal)
                        || property.Name.Contains("Path", StringComparison.Ordinal)
                        || property.Name.Contains("Key", StringComparison.Ordinal));
    }

    [Fact]
    public void ProjectsUseOpaqueIdsAndDisplayLabels()
    {
        ProjectSettingsPayload? payload = Response(
            """{"projects":[{"project_id":"sha256:opaque","project_label":"client-api","mode":"ignore","configured":true}]}""")
            .ResultAs<ProjectSettingsPayload>();

        ProjectSetting project = Assert.Single(payload!.Projects);
        Assert.Equal("sha256:opaque", project.ProjectId);
        Assert.Equal("client-api", project.ProjectLabel);
        Assert.Equal("ignore", project.Mode);

        // The key is the identity, and it is the full local path folded for
        // case. It must never cross this socket, and neither must any other
        // path -- except the one the folder-first queue exists to show.
        // `ProjectPath` is that one: a `~`-abbreviated folder for display,
        // sanctioned by `ProjectEntry::display_path` on the daemon side and
        // bound there to "rendered, never logged, audited, notified, or
        // persisted to history". Naming it explicitly keeps this guard able
        // to fail on a SECOND path field, which is what it is really for.
        Assert.DoesNotContain(
            typeof(ProjectSetting).GetProperties(),
            property => property.Name.Contains("Key", StringComparison.Ordinal)
                        || (property.Name.Contains("Path", StringComparison.Ordinal)
                            && property.Name != nameof(ProjectSetting.ProjectPath)));
    }

    [Fact]
    public void StatusCarriesTheCurrentConsentSet()
    {
        DaemonStatus? status = Response(
            """{"logged_in":true,"paused":false,"queue_depth":0,"consent_scopes":["debugging_evaluation","benchmark_creation"]}""")
            .ResultAs<DaemonStatus>();

        Assert.NotNull(status);
        Assert.True(status!.LoggedIn);
        Assert.Equal(2, status.ConsentScopes.Count);
        Assert.Contains("benchmark_creation", status.ConsentScopes);
    }

    private static DaemonResponse Response(string result) =>
        DaemonResponse.Parse($$"""{"id":1,"result":{{result}}}""");
}
