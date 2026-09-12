using System;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>Optional additions keep pre-v3 snapshots readable as unknown, without guessed evidence.</summary>
public sealed class InsightEvidence
{
    public DeclaredModelObservations? ModelObservations { get; init; }
    public OutcomeLink[] OutcomeLinks { get; init; } = Array.Empty<OutcomeLink>();
    public static InsightEvidence Decode(JsonElement insight) => insight.Deserialize<InsightEvidence>(
        new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower })
        ?? throw new InvalidOperationException("insights-response-invalid");
}
public sealed class DeclaredModelObservations
{
    public required uint SchemaVersion { get; init; }
    public required string Scope { get; init; }
    public required string SourceFormat { get; init; }
    public required string SourceDigest { get; init; }
    public required string Coordinates { get; init; }
    public required ulong RecordCount { get; init; }
    public required ulong CandidateRecords { get; init; }
    public required ulong ValidDeclarations { get; init; }
    public required ulong MissingDeclarations { get; init; }
    public required ulong InvalidDeclarations { get; init; }
    public required ulong OmittedDeclarations { get; init; }
    public required bool ModelLabelsOmitted { get; init; }
    public required bool MixedDeclaredModels { get; init; }
    public required string[] DeclaredModels { get; init; }
    public required ModelDeclaration[] Declarations { get; init; }
}
public sealed class ModelDeclaration
{
    public required string Model { get; init; }
    public required ulong RecordIndex { get; init; }
    public required string Kind { get; init; }
}
public sealed class OutcomeLink
{
    public required string Id { get; init; }
    public required string SourceDigest { get; init; }
    public required DateTimeOffset LinkedAt { get; init; }
    public required string Provenance { get; init; }
    public required OutcomeObservation Evidence { get; init; }
}
[JsonPolymorphic(TypeDiscriminatorPropertyName = "type")]
[JsonDerivedType(typeof(GitCommitObservation), "git_commit")]
[JsonDerivedType(typeof(TestReportObservation), "test_report")]
public abstract class OutcomeObservation { }
public sealed class GitCommitObservation : OutcomeObservation
{
    public required GitCommitEvidence Evidence { get; init; }
}
public sealed class TestReportObservation : OutcomeObservation
{
    public required TestReportEvidence Evidence { get; init; }
}
public sealed class GitCommitEvidence
{
    public required string RepositoryPathDigest { get; init; }
    public required string ObjectId { get; init; }
    public required string TreeId { get; init; }
    public required string[] ParentIds { get; init; }
    public required DateTimeOffset InspectedAt { get; init; }
    public required string Provenance { get; init; }
}
public sealed class TestReportEvidence
{
    public required uint SchemaVersion { get; init; }
    public required string Runner { get; init; }
    public required ulong Passed { get; init; }
    public required ulong Failed { get; init; }
    public required ulong Skipped { get; init; }
    public required DateTimeOffset ObservedAt { get; init; }
    public string? CommitId { get; init; }
    public required string ArtifactDigest { get; init; }
    public required DateTimeOffset ImportedAt { get; init; }
    public required string Provenance { get; init; }
}
