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

    /// <summary>
    /// <c>watch</c>, <c>off</c> or <c>unset</c>, for the Claude Code source.
    /// </summary>
    /// <remarks>
    /// <see cref="ClaudeRootConfigured"/> cannot carry the distinction the
    /// routing surface needs: it is false both for a source pointed at the
    /// conventional location and for one the contributor said they do not
    /// use, and only the second of those reads as an unused tool.
    /// </remarks>
    [JsonPropertyName("claude_source_mode")]
    public string ClaudeSourceMode { get; set; } = string.Empty;

    [JsonPropertyName("codex_source_mode")]
    public string CodexSourceMode { get; set; } = string.Empty;

    /// <summary>
    /// The Gemini CLI source. There is deliberately no
    /// <c>gemini_root_configured</c> beside it: the <c>*_root_configured</c>
    /// pair exists for shells written before the modes did, and none of them
    /// knows about this source.
    /// </summary>
    [JsonPropertyName("gemini_source_mode")]
    public string GeminiSourceMode { get; set; } = string.Empty;

    /// <summary>
    /// The local proxy declaration as the daemon holds it. Absent means off:
    /// there is no conventional fallback for a local service, so unlike a
    /// source root there is no third state.
    /// </summary>
    [JsonPropertyName("ironwire")]
    public RoutingDeclarationSnapshot? Routing { get; set; }

    /// <summary>Whether IronWire is declared on this machine.</summary>
    public bool RoutingDeclared =>
        string.Equals(Routing?.Mode, "watch", System.StringComparison.Ordinal);
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

/// <summary>
/// <c>list_projects</c>. Carries a display path and nothing else path-shaped:
/// see <see cref="ProjectSetting.ProjectPath"/> for the one relaxation and
/// the rule it is bounded by.
/// </summary>
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

    /// <summary>
    /// The project's folder, <c>~</c>-abbreviated, for display only.
    /// </summary>
    /// <remarks>
    /// Same field, same rule, same reasoning as
    /// <see cref="QueueEntry.ProjectPath"/>: it may be rendered, and it may
    /// not be logged, audited, notified, or persisted to history. This is
    /// where history's folder paths are resolved from, by matching a
    /// record's <see cref="HistoryRecord.ProjectId"/> against these rows.
    ///
    /// Empty against a daemon predating the field.
    /// </remarks>
    [JsonPropertyName("project_path")]
    public string ProjectPath { get; set; } = string.Empty;

    [JsonPropertyName("mode")]
    public string Mode { get; set; } = "ask";

    [JsonPropertyName("configured")]
    public bool Configured { get; set; }

    /// <summary>
    /// Marks the row holding sessions whose project the daemon cannot name.
    ///
    /// The daemon decides this, because the daemon is what enforces it: it
    /// refuses <c>auto_upload</c> for that bucket in two independent places.
    /// A client MUST NOT infer the row from <c>project_label</c> -- see
    /// <c>docs/contributor-daemon-ipc-v1_1.md</c> -- because the wire carries
    /// the slug <c>unknown-project</c> as that label, and any project a
    /// contributor happened to name the same would be told it can never be
    /// armed.
    /// </summary>
    [JsonPropertyName("is_unresolved_bucket")]
    public bool IsUnresolvedBucket { get; set; }
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
