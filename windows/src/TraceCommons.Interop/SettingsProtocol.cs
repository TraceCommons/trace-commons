using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary><c>consent_options</c>: daemon-owned names and descriptions.</summary>
public sealed class ConsentOptionsPayload
{
    [JsonPropertyName("scopes")]
    public List<ConsentOption> Scopes { get; set; } = new();
}

public sealed class ConsentOption
{
    [JsonPropertyName("name")]
    public string Name { get; set; } = string.Empty;

    [JsonPropertyName("description")]
    public string Description { get; set; } = string.Empty;

    [JsonPropertyName("always_on")]
    public bool AlwaysOn { get; set; }

    [JsonPropertyName("grants_data_use")]
    public bool GrantsDataUse { get; set; }
}

/// <summary>
/// Privacy-safe <c>get_settings</c> projection. Paths and credentials are
/// represented only by configured-or-not booleans.
/// </summary>
public sealed class DaemonSettingsSnapshot
{
    [JsonPropertyName("quiescence_secs")]
    public ulong QuiescenceSeconds { get; set; }

    [JsonPropertyName("digest_interval_secs")]
    public ulong DigestIntervalSeconds { get; set; }

    [JsonPropertyName("approval_hold_secs")]
    public ulong ApprovalHoldSeconds { get; set; }

    [JsonPropertyName("queue_ttl_days")]
    public long QueueTtlDays { get; set; }

    [JsonPropertyName("local_notifications")]
    public bool LocalNotifications { get; set; }

    [JsonPropertyName("claude_root_configured")]
    public bool ClaudeRootConfigured { get; set; }

    [JsonPropertyName("codex_root_configured")]
    public bool CodexRootConfigured { get; set; }

    [JsonPropertyName("near_ai_configured")]
    public bool NearAiConfigured { get; set; }
}

public enum BehaviorSetting
{
    QuiescenceMinutes,
    ApprovalHoldSeconds,
    DigestHours,
}

/// <summary>Serializes exactly one <c>set_settings</c> key per user edit.</summary>
public static class BehaviorSettingsRequest
{
    public static string Serialize(BehaviorSetting setting, double displayedValue)
    {
        if (!double.IsFinite(displayedValue) || displayedValue < 0)
        {
            throw new ArgumentOutOfRangeException(nameof(displayedValue));
        }

        double seconds = setting switch
        {
            BehaviorSetting.QuiescenceMinutes => displayedValue * 60,
            BehaviorSetting.ApprovalHoldSeconds => displayedValue,
            BehaviorSetting.DigestHours => displayedValue * 3600,
            _ => throw new ArgumentOutOfRangeException(nameof(setting)),
        };
        ulong wholeSeconds = checked((ulong)Math.Round(seconds, MidpointRounding.AwayFromZero));
        string key = setting switch
        {
            BehaviorSetting.QuiescenceMinutes => "quiescence_secs",
            BehaviorSetting.ApprovalHoldSeconds => "approval_hold_secs",
            BehaviorSetting.DigestHours => "digest_interval_secs",
            _ => throw new ArgumentOutOfRangeException(nameof(setting)),
        };

        return JsonSerializer.Serialize(new Dictionary<string, ulong> { [key] = wholeSeconds });
    }
}

/// <summary><c>list_projects</c>, containing no local path.</summary>
public sealed class ProjectSettingsPayload
{
    [JsonPropertyName("projects")]
    public List<ProjectSetting> Projects { get; set; } = new();
}

public sealed class ProjectSetting
{
    [JsonPropertyName("project_id")]
    public string ProjectId { get; set; } = string.Empty;

    [JsonPropertyName("project_label")]
    public string ProjectLabel { get; set; } = string.Empty;

    [JsonPropertyName("mode")]
    public string Mode { get; set; } = "ask";

    [JsonPropertyName("configured")]
    public bool Configured { get; set; }
}

/// <summary><c>list_audit</c>'s privacy-safe local change log.</summary>
public sealed class AuditSettingsPayload
{
    [JsonPropertyName("entries")]
    public List<AuditSettingEntry> Entries { get; set; } = new();
}

public sealed class AuditSettingEntry
{
    [JsonPropertyName("at")]
    public DateTimeOffset At { get; set; }

    [JsonPropertyName("action")]
    public string Action { get; set; } = string.Empty;

    [JsonPropertyName("project_label")]
    public string? ProjectLabel { get; set; }
}
