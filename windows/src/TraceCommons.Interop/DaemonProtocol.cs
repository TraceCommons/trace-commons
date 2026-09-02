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

        /// <summary>
        /// Drains in-flight uploads and parks the queue, bounded by
        /// <c>timeout_secs</c>. Answered only by the async dispatcher, which
        /// is what <c>tc_call</c> reaches: it routes through
        /// <c>ipc::handle_local</c> to <c>handle_request_async</c>, so the
        /// socket path's "quiesce-requires-async" refusal never applies here.
        /// </summary>
        public const string Quiesce = "quiesce";

        // Onboarding. Every one of these was already in the daemon's pinned
        // METHODS array before this app could call any of them: the gap on
        // Windows was never protocol, only that nothing here asked.
        public const string Enroll = "enroll";
        public const string ConsentOptions = "consent_options";
        public const string SetConsentScopes = "set_consent_scopes";
        public const string GetSettings = "get_settings";
        public const string SetSettings = "set_settings";
        public const string ListProjects = "list_projects";
        public const string SetProjectMode = "set_project_mode";
        public const string ListAudit = "list_audit";
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

        // The public roster profile. Like the two blocks above, every one of
        // these was already in the daemon's pinned METHODS array before this
        // app could call any of them -- the gap on Windows was never protocol,
        // only that nothing here asked. Adding a name that is NOT in that
        // array would break
        // `hello_advertises_exactly_the_documented_method_set`.
        public const string GetPublicProfile = "get_public_profile";

        /// <summary>
        /// Claims or re-publishes a handle.
        /// </summary>
        /// <remarks>
        /// The whole profile, every time: the server upserts with
        /// <c>bio = excluded.bio</c>, so there is no partial update to
        /// express and the daemon refuses an omitted <c>bio</c> rather than
        /// guessing. <see cref="PublicProfileRequest.Serialize"/> is what
        /// builds the parameters, and it always sends the key.
        /// </remarks>
        public const string SetPublicProfile = "set_public_profile";

        public const string ClearPublicProfile = "clear_public_profile";

        /// <summary>
        /// Asks the daemon's bounded preview scheduler for a card's preview
        /// and returns immediately -- never the pipeline itself. See
        /// <c>docs/superpowers/specs/2026-08-20-preview-scheduler-design.md</c>
        /// and <c>PreviewCardOutcome</c>, which decodes what comes back.
        /// </summary>
        public const string PreviewRequest = "preview_request";

        /// <summary>
        /// Replaces the daemon's idea of which entries are on screen,
        /// wholesale. Decides preview build ORDER, never membership -- an
        /// entry that scrolls away keeps its place in the queue until
        /// <see cref="PreviewCancel"/> drops it.
        /// </summary>
        public const string PreviewVisible = "preview_visible";

        /// <summary>
        /// Drops a queued preview, or discards a running one's result.
        /// <c>dropped: false</c> is a no-op, not an error.
        /// </summary>
        public const string PreviewCancel = "preview_cancel";
    }

    public static class Events
    {
        public const string Snapshot = "snapshot";
        public const string QueueChanged = "queue_changed";
        public const string StatusChanged = "status_changed";
        public const string DigestDue = "digest_due";
        public const string ResyncRequired = "resync_required";

        /// <summary>
        /// A scheduled preview finished and was delivered. Carries the same
        /// object <c>preview_request</c>'s result would -- see
        /// <see cref="PreviewCardOutcome"/> -- and is published only for a
        /// build that was actually queued or running; a cache hit answers
        /// <c>preview_request</c> directly and publishes no event.
        /// </summary>
        public const string PreviewReady = "preview_ready";

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

    /// <summary>Which ADAPTER produced this. Not always the tool the
    /// contributor used -- see <see cref="DeclaredSource"/>.</summary>
    [JsonPropertyName("source")]
    public string? Source { get; set; }

    /// <summary>
    /// What the transcript declares itself to be, when the daemon knew it.
    ///
    /// An imported Antigravity conversation is stored as a trajectory file,
    /// so <see cref="Source"/> says <c>trajectory</c>: the storage format,
    /// and not the word the contributor typed to collect it. Null for every
    /// native adapter, and for a daemon predating the field.
    /// </summary>
    [JsonPropertyName("declared_source")]
    public string? DeclaredSource { get; set; }

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

    /// <summary>
    /// How many delegated subagent transcripts this entry's session covers.
    /// A Claude Code conversation is not one file, and a card standing for
    /// 114 of them has to be able to say so: what is being consented to is
    /// the whole conversation, and its extent is part of the description.
    /// </summary>
    [JsonPropertyName("subagent_count")]
    public int SubagentCount { get; set; }

    /// <summary>
    /// How many delegated subagent transcripts were left out because the
    /// conversation exceeded the source's raw byte budget. The contract
    /// makes surfacing a non-zero value a <b>must</b>: the difference
    /// between a trace the contributor knows was trimmed and one that
    /// silently arrives partial is the whole point of showing it.
    ///
    /// Zero on every source with no such structure, and on any daemon
    /// predating the field -- which is the only safe reading of silence.
    /// </summary>
    [JsonPropertyName("subagents_dropped")]
    public int SubagentsDropped { get; set; }
}

