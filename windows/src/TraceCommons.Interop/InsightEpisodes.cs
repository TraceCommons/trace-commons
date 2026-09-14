using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

public sealed record EpisodeMember(string SnapshotId, string SourceDigest);
public sealed record EpisodeEvidence(string Id, string SourceDigest);
public sealed record EpisodeAssessment(string Category, string Outcome, DateTimeOffset RecordedAt,
    ulong MembershipRevision, string MembersDigest);
public sealed record InsightEpisode(uint SchemaVersion, string Id, ulong Revision, ulong MembershipRevision,
    DateTimeOffset CreatedAt, DateTimeOffset UpdatedAt, IReadOnlyList<EpisodeMember> Members,
    EpisodeAssessment? Assessment);
public sealed record EpisodeListItem(InsightEpisode Episode, IReadOnlyList<string> OverlappingEpisodeIds);
public sealed record EpisodeMemberEvidence(string SnapshotId, string SourceFormat, DateTimeOffset AnalyzedAt,
    IReadOnlyList<EpisodeEvidence> Evidence, InsightEvidence Projection);
public sealed record EpisodeOverlap(string SnapshotId, IReadOnlyList<string> EpisodeIds);
public sealed record EpisodeDetail(InsightEpisode Episode, IReadOnlyList<EpisodeMemberEvidence> Members,
    IReadOnlyList<EpisodeOverlap> Overlap, DateTimeOffset ResolvedAt);

/// <summary>Strict typed decoding for the shared episode service. It derives no episode metrics.</summary>
public static class InsightEpisodeResponses
{
    private static readonly HashSet<string> Categories = new(StringComparer.Ordinal) {
        "refactor", "tests", "docs", "debugging", "other", "unknown"
    };
    private static readonly HashSet<string> Outcomes = new(StringComparer.Ordinal) {
        "accepted", "partial", "rejected", "unknown"
    };

    public static IReadOnlyList<EpisodeListItem> DecodeList(JsonElement response)
    {
        RequireType(response, "episode_list");
        return response.GetProperty("episodes").EnumerateArray().Select(item => new EpisodeListItem(
            DecodeEpisode(item.GetProperty("episode")), Strings(item.GetProperty("overlapping_episode_ids")))).ToArray();
    }

    public static InsightEpisode DecodeMutation(JsonElement response, string type)
    {
        RequireType(response, type);
        return DecodeEpisode(response.GetProperty("episode"));
    }

    public static EpisodeDetail DecodeDetail(JsonElement response)
    {
        RequireType(response, "episode_explain");
        var detail = response.GetProperty("detail");
        var episode = DecodeEpisode(detail.GetProperty("episode"));
        var members = detail.GetProperty("members").EnumerateArray().Select(insight => {
            string id = Required(insight, "id");
            var evidence = insight.GetProperty("report").GetProperty("evidence").EnumerateArray()
                .Select(item => new EpisodeEvidence(Required(item, "id"), Required(item, "source_digest"))).ToArray();
            return new EpisodeMemberEvidence(id, Required(insight, "source_format"),
                insight.GetProperty("analyzed_at").GetDateTimeOffset(), evidence, InsightEvidence.Decode(insight));
        }).ToArray();
        var overlap = detail.GetProperty("overlap").EnumerateArray()
            .Select(item => new EpisodeOverlap(Required(item, "snapshot_id"), Strings(item.GetProperty("episode_ids")))).ToArray();
        if (!episode.Members.Select(member => member.SnapshotId).SequenceEqual(members.Select(member => member.SnapshotId)))
            throw Invalid();
        for (int index = 0; index < members.Length; index++)
            if (!members[index].Evidence.Any(evidence => evidence.SourceDigest == episode.Members[index].SourceDigest))
                throw Invalid();
        return new EpisodeDetail(episode, members, overlap, detail.GetProperty("resolved_at").GetDateTimeOffset());
    }

    private static InsightEpisode DecodeEpisode(JsonElement value)
    {
        uint schema = value.GetProperty("schema_version").GetUInt32();
        string id = Required(value, "id");
        ulong revision = value.GetProperty("revision").GetUInt64();
        ulong membershipRevision = value.GetProperty("membership_revision").GetUInt64();
        DateTimeOffset created = value.GetProperty("created_at").GetDateTimeOffset();
        DateTimeOffset updated = value.GetProperty("updated_at").GetDateTimeOffset();
        if (schema != 1 || !Guid.TryParseExact(id, "D", out _) || revision == 0 || membershipRevision == 0 ||
            membershipRevision > revision || updated < created || Required(value, "provenance") != "user_selected_whole_snapshots")
            throw Invalid();
        var members = value.GetProperty("members").EnumerateArray()
            .Select(member => new EpisodeMember(Required(member, "snapshot_id"), Required(member, "source_digest"))).ToArray();
        if (members.Length == 0 || members.Select(member => member.SnapshotId).Distinct(StringComparer.Ordinal).Count() != members.Length)
            throw Invalid();
        EpisodeAssessment? assessment = null;
        var manual = value.GetProperty("manual_assessment");
        if (manual.ValueKind != JsonValueKind.Null)
        {
            string category = Required(manual, "category");
            string outcome = Required(manual, "outcome");
            assessment = new EpisodeAssessment(category, outcome, manual.GetProperty("recorded_at").GetDateTimeOffset(),
                manual.GetProperty("membership_revision").GetUInt64(), Required(manual, "members_digest"));
            if (!Categories.Contains(category) || !Outcomes.Contains(outcome) || Required(manual, "provenance") != "user_reported" ||
                assessment.MembershipRevision != membershipRevision)
                throw Invalid();
        }
        return new InsightEpisode(schema, id, revision, membershipRevision, created, updated, members, assessment);
    }

    private static IReadOnlyList<string> Strings(JsonElement value) => value.EnumerateArray()
        .Select(item => item.GetString() ?? throw Invalid()).ToArray();
    private static string Required(JsonElement value, string name) =>
        value.GetProperty(name).GetString() ?? throw Invalid();
    private static void RequireType(JsonElement response, string expected)
    {
        if (Required(response, "type") != expected) throw Invalid();
    }
    private static InvalidOperationException Invalid() => new("insights-response-invalid");
}

public sealed class InsightsServiceException : InvalidOperationException
{
    public string Code { get; }
    public InsightsServiceException(string code) : base(code) => Code = code;
}
