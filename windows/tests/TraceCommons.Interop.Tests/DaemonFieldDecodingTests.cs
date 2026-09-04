using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The daemon gained a project path, a session path, distinct redaction
/// counts, and a project id on history records. Every one must tolerate
/// absence: this app ships separately from the daemon and routinely runs
/// against an older one.
/// </summary>
public class DaemonFieldDecodingTests
{
    /// <summary>
    /// The same deserializer the app uses, so a field that decodes here
    /// decodes at run time too. <see cref="PreviewSummary.Parse"/> and
    /// <see cref="DaemonResponse.Parse"/> both route through these options,
    /// and a test with its own would be testing a second configuration
    /// nothing ships.
    /// </summary>
    private static T Parse<T>(string json) =>
        JsonSerializer.Deserialize<T>(json, DaemonProtocol.SerializerOptions)!;

    [Fact]
    public void AQueueEntryDecodesProjectAndSessionPaths()
    {
        var entry = Parse<QueueEntry>("""
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":"~/code/repo/crates/inner",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """);

        Assert.Equal("~/code/repo", entry.ProjectPath);
        Assert.Equal("~/code/repo/crates/inner", entry.SessionPath);
    }

    [Fact]
    public void AQueueEntryFromAnOlderDaemonHasNoPaths()
    {
        var entry = Parse<QueueEntry>("""
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """);

        Assert.Equal("", entry.ProjectPath);
        Assert.Null(entry.SessionPath);
    }

    /// <summary>
    /// Null, not a repeat of the project path, when the session ran at the
    /// root. The daemon sends null there on purpose so a row can draw the
    /// line only when it says something.
    /// </summary>
    [Fact]
    public void ASessionThatRanAtTheProjectRootReportsNoSessionPath()
    {
        var entry = Parse<QueueEntry>("""
        {"entry_id":"e1","project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":null,
         "size_bytes":12,"state":"pending","attempts":0}
        """);

        Assert.Equal("~/code/repo", entry.ProjectPath);
        Assert.Null(entry.SessionPath);
    }

    [Fact]
    public void APreviewSummaryDecodesDistinctCounts()
    {
        PreviewSummary? summary = PreviewSummary.Parse("""
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "redactions_distinct":{"local_path":12},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """);

        Assert.NotNull(summary);
        Assert.Equal(185, summary!.Redactions["local_path"]);
        Assert.Equal(12, summary.RedactionsDistinct["local_path"]);
    }

    [Fact]
    public void APreviewSummaryFromAnOlderDaemonHasNoDistinctCounts()
    {
        PreviewSummary? summary = PreviewSummary.Parse("""
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """);

        Assert.NotNull(summary);
        Assert.Empty(summary!.RedactionsDistinct);
    }

    /// <summary>
    /// A secrets-only session, which is the ordinary case and not an edge one.
    /// </summary>
    /// <remarks>
    /// Distinct counts come from the placeholder map, and only
    /// <c>local_path</c> and <c>private_email</c> mint a placeholder. So a
    /// session whose removals are all secrets carries occurrences and an EMPTY
    /// distinct map, and no <c>(N distinct)</c> suffix may appear anywhere.
    /// Printing <c>(0 distinct)</c> beside a non-zero occurrence count would
    /// read as "nothing was removed", which is the one direction this figure
    /// must not fail in.
    ///
    /// <c>PreviewSummary</c> always carries the key, as <c>{}</c>;
    /// <c>PrivacyMetadata</c> skips it when empty. Either way the rule is the
    /// same: no entry means no distinct count is AVAILABLE, never that it is
    /// zero.
    /// </remarks>
    [Fact]
    public void ASecretsOnlySummaryRendersNoDistinctSuffix()
    {
        PreviewSummary? summary = PreviewSummary.Parse("""
        {"redactions":{"secret":1,"secret:openai_api_key":1},
         "redactions_distinct":{}}
        """);

        Assert.NotNull(summary);
        Assert.Empty(summary!.RedactionsDistinct);
        Assert.DoesNotContain("distinct", summary.ScrubbingFound, StringComparison.Ordinal);
        Assert.DoesNotContain("distinct", summary.RedactionReceipt, StringComparison.Ordinal);
        Assert.Equal("1 secret  \u00b7  1 secret:openai api key", summary.ScrubbingFound);
    }

    [Fact]
    public void AHistoryRecordDecodesItsProjectId()
    {
        var record = Parse<HistoryRecord>("""
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z","project_id":"proj_abc",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """);

        Assert.Equal("proj_abc", record.ProjectId);
    }

    [Fact]
    public void AHistoryRecordFromBeforeTheUpgradeHasNoProjectId()
    {
        var record = Parse<HistoryRecord>("""
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """);

        Assert.Equal("", record.ProjectId);
    }

    /// <summary>
    /// The row history resolves its folder paths from. A record's project id
    /// is matched against these; a record whose project the daemon no longer
    /// knows renders with its label alone.
    /// </summary>
    [Fact]
    public void AProjectRowDecodesItsPath()
    {
        var row = Parse<ProjectSetting>("""
        {"project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","mode":"ask","configured":true,
         "is_unresolved_bucket":false}
        """);

        Assert.Equal("~/code/repo", row.ProjectPath);
    }

    [Fact]
    public void AProjectRowFromAnOlderDaemonHasNoPath()
    {
        var row = Parse<ProjectSetting>("""
        {"project_id":"proj_abc","project_label":"repo","mode":"ask",
         "configured":true,"is_unresolved_bucket":false}
        """);

        Assert.Equal("", row.ProjectPath);
    }
}