/// <summary>
/// The <c>status</c> payload -- "the tray's whole world in one object", as
/// <c>ipc.rs</c> puts it.
/// </summary>
/// <remarks>
/// Only the fields the tray needs are modelled. Note what is deliberately not
/// read here even though the daemon sends it: <c>tenant_id</c> and
/// <c>consent_scopes</c> are identity and policy, and neither belongs on a
/// surface that renders a tooltip.
/// </remarks>
public sealed class DaemonStatus
{
    [JsonPropertyName("logged_in")]
    public bool LoggedIn { get; set; }

    [JsonPropertyName("paused")]
    public bool Paused { get; set; }

    /// <summary>Decisions owed: the daemon's own count of pending entries.</summary>
    [JsonPropertyName("queue_depth")]
    public int QueueDepth { get; set; }

    /// <summary>The scopes currently granted by this contributor.</summary>
    [JsonPropertyName("consent_scopes")]
    public List<string> ConsentScopes { get; set; } = new();

    [JsonPropertyName("health")]
    public DaemonHealth? Health { get; set; }

    /// <summary>
    /// The daily volume caps, and how much already-approved work they are
    /// holding back.
    /// </summary>
    /// <remarks>
    /// Read this independently of <see cref="Health"/>. The daemon does set
    /// a <c>daily-cap-reached</c> label when a cap refuses an upload, but
    /// that label is last in its precedence order, so any other condition
    /// takes the single <c>last_error_label</c> slot and the cap becomes
    /// invisible. A contributor watched fourteen approved traces sit still
    /// for an evening while the window reported a full queue and nothing
    /// else. Null from a daemon that predates the field.
    /// </remarks>
    [JsonPropertyName("daily_budget")]
    public DailyBudget? DailyBudget { get; set; }

    /// <summary>
    /// Whether approved traces are waiting on the daily budget. False when
    /// the daemon said nothing about it.
    /// </summary>
    public bool BudgetIsBlocking => DailyBudget?.Blocked == true;

    /// <summary>
    /// Whether there is nothing to report.
    /// </summary>
    /// <remarks>
    /// Health is expressed as the label of the last error and when it
    /// started, so "healthy" is the absence of a label rather than a flag.
    /// Reading it as the absence means a label this client has never heard of
    /// still counts as unhealthy, which is the safe direction: an unknown
    /// problem should not render as fine.
    /// </remarks>
    public bool IsHealthy => string.IsNullOrEmpty(Health?.LastErrorLabel);
}

/// <summary>
/// The health sub-object. A fixed label and a timestamp; never a path, a URL
/// or a message from a server.
/// </summary>
public sealed class DaemonHealth
{
    [JsonPropertyName("last_error_label")]
    public string? LastErrorLabel { get; set; }

    [JsonPropertyName("since")]
    public string? Since { get; set; }
}

/// <summary>
/// <c>status.daily_budget</c>: today's volume caps and what they are holding.
/// Counts and one timestamp; nothing identifying can appear here.
/// </summary>
public sealed class DailyBudget
{
    [JsonPropertyName("bytes_today")]
    public long BytesToday { get; set; }

    [JsonPropertyName("max_bytes_per_day")]
    public long MaxBytesPerDay { get; set; }

    [JsonPropertyName("bytes_remaining")]
    public long BytesRemaining { get; set; }

    [JsonPropertyName("uploads_today")]
    public int UploadsToday { get; set; }

    [JsonPropertyName("max_uploads_per_day")]
    public int MaxUploadsPerDay { get; set; }

    [JsonPropertyName("uploads_remaining")]
    public int UploadsRemaining { get; set; }

