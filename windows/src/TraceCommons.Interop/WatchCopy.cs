using System;
using System.Security.Cryptography;
using System.Text;

namespace TraceCommons.Interop;

/// <summary>
/// Onboarding screen 5's words, and the rule for recognising the bucket that
/// holds sessions with no resolvable project.
///
/// In the interop assembly rather than a view model for the reason
/// <see cref="SessionRootsCopy"/> gives: this is the screen that decides which
/// of a contributor's repositories are eligible to leave the machine, so it is
/// a safety property of the shell, and here it is exercised by tests on a
/// machine that cannot build WinUI at all.
///
/// Every string below is TRANSCRIBED from
/// <c>docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md</c>,
/// "### 5. What to watch". That section carried no copy until 2026-08-19, so
/// this screen shipped in all three shells as a bare title over an unlabelled
/// list. The words are now specified precisely so the three shells describe one
/// decision the same way -- do not reword them here alone.
/// </summary>
public static class WatchCopy
{
    /// <summary>The screen's heading.</summary>
    public const string Title = "What to watch";

    /// <summary>
    /// The subtitle. States the DEFAULT before the exception, on purpose: the
    /// default is what happens to a contributor who reads nothing and clicks
    /// Continue, which is most of them.
    /// </summary>
    public const string Subtitle =
        "Every project starts at ask-first: you see each session before anything is sent. "
        + "Ignore a project to leave it out entirely.";

    /// <summary>The eyebrow over the list. Rendered uppercase by the style.</summary>
    public const string Section = "Projects";

    /// <summary>
    /// The per-row state for a project that has not been ignored. This is the
    /// vocabulary Settings already uses for the same mode: two screens setting
    /// one field must not name it two ways.
    /// </summary>
    public const string AskMeFirst = "Ask me first";

    /// <summary>
    /// The state after <c>Ignore</c>. Echoes the button that produced it rather
    /// than introducing a third name for the mode.
    /// </summary>
    public const string Ignored = "Ignored";

    /// <summary>Shown when the daemon reports no projects at all.</summary>
    public const string Empty =
        "No projects yet. Sessions you run later will appear here, and in Settings.";

    /// <summary>
    /// The human name for the unresolvable bucket.
    ///
    /// The wire carries the slug <c>unknown-project</c> as this row's
    /// <c>project_label</c>, because the daemon's <c>project_label_for</c>
    /// deliberately returns the constant rather than degrade to something that
    /// might carry a path. A slug is not a project name, so it is not shown.
    /// </summary>
    public const string UnknownLabel = "Sessions with no project";

    /// <summary>
    /// Why that bucket can never be armed.
    ///
    /// Phrased as a CONSEQUENCE, not a fault. The bucket exists because a cwd
    /// with no usable final segment has no label but itself, and
    /// <c>project_label</c> reaches <c>daemon-audit.jsonl</c>, notification
    /// text and <c>HistoryRecord</c> -- so naming it would write a full local
    /// path into all three. Not being armable is the protective half of that,
    /// and nothing in it is a contributor's to fix.
    ///
    /// It REPLACES the state line rather than adding a third: "you'll always be
    /// asked" already says what <see cref="AskMeFirst"/> says.
    /// </summary>
    public const string UnknownNote =
        "Trace Commons can't tell which folder these ran in, so they can never be "
        + "contributed automatically. You'll always be asked.";

    /// <summary>
    /// The daemon's key for the unresolvable bucket, from
    /// <c>policy.rs</c>'s <c>UNKNOWN_PROJECT_KEY</c>.
    /// </summary>
    internal const string UnknownProjectKey = "unknown-project";

    private const string ProjectIdPrefix = "proj_";

    /// <summary>Hex characters of SHA-256 carried in an opaque project id.</summary>
    private const int ProjectIdHexChars = 16;

    /// <summary>
    /// The opaque id the daemon mints for the unresolvable bucket.
    ///
    /// Derived rather than hardcoded, mirroring <c>policy.rs</c>'s
    /// <c>project_id_for</c>: SHA-256 of the key, first 16 hex characters,
    /// <c>proj_</c> prefix. Recognition goes through the ID and never the
    /// label, because the id is what the row IS while the label is only what it
    /// displays -- and a shell that matched on the label would break the moment
    /// the label became a translated string.
    ///
    /// DUPLICATION, STATED PLAINLY: this re-implements a derivation that lives
    /// in Rust. The GTK shell has no such copy because it links the crate and
    /// calls <c>project_id_for</c> directly; a shell reaching the daemon over
    /// the C ABI cannot. <c>AnUnresolvableBucketIsRecognisedByItsDaemonMintedId</c>
    /// pins the value the daemon actually produces, but a C# test cannot notice
    /// if the Rust side changes its algorithm. The durable fix is for
    /// <c>list_projects</c> to mark the row explicitly; until it does, this is
    /// the only way a C ABI client can tell.
    /// </summary>
    public static string UnknownProjectId { get; } = DeriveProjectId(UnknownProjectKey);

    /// <summary>
    /// True when <paramref name="projectId"/> is the unresolvable bucket.
    /// </summary>
    public static bool IsUnresolvable(string? projectId) =>
        string.Equals(projectId, UnknownProjectId, StringComparison.Ordinal);

    /// <summary>
    /// What to show as a row's name: the human label for the unresolvable
    /// bucket, the daemon's label otherwise.
    /// </summary>
    public static string LabelFor(string? projectId, string? projectLabel)
    {
        if (IsUnresolvable(projectId))
        {
            return UnknownLabel;
        }

        return string.IsNullOrWhiteSpace(projectLabel) ? UnknownLabel : projectLabel;
    }

    /// <summary>
    /// The line beneath a row's name: the note for the unresolvable bucket,
    /// otherwise the mode. The note replaces the state rather than joining it.
    /// </summary>
    public static string SubLineFor(string? projectId, string? mode)
    {
        if (IsUnresolvable(projectId))
        {
            return UnknownNote;
        }

        return string.Equals(mode, "ignore", StringComparison.Ordinal) ? Ignored : AskMeFirst;
    }

    private static string DeriveProjectId(string projectKey)
    {
        byte[] digest = SHA256.HashData(Encoding.UTF8.GetBytes(projectKey));
        var hex = new StringBuilder(ProjectIdPrefix, ProjectIdPrefix.Length + ProjectIdHexChars);
        for (int nibble = 0; nibble < ProjectIdHexChars; nibble++)
        {
            byte b = digest[nibble / 2];
            int value = (nibble % 2 == 0) ? (b >> 4) : (b & 0x0f);
            hex.Append(value.ToString("x", System.Globalization.CultureInfo.InvariantCulture));
        }

        return hex.ToString();
    }
}
