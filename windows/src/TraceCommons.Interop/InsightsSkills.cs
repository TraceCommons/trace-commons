using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The Skills tab's state (<c>InsightsSkillsModel</c>,
/// macos/Sources/TraceCommonsApp/InsightsSkillsModel.swift): a confirmed
/// daemon snapshot. Reads and follow writes share one lane (<see cref="Busy"/>),
/// so an older read never lands over a newer follow confirmation. Every word
/// is the snapshot's <c>copy</c>, or the core's Insights table before the
/// first answer.
/// </summary>
public sealed class InsightsSkillsModel : INotifyPropertyChanged
{
    public const string Method = "insights_skills";
    public const string FollowMethod = "insights_skills_follow";

    /// <summary>The idle poll: five minutes (InsightsSkillsModel.swift:39).</summary>
    public static readonly TimeSpan IdlePollInterval = TimeSpan.FromMinutes(5);

    /// <summary>The poll while the daemon says it is refreshing: two seconds (:271).</summary>
    public static readonly TimeSpan RefreshingPollInterval = TimeSpan.FromSeconds(2);

    public delegate Task<SkillsSnapshot> Reader(bool refresh);
    public delegate Task<SkillsSnapshot> Writer(string publisherId, bool followed);

    private readonly Reader? _reader;
    private readonly Writer? _writer;
    private readonly Func<string, Task<SkillDefinition>>? _definitionReader;
    private readonly Func<string, Task<SkillEvidence>>? _evidenceReader;
    private readonly Func<string, string?> _fallbackCopy;
    private readonly TimeSpan _idlePoll;
    private long _definitionGeneration;
    private long _evidenceGeneration;
    private string? _selectedDefinition;
    private string? _selectedEvidence;

    public InsightsSkillsModel(
        Reader? reader = null, Writer? writer = null,
        Func<string, Task<SkillDefinition>>? definitionReader = null,
        Func<string, Task<SkillEvidence>>? evidenceReader = null,
        Func<string, string?>? copy = null, TimeSpan? idlePollInterval = null)
    {
        _reader = reader;
        _writer = writer;
        _definitionReader = definitionReader;
        _evidenceReader = evidenceReader;
        _fallbackCopy = copy ?? (_ => null);
        _idlePoll = idlePollInterval ?? IdlePollInterval;
    }

    /// <summary>The daemon, as the Mac's <c>LiveDaemonClient</c> asks it (LiveDaemonClient.swift:322-338).</summary>
    public static InsightsSkillsModel ForDaemon(Func<string, string, Task<DaemonResponse>> call, Func<string, string?> copy) =>
        new(
            async refresh => Decode(await call(Method, JsonSerializer.Serialize(new { refresh })).ConfigureAwait(true), SkillsSnapshot.Parse),
            async (publisher, followed) => Decode(
                await call(FollowMethod, JsonSerializer.Serialize(new { publisher_id = publisher, followed })).ConfigureAwait(true),
                SkillsSnapshot.Parse),
            async id => Decode(await call(Method, JsonSerializer.Serialize(new { skill_id = id })).ConfigureAwait(true), SkillDefinition.Parse),
            reference => EvidenceAsync(call, reference),
            copy);

    public const string DrillMethod = "insights_session_drill";

    /// <summary>
    /// One session's evidence: an <c>os.</c> reference from <c>insights_skills</c>,
    /// any other from <c>insights_session_drill</c> with the zone's offset
    /// (LiveDaemonClient.swift:324-330).
    /// </summary>
    public static async Task<SkillEvidence> EvidenceAsync(Func<string, string, Task<DaemonResponse>> call, string reference)
    {
        string parameters = reference.StartsWith("os.", StringComparison.Ordinal)
            ? JsonSerializer.Serialize(new { session_ref = reference })
            : JsonSerializer.Serialize(new { session_ref = reference, tz = (int)TimeZoneInfo.Local.GetUtcOffset(DateTimeOffset.UtcNow).TotalSeconds });
        string method = reference.StartsWith("os.", StringComparison.Ordinal) ? Method : DrillMethod;
        return Decode(await call(method, parameters).ConfigureAwait(true), SkillEvidence.Parse);
    }

