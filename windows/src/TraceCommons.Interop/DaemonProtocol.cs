using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The wire shapes of the <c>trace_commons.daemon.v1_1</c> IPC contract, as
/// served in-process through <see cref="TcDaemon.Call"/>.
///
/// Mirrors <c>crates/trace-commons-contributor/src/daemon/ipc.rs</c>. Only the
/// subset this app currently drives is modelled; the daemon exposes 27
/// methods and unmodelled ones stay reachable as raw JSON through
/// <see cref="TcDaemon.Call"/> rather than being blocked by this file.
/// </summary>
public static class DaemonProtocol
{
    /// <summary>The schema this client speaks.</summary>
    public const string SchemaVersion = "trace_commons.daemon.v1_1";

    /// <summary>
    /// Versions the daemon accepts. A v1 client remains supported, so a
    /// handshake reporting only v1 is a downgrade to notice, not a failure.
    /// </summary>
    public static readonly string[] SupportedVersions =
    {
        "trace_commons.daemon.v1",
        "trace_commons.daemon.v1_1",
    };

    public static class Methods
    {
        public const string Hello = "hello";
        public const string Status = "status";
        public const string ListPending = "list_pending";
        public const string Pause = "pause";
        public const string Resume = "resume";
        public const string Approve = "approve";
        public const string Dismiss = "dismiss";

        /// <summary>
        /// Recalls an approval while the daemon's hold is still running. This
        /// is what the undo bar is made of; without it "Sending…" would be a
        /// label rather than a reprieve.
        /// </summary>
        public const string Cancel = "cancel";
        public const string Shutdown = "shutdown";

        // Onboarding. Every one of these was already in the daemon's pinned
        // METHODS array before this app could call any of them: the gap on
        // Windows was never protocol, only that nothing here asked.
        public const string Enroll = "enroll";
        public const string ConsentOptions = "consent_options";
        public const string SetConsentScopes = "set_consent_scopes";
        public const string GetSettings = "get_settings";
        public const string ListProjects = "list_projects";
        public const string SetProjectMode = "set_project_mode";
        public const string AcknowledgeNearAiNotice = "acknowledge_near_ai_notice";

        // History and withdrawal. Like the onboarding block above, every one
        // of these was already in the daemon's pinned METHODS array before
        // this app could call any of them -- the gap on Windows was never
        // protocol, only that nothing here asked. Adding a name that is NOT
        // in that array would break
        // `hello_advertises_exactly_the_documented_method_set`.
        public const string ListHistory = "list_history";
        public const string HistoryRollup = "history_rollup";
        public const string RefreshHistory = "refresh_history";
        public const string QueueOutcomeCounts = "queue_outcome_counts";

        /// <summary>
        /// Withdraws one submission and reports the tier the server applied.
        ///
        /// Deliberately paired with no <c>withdraw_bulk</c> constant. Bulk
        /// reports only counts, so a bulk outcome cannot name a per-trace
        /// tier, and the contract's first withdrawal rule -- never a generic
        /// "withdrawn" -- cannot be honoured for it at all. See
        /// <see cref="WithdrawCopy.NoBulk"/>, which says that to the
        /// contributor rather than leaving the affordance silently missing.
        /// </summary>
        public const string Withdraw = "withdraw";
    }

    public static class Events
    {
        public const string Snapshot = "snapshot";
        public const string QueueChanged = "queue_changed";
        public const string StatusChanged = "status_changed";
        public const string DigestDue = "digest_due";
        public const string ResyncRequired = "resync_required";

        /// <summary>
        /// Synthesized by the ABI, not the daemon, when more than 256 events
        /// were published between polls. It carries a skipped count and means
        /// the local view is stale: the correct response is a full refetch,
        /// not an incremental update.
        /// </summary>
        public const string Lagged = "lagged";
    }

    internal static readonly JsonSerializerOptions SerializerOptions = new()
    {
        PropertyNameCaseInsensitive = true,
    };
}

/// <summary>
/// The response envelope: exactly one of <see cref="Result"/> or
/// <see cref="Error"/> is present.
/// </summary>
public sealed class DaemonResponse
{
    [JsonPropertyName("id")]
    public ulong Id { get; set; }

    [JsonPropertyName("result")]
    public JsonElement? Result { get; set; }

    [JsonPropertyName("error")]
    public DaemonError? Error { get; set; }

    public bool IsError => Error is not null;

    /// <summary>
    /// Parses a raw response.
    ///
    /// Malformed JSON is turned into a synthetic error frame rather than an
    /// exception. Every caller of this already has to handle
    /// <see cref="DaemonError"/>, and giving them a second failure channel for
    /// what is the same condition -- the daemon did not answer usefully -- only
    /// multiplies the branches at each call site.
    /// </summary>
    public static DaemonResponse Parse(string json)
    {
        ArgumentNullException.ThrowIfNull(json);

        try
        {
            return JsonSerializer.Deserialize<DaemonResponse>(
                       json,
                       DaemonProtocol.SerializerOptions)
                   ?? MalformedFrame();
        }
        catch (JsonException)
        {
            return MalformedFrame();
        }
    }