    /// <summary>
    /// When the counters zero, as the daemon reported it. Derived from its
    /// own UTC day bucket, so it is a fact and may be stated -- unlike
    /// "tomorrow", which is wrong for most of the world.
    /// </summary>
    [JsonPropertyName("resets_at")]
    public string? ResetsAt { get; set; }

    /// <summary>
    /// Whether at least one approved trace cannot go out before the reset.
    /// False when the budget is spent but nothing is waiting on it: there is
    /// no one to tell in that case.
    /// </summary>
    [JsonPropertyName("blocked")]
    public bool Blocked { get; set; }

    /// <summary>
    /// How many approved traces will not go out today. Counts everything
    /// behind the first entry that does not fit, not only the entries that
    /// individually overflow: the upload pass stops rather than skipping
    /// past, so a small trace queued behind a large one waits too.
    /// </summary>
    [JsonPropertyName("blocked_entries")]
    public int BlockedEntries { get; set; }

    [JsonPropertyName("blocked_bytes")]
    public long BlockedBytes { get; set; }

    /// <summary>
    /// <see cref="ResetsAt"/> parsed, or null when the daemon sent nothing
    /// usable. A copy path that cannot parse it must say less, not guess.
    /// </summary>
    public DateTimeOffset? ResetsAtUtc =>
        DateTimeOffset.TryParse(
            ResetsAt,
            System.Globalization.CultureInfo.InvariantCulture,
            System.Globalization.DateTimeStyles.AdjustToUniversal
                | System.Globalization.DateTimeStyles.AssumeUniversal,
            out var parsed)
            ? parsed
            : null;
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
    public int SkippedCount => IntField("skipped");

    /// <summary>
    /// How many decisions are owed, for a <c>digest_due</c> frame.
    /// </summary>
    /// <remarks>
    /// Zero on a frame that carries no count, which reads downstream as
    /// "nothing to say" and suppresses the notification. That is the safe
    /// direction: a malformed frame must not produce an interruption claiming
    /// an unknown number of sessions.
    /// </remarks>
    public int PendingCount => IntField("pending");

    /// <summary>
    /// How many sessions were contributed without being asked about since the
    /// last digest, for a <c>digest_due</c> frame.
    /// </summary>
    /// <remarks>
    /// Zero on a frame that carries no count, including every frame from a
    /// daemon predating this field. That degrades the digest to the
    /// waiting-only one that shipped before rather than to a wrong number.
    /// An armed project never queues anything, so this is the only count that
    /// is ever nonzero for a contributor who armed everything.
    /// </remarks>
    public int ContributedCount => IntField("contributed");

    /// <summary>
    /// Pending credit carried by those contributions. Pending, never earned:
    /// settlement is off on every deployment shipped so far.
    /// </summary>
    public double CreditPending =>
        Data is { } data
        && data.ValueKind == JsonValueKind.Object
        && data.TryGetProperty("credit_pending", out JsonElement credit)
        && credit.TryGetDouble(out double value)
            ? value
            : 0;

    /// <summary>
    /// The project labels those contributions came from. Labels only: the
    /// daemon has already reduced them from paths, and these go straight into
    /// notification text that Windows may persist in its notification centre.
    /// </summary>
    public IReadOnlyList<string> ContributedProjects
    {
        get
        {
            if (Data is not { } data
                || data.ValueKind != JsonValueKind.Object
                || !data.TryGetProperty("contributed_projects", out JsonElement projects)
                || projects.ValueKind != JsonValueKind.Array)
            {
                return Array.Empty<string>();
            }

            var labels = new List<string>();
            foreach (JsonElement item in projects.EnumerateArray())
            {
                if (item.ValueKind == JsonValueKind.String
                    && item.GetString() is { Length: > 0 } label)
                {
                    labels.Add(label);
                }
            }

            return labels;
        }
    }

    /// <summary>
    /// The decoded payload of a <see cref="DaemonProtocol.Events.PreviewReady"/>
    /// frame, or null for any other event or a payload that does not fit.
    /// </summary>
    public PreviewCardOutcome? PreviewOutcome =>
        Data is { } data ? PreviewCardOutcome.Parse(data) : null;

    private int IntField(string name) =>
        Data is { } data
        && data.ValueKind == JsonValueKind.Object
        && data.TryGetProperty(name, out JsonElement field)
        && field.TryGetInt32(out int value)
            ? value
            : 0;
}