    private static T Decode<T>(DaemonResponse response, Func<JsonElement, T?> parse) where T : class
    {
        if (response.Error is { } error) throw new InvalidOperationException(error.Code);
        if (response.Result is not { } result || parse(result) is not { } value) throw new InvalidOperationException("undecodable");
        return value;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public SkillsSnapshot? Shown { get; private set; }
    public bool Busy { get; private set; }
    public string? FailureKey { get; private set; }
    public string? PendingFollowPublisherId { get; private set; }
    public string? FollowFailurePublisherId { get; private set; }
    public SkillDefinition? Definition { get; private set; }
    public SkillEvidence? Evidence { get; private set; }
    public bool DefinitionFailure { get; private set; }
    public bool EvidenceFailure { get; private set; }

    /// <summary>Changes whenever the details are dropped, so a view re-reads the one it shows.</summary>
    public long DetailEpoch { get; private set; }

    /// <summary>How many reads have been sent; tests pin that a hidden tab sends none.</summary>
    public int ReadCount { get; private set; }

    public bool CanFollow => _writer is not null && !Busy && Shown is { Enabled: true, Readable: true };

    public string Text(string key, IReadOnlyDictionary<string, string>? values = null)
    {
        string result = Shown?.Copy.TryGetValue(key, out string? text) == true ? text! : _fallbackCopy(key) ?? string.Empty;
        if (values is not null)
        {
            foreach ((string name, string value) in values) result = result.Replace("{" + name + "}", value, StringComparison.Ordinal);
        }

        return result;
    }

    public string Text(string key, string name, string value) => Text(key, new Dictionary<string, string> { [name] = value });

    public string AgentLabel(string agent)
    {
        string label = Text("skills_agent_" + agent);
        return label.Length == 0 ? agent : label;
    }

    public string ScopeLabel(string scope)
    {
        string label = Text("skills_scope_" + scope);
        return label.Length == 0 ? scope : label;
    }

    public string PopulationLabel(string population) =>
        Text("skills_population_" + (population is "direct" or "delegated" ? population : "unknown"));

    /// <summary>The Mac's <c>.abbreviated</c> date with a short time; the value itself when it does not parse.</summary>
    public static string Timestamp(string value) =>
        SkillsDates.Parse(value) is { } date ? date.ToLocalTime().ToString("MMM d, yyyy, h:mm tt", CultureInfo.CurrentCulture) : value;

    public void InvalidateDetails()
    {
        _definitionGeneration++;
        _evidenceGeneration++;
        _selectedDefinition = null;
        _selectedEvidence = null;
        Definition = null;
        Evidence = null;
        DefinitionFailure = false;
        // As on the Mac (InsightsSkillsModel.swift:49): a closed evidence
        // panel reads as unavailable until a new read starts.
        EvidenceFailure = true;
        DetailEpoch++;
        Changed();
    }

    /// <summary>
    /// Reads one installed skill's definition, once per id until the details
    /// are dropped (the Mac's <c>.task(id: "\(id):\(detailEpoch)")</c>):
    /// asking again for the id already asked for sends nothing.
    /// </summary>
    public async Task LoadDefinitionAsync(string id)
    {
        if (_selectedDefinition == id) return;
        long generation = ++_definitionGeneration;
        _selectedDefinition = id;
        Definition = null;
        DefinitionFailure = false;
        Changed();
        if (_definitionReader is null)
        {
            DefinitionFailure = true;
            Changed();
            return;
        }

        try
        {
            SkillDefinition result = await _definitionReader(id).ConfigureAwait(true);
            if (_selectedDefinition != id || _definitionGeneration != generation) return;
            Definition = result;
            DefinitionFailure = result.Status != "available";
        }
        catch (Exception)
        {
            if (_selectedDefinition == id && _definitionGeneration == generation) DefinitionFailure = true;
        }

        Changed();
    }

    public async Task LoadEvidenceAsync(string reference)
    {
        long generation = ++_evidenceGeneration;
        _selectedEvidence = reference;
        Evidence = null;
        EvidenceFailure = false;
        Changed();
        if (_evidenceReader is null)
        {
            EvidenceFailure = true;
            Changed();
            return;
        }

        try
        {
            SkillEvidence result = await _evidenceReader(reference).ConfigureAwait(true);
            if (_selectedEvidence != reference || _evidenceGeneration != generation) return;
            Evidence = result;
            EvidenceFailure = !result.Enabled || result.Readable != true || (result.Session is null && result.SkillUsage is null);
        }
        catch (Exception)
        {
            if (_selectedEvidence == reference && _evidenceGeneration == generation) EvidenceFailure = true;
        }

        Changed();
    }

    public async Task LoadAsync(bool refresh = false, CancellationToken cancellation = default)
    {
        if (Busy || cancellation.IsCancellationRequested) return;
        if (_reader is null)
        {
            FailureKey = "skills_unavailable";
            Changed();
            return;
        }

        Busy = true;
        Changed();
        try
        {
            ReadCount++;
            SkillsSnapshot snapshot = await _reader(refresh).ConfigureAwait(true);
            if (cancellation.IsCancellationRequested) return;
            InvalidateDetails();
            FollowFailurePublisherId = null;
            if (snapshot.Readable)
            {
                Shown = snapshot;
                FailureKey = snapshot.Error is null ? null : "skills_refresh_failed";
            }
            else
            {
                Shown = null;
                FailureKey = "skills_unavailable";
            }
        }
        catch (Exception)
        {
            if (!cancellation.IsCancellationRequested) FailureKey = Shown is null ? "skills_unavailable" : "skills_refresh_failed";
        }
        finally
        {
            Busy = false;
            Changed();
        }
    }

    public async Task SetFollowAsync(string publisherId, bool followed)
    {
        if (!CanFollow || _writer is null) return;
        Busy = true;
        PendingFollowPublisherId = publisherId;
        FollowFailurePublisherId = null;
        if (FailureKey == "skills_follow_failed") FailureKey = null;
        Changed();
        try
        {
            SkillsSnapshot snapshot = await _writer(publisherId, followed).ConfigureAwait(true);
            // An authoritative loss of access revokes the saved display too.
            if (!snapshot.Readable || !snapshot.Enabled)
            {
                InvalidateDetails();
                Shown = snapshot.Readable ? snapshot : null;
                FollowFailurePublisherId = publisherId;
                FailureKey = "skills_follow_failed";
                return;
            }

            // A readable reply alone does not confirm the requested change.
            if (snapshot.FollowedPublishers.Contains(publisherId) != followed)
            {
                FollowFailurePublisherId = publisherId;
                FailureKey = "skills_follow_failed";
                return;
            }

            InvalidateDetails();
            Shown = snapshot;
            // Public catalog refresh errors can coexist with a saved follow.
            FailureKey = snapshot.Error is null ? null : "skills_refresh_failed";
        }
        catch (Exception)
        {
            FollowFailurePublisherId = publisherId;
            FailureKey = "skills_follow_failed";
        }
        finally
        {
            PendingFollowPublisherId = null;
            Busy = false;
            Changed();
        }
    }

    /// <summary>
    /// Reads now and then on the Mac's clock until cancelled (<c>observe()</c>):
    /// <see cref="IdlePollInterval"/> idle, <see cref="RefreshingPollInterval"/>
    /// while the daemon refreshes, and <see cref="RetryDelay"/> after a
    /// failure. Automatic reads never force a catalog refresh.
    /// </summary>
    public async Task ObserveAsync(CancellationToken cancellation, Func<TimeSpan, CancellationToken, Task>? delay = null)
    {
        delay ??= Task.Delay;
        await ReadWhenReadyAsync(cancellation).ConfigureAwait(true);
        int failures = 0;
        while (!cancellation.IsCancellationRequested && _reader is not null)
        {
            TimeSpan wait = ObservationDelay(failures);
            failures = FailureKey is null ? 0 : Math.Min(failures + 1, 4);
            try
            {
                await delay(wait, cancellation).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            await ReadWhenReadyAsync(cancellation).ConfigureAwait(true);
        }
    }

    public TimeSpan ObservationDelay(int failureAttempt)
    {
        // A transport failure can keep a snapshot whose refresh flag is stale.
        if (FailureKey is not null) return RetryDelay(failureAttempt);
        return Shown?.Refreshing == true ? RefreshingPollInterval : _idlePoll;
    }

    /// <summary>2 s doubling to at most 30 s (<c>retryDelay(attempt:)</c>).</summary>
    public static TimeSpan RetryDelay(int attempt)
    {
        double seconds = 2 * (1 << Math.Min(Math.Max(attempt, 0), 4));
        return TimeSpan.FromSeconds(Math.Min(seconds, 30));
    }

    private async Task ReadWhenReadyAsync(CancellationToken cancellation)
    {
        while (Busy && !cancellation.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(10, cancellation).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }

        if (!cancellation.IsCancellationRequested) await LoadAsync(false, cancellation).ConfigureAwait(true);
    }

    public IReadOnlyList<InstalledSkill> InventorySkills(
        SkillsSnapshot snapshot, string query, string agent = "", string availability = "", string sort = "name", int days = 30)
    {
        query = query.Trim();
        return snapshot.Inventory.Skills.Where(skill =>
                (query.Length == 0 || SkillsText.Contains(skill.Name, query) || SkillsText.Contains(skill.Description, query))
                && (agent.Length == 0 || skill.Agent == agent)
                && (availability.Length == 0
                    || (snapshot.Inventory.Details.TryGetValue(skill.Id, out SkillInstallationDetails? d) ? d.Availability : "unknown") == availability))
            .OrderBy(skill => skill, Comparer<InstalledSkill>.Create((lhs, rhs) =>
            {
                SkillsUsageRow? left = Usage(lhs, snapshot), right = Usage(rhs, snapshot);
                if (sort == "sessions")
                {
                    int a = left?.Windows.FirstOrDefault(w => w.Days == days)?.Sessions ?? 0;
                    int b = right?.Windows.FirstOrDefault(w => w.Days == days)?.Sessions ?? 0;
                    if (a != b) return b.CompareTo(a);
                }

                if (sort == "recent")
                {
                    string a = UsageRows(lhs, snapshot).Select(r => r.LastSeen).Where(s => s is not null).Max() ?? string.Empty;
                    string b = UsageRows(rhs, snapshot).Select(r => r.LastSeen).Where(s => s is not null).Max() ?? string.Empty;
                    int order = string.CompareOrdinal(b, a);
                    if (order != 0) return order;
                }

                return SkillsText.StandardCompare(lhs.Name, rhs.Name);
            }))
            .ToArray();
    }

    private static readonly string[] SharedAgents = { "codex", "claude_code", "opencode" };

    /// <summary>
    /// A shared root's skill can be read by several agents: their
    /// observations stay apart, since capped all-time references cannot
    /// remove duplicates from window totals.
    /// </summary>
    public IReadOnlyList<SkillsUsageRow> UsageRows(InstalledSkill skill, SkillsSnapshot snapshot)
    {
        if (HasUsageCollision(skill, snapshot)) return Array.Empty<SkillsUsageRow>();
        if (skill.Agent == "shared")
        {
            var supported = snapshot.Usage.Coverage
                .Where(c => SharedAgents.Contains(c.Agent) && c.Status != "unsupported").Select(c => c.Agent).ToHashSet();
            return snapshot.Usage.Skills.Where(r => r.Name == skill.Name && supported.Contains(r.Agent))
                .OrderBy(r => r.Identity, StringComparer.Ordinal).ToArray();
        }

        return snapshot.Usage.Skills.Where(r => r.Agent == skill.Agent && r.Name == skill.Name).ToArray();
    }

    private static bool HasUsageCollision(InstalledSkill skill, SkillsSnapshot snapshot) =>
        snapshot.Inventory.Skills.Count(s =>
            s.Name == skill.Name && (skill.Agent == "shared" || s.Agent == "shared" || s.Agent == skill.Agent)) != 1;

    /// <summary>A single row's total, only when its meaning is unambiguous.</summary>
    public SkillsUsageRow? Usage(InstalledSkill skill, SkillsSnapshot snapshot)
    {
        IReadOnlyList<SkillsUsageRow> rows = UsageRows(skill, snapshot);
        return rows.Count == 1 ? rows[0] : null;
    }

    public string UsageState(InstalledSkill skill, SkillsSnapshot snapshot)
    {
        if (!snapshot.Usage.Enabled) return "skills_usage_unavailable";
        var coverage = snapshot.Usage.Coverage
            .Where(c => skill.Agent == "shared" ? SharedAgents.Contains(c.Agent) : c.Agent == skill.Agent).ToArray();
        if (!coverage.Any(c => c.Status != "unsupported")) return "skills_usage_unavailable";
        if (HasUsageCollision(skill, snapshot)) return "skills_usage_unattributed";
        if (coverage.Any(c => c.Status is "partial" or "pending" or "unknown") || snapshot.Usage.Partial
            || snapshot.Usage.LastAnalysisAt is null) return "skills_usage_pending";
        return "skills_no_usage";
    }

    public IReadOnlyList<CatalogSkill> CatalogSkills(SkillsSnapshot snapshot, string query)
    {
        query = query.Trim();
        if (query.Length == 0) return snapshot.Catalog.Skills;
        var publishers = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (SkillsPublisher p in snapshot.Catalog.Publishers) publishers.TryAdd(p.Id, p.Name);
        return snapshot.Catalog.Skills.Where(skill =>
                new[] { skill.Name, skill.Description, skill.Repository, publishers.TryGetValue(skill.PublisherId, out string? n) ? n : string.Empty }
                    .Any(field => SkillsText.Contains(field, query)))
            .ToArray();
    }

    /// <summary>The daemon already bounds evidence; never used to rebuild totals.</summary>
    public static IReadOnlyList<SkillsUsageEvidence> EvidenceLinks(SkillsUsageRow row)
    {
        IEnumerable<SkillsUsageEvidence> links = row.Evidence.Count == 0
            ? row.SessionRefs.Select(r => new SkillsUsageEvidence(r, null))
            : row.Evidence;
        var seen = new HashSet<string>(StringComparer.Ordinal);
        return links.Where(l => seen.Add(l.SessionRef)).ToArray();
    }

    public string UsageSummary(SkillsUsageRow row, IReadOnlyList<SkillsUsageCoverage> coverage)
    {
        string? observation = coverage.FirstOrDefault(c => c.Agent == row.Agent)?.Status;
        var figures = new List<string>();
        if (observation == "explicit_invocations" || row.Invocations > 0)
        {
            figures.Add(row.Invocations > 0
                ? Text("skills_invocations", "count", row.Invocations.ToString(CultureInfo.InvariantCulture))
                : Text("skills_no_invocations"));
        }

        if (observation == "loads_only" || row.Loads > 0)
        {
            figures.Add(Text("skills_loads", "count", row.Loads.ToString(CultureInfo.InvariantCulture)));
        }

        figures.Add(Text("skills_sessions", "count", row.Sessions.ToString(CultureInfo.InvariantCulture)));
        return string.Join(" · ", figures.Where(f => f.Length > 0));
    }

    /// <summary>A shared display name never establishes an installed skill's source.</summary>
    public static CatalogSkill? CatalogMatch(InstalledSkill installed, SkillsSnapshot snapshot) =>
        installed.Source is not { } source
            ? null
            : snapshot.Catalog.Skills.FirstOrDefault(s => s.Repository == source.Repository && s.Path == source.Path);

    private void Changed() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
}

/// <summary>Content identity groups copies across agents, never a display name alone (<c>InsightsSkillsIndex</c>).</summary>
public static class InsightsSkillsIndex
{
    public sealed record Group(IReadOnlyList<InstalledSkill> Installations)
    {
        public string Id => Installations[0].Id;
        public InstalledSkill Representative => Installations[0];
        public IReadOnlyList<string> Agents => Installations.Select(i => i.Agent).Distinct().OrderBy(a => a, StringComparer.Ordinal).ToArray();

        public InstalledSkill DefinitionInstallation(SkillsSnapshot snapshot) =>
            Installations.FirstOrDefault(i => snapshot.Inventory.Details.TryGetValue(i.Id, out SkillInstallationDetails? d)
                && d.Availability == "available" && d.DefinitionStatus == "valid") ?? Representative;
    }

    public static IReadOnlyList<Group> Groups(IEnumerable<InstalledSkill> installations, IReadOnlyList<string>? orderedIds = null)
    {
        var result = new List<Group>();
        foreach (IGrouping<string, InstalledSkill> bucket in installations.GroupBy(skill =>
                 {
                     string hash = skill.Sha256.ToLowerInvariant();
                     // Missing or invalid hashes cannot establish identical definitions.
                     return hash.Length == 64 && hash.All(Uri.IsHexDigit) ? "content:" + skill.Name + "\0" + hash : "id:" + skill.Id;
                 }))
        {
            var copies = bucket.ToArray();
            int origins = copies.Where(c => c.Source is not null).Select(c => c.Source!.Repository + "\0" + c.Source.Path).Distinct().Count();
            if (origins > 1)
            {
                // Equal files can be published independently: conflicting
                // known origins stay apart, and an unknown one picks no side.
                foreach (IGrouping<string, InstalledSkill> part in copies.GroupBy(s =>
                             s.Source is { } src ? src.Repository + "\0" + src.Path : "unknown:" + s.Id))
                {
                    result.Add(new Group(part.OrderBy(s => s.Id, StringComparer.Ordinal).ToArray()));
                }
            }
            else
            {
                result.Add(new Group(copies.OrderBy(s => s.Id, StringComparer.Ordinal).ToArray()));
            }
        }

        if (orderedIds is not null)
        {
            var positions = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < orderedIds.Count; i++) positions[orderedIds[i]] = i;
            int Position(Group g) => g.Installations.Select(s => positions.TryGetValue(s.Id, out int p) ? p : int.MaxValue).Min();
            return result.Where(g => Position(g) != int.MaxValue)
                .OrderBy(Position).ThenBy(g => g.Id, StringComparer.Ordinal).ToArray();
        }

        return result.OrderBy(g => g.Id, StringComparer.Ordinal).ToArray();
    }

    public static IReadOnlyList<SkillsUsageRow> UsageRows(Group group, SkillsSnapshot snapshot, InsightsSkillsModel model) =>
        group.Installations.SelectMany(i => model.UsageRows(i, snapshot))
            .GroupBy(r => r.Identity).Select(g => g.First())
            .OrderBy(r => r.Identity, StringComparer.Ordinal).ToArray();
}

/// <summary>Text matching and ordering as Foundation's <c>localizedStandard*</c> does it, near enough for names.</summary>
internal static class SkillsText
{
    public static bool Contains(string text, string query) =>
        CultureInfo.CurrentCulture.CompareInfo.IndexOf(text, query,
            CompareOptions.IgnoreCase | CompareOptions.IgnoreNonSpace | CompareOptions.IgnoreWidth) >= 0;