    private static DaemonResponse MalformedFrame() => new()
    {
        Error = new DaemonError { Code = "unavailable", Message = "malformed-response" },
    };

    /// <summary>
    /// Deserializes <see cref="Result"/> into <typeparamref name="T"/>, or
    /// returns null when this is an error frame or the payload does not fit.
    /// </summary>
    public T? ResultAs<T>()
        where T : class
    {
        if (Result is null || IsError)
        {
            return null;
        }

        try
        {
            return Result.Value.Deserialize<T>(DaemonProtocol.SerializerOptions);
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>
/// A daemon error. Both fields are fixed labels by the repo's hash-only
/// discipline: no path, token, URL, or trace content appears here, so both are
/// safe to log and safe to show.
/// </summary>
public sealed class DaemonError
{
    [JsonPropertyName("code")]
    public string Code { get; set; } = string.Empty;

    [JsonPropertyName("message")]
    public string Message { get; set; } = string.Empty;
}

/// <summary>The <c>list_pending</c> payload.</summary>
public sealed class PendingList
{
    [JsonPropertyName("pending")]
    public List<QueueEntry> Pending { get; set; } = new();
}

/// <summary>
/// One queue entry, mirroring <c>entry_value</c> in ipc.rs.
///
/// Note what is absent: no file path and no transcript text. The entry names a
/// project and a size, and the content is reachable only by opening a preview.
/// Keep it that way -- a field added here because it was convenient is a field
/// that ends up in a log.
/// </summary>
public sealed class QueueEntry
{
    [JsonPropertyName("entry_id")]
    public string EntryId { get; set; } = string.Empty;

    [JsonPropertyName("session_hash")]
    public string? SessionHash { get; set; }

    [JsonPropertyName("source")]
    public string? Source { get; set; }

    [JsonPropertyName("project_id")]
    public string? ProjectId { get; set; }

    [JsonPropertyName("project_label")]
    public string? ProjectLabel { get; set; }

    [JsonPropertyName("size_bytes")]
    public long SizeBytes { get; set; }

    [JsonPropertyName("discovered_at")]
    public string? DiscoveredAt { get; set; }

    [JsonPropertyName("state")]
    public string? State { get; set; }

    /// <summary>
    /// Why the entry is in its current state, as a fixed label. Shown to the
    /// contributor verbatim; it is already written to be read.
    /// </summary>
    [JsonPropertyName("reason_label")]
    public string? ReasonLabel { get; set; }

    [JsonPropertyName("attempts")]
    public int Attempts { get; set; }

    [JsonPropertyName("retry_after")]
    public string? RetryAfter { get; set; }

    [JsonPropertyName("submission_id")]
    public string? SubmissionId { get; set; }
}

/// <summary>
/// The <c>hello</c> payload: the handshake that establishes the daemon speaks
/// a schema this client understands.
/// </summary>
public sealed class DaemonHello
{
    [JsonPropertyName("schema_version")]
    public string SchemaVersion { get; set; } = string.Empty;

    [JsonPropertyName("supported_versions")]
    public List<string> SupportedVersions { get; set; } = new();

    [JsonPropertyName("methods")]
    public List<string> Methods { get; set; } = new();

    [JsonPropertyName("events")]
    public List<string> Events { get; set; } = new();

    [JsonPropertyName("max_line_bytes")]
    public long MaxLineBytes { get; set; }

    /// <summary>
    /// Whether the daemon accepts the schema this client speaks.
    ///
    /// Checked against the daemon's <c>supported_versions</c> list rather than
    /// against its own <c>schema_version</c>: the daemon may run a newer
    /// schema than us and still support ours, which is the entire reason the
    /// list is on the wire.
    /// </summary>
    public bool AcceptsClientSchema =>
        SupportedVersions.Contains(DaemonProtocol.SchemaVersion, StringComparer.Ordinal);
}

/// <summary>
/// An event frame delivered to a <see cref="TcDaemon.Subscribe"/> handler.
/// </summary>
public sealed class DaemonEvent
{
    [JsonPropertyName("event")]
    public string Event { get; set; } = string.Empty;

    [JsonPropertyName("data")]
    public JsonElement? Data { get; set; }

    /// <summary>
    /// Parses an event frame, or returns null if it is not one. Callbacks
    /// arrive on a Rust thread where throwing is not an option, so this
    /// reports failure by value.
    /// </summary>
    public static DaemonEvent? Parse(string json)
    {
        try
        {
            var parsed = JsonSerializer.Deserialize<DaemonEvent>(
                json,
                DaemonProtocol.SerializerOptions);
            return string.IsNullOrEmpty(parsed?.Event) ? null : parsed;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// How many events the ABI dropped, for a <c>lagged</c> frame. Any nonzero
    /// answer means the local view must be refetched rather than patched.
    /// </summary>
    public int SkippedCount =>
        Data is { } data
        && data.ValueKind == JsonValueKind.Object
        && data.TryGetProperty("skipped", out JsonElement skipped)
        && skipped.TryGetInt32(out int value)
            ? value
            : 0;
}