    public static int StandardCompare(string a, string b) =>
        string.Compare(a, b, CultureInfo.CurrentCulture, CompareOptions.IgnoreCase);
}

/// <summary>ISO 8601 instants and local days, as the Mac reads them.</summary>
public static class SkillsDates
{
    /// <summary>An ISO 8601 instant with or without fractional seconds; null otherwise.</summary>
    public static DateTimeOffset? Parse(string value) =>
        value.Length >= 20 && (value.EndsWith('Z') || value[^6] is '+' or '-')
        && DateTimeOffset.TryParse(value, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out DateTimeOffset date)
            ? date
            : null;

    /// <summary>A <c>yyyy-MM-dd</c> day that round-trips exactly, at the start of that day in <paramref name="zone"/>.</summary>
    public static DateTimeOffset? Day(string day, TimeZoneInfo zone)
    {
        if (!DateTime.TryParseExact(day, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out DateTime date)
            || date.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture) != day)
        {
            return null;
        }

        return StartOfDay(date, zone);
    }

    public static DateTimeOffset StartOfDay(DateTimeOffset now, TimeZoneInfo zone)
    {
        DateTime local = TimeZoneInfo.ConvertTime(now, zone).Date;
        return StartOfDay(local, zone);
    }

    private static DateTimeOffset StartOfDay(DateTime localDate, TimeZoneInfo zone)
    {
        DateTime unspecified = DateTime.SpecifyKind(localDate, DateTimeKind.Unspecified);
        // A day that starts in a gap starts at the gap's end.
        while (zone.IsInvalidTime(unspecified)) unspecified = unspecified.AddMinutes(30);
        return new DateTimeOffset(unspecified, zone.GetUtcOffset(unspecified));
    }

    public static DateTimeOffset AddDays(DateTimeOffset day, int days, TimeZoneInfo zone) =>
        StartOfDay(TimeZoneInfo.ConvertTime(day, zone).Date.AddDays(days), zone);

    public static string Format(DateTimeOffset date, TimeZoneInfo zone) =>
        TimeZoneInfo.ConvertTime(date, zone).ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);

    /// <summary>The start of the ISO week (Monday) holding <paramref name="now"/>.</summary>
    public static DateTimeOffset IsoWeekStart(DateTimeOffset now, TimeZoneInfo zone)
    {
        DateTime local = TimeZoneInfo.ConvertTime(now, zone).Date;
        int back = ((int)local.DayOfWeek + 6) % 7;
        return StartOfDay(local.AddDays(-back), zone);
    }
}
