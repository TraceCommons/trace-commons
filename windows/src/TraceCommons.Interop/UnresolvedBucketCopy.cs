namespace TraceCommons.Interop;

/// <summary>
/// The bucket holding sessions whose project the daemon cannot name: its
/// words, and the one mode it must never be offered.
///
/// Shared rather than owned by either surface. Onboarding screen 5 and Settings
/// both list projects, both show this row, and
/// <c>docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md</c>
/// ("The unresolvable bucket in Settings") requires the same words on both,
/// because it is one fact stated twice. Every shell got this wrong the same way
/// -- the raw slug with no explanation -- and a second copy of the strings is
/// how one surface gets quietly reworded later.
/// </summary>
public static class UnresolvedBucketCopy
{
    /// <summary>
    /// The human name for the bucket.
    ///
    /// The wire carries the slug <c>unknown-project</c> as this row's
    /// <c>project_label</c>, because the daemon's <c>project_label_for</c>
    /// deliberately returns the constant rather than degrade to something that
    /// might carry a path. A slug is not a project name, so it is not shown.
    /// </summary>
    public const string Label = "Sessions with no project";

    /// <summary>
    /// Why it can never be armed.
    ///
    /// A statement of what the daemon does, not an apology. The bucket exists
    /// because a cwd with no usable final segment has no label but itself, and
    /// <c>project_label</c> reaches <c>daemon-audit.jsonl</c>, notification text
    /// and <c>HistoryRecord</c> -- so naming it would write a full local path
    /// into all three. Not being armable is the protective half of that, and
    /// nothing in it is a contributor's to fix.
    /// </summary>
    public const string Note =
        "Trace Commons can't tell which folder these ran in, so they can never be "
        + "contributed automatically. You'll always be asked.";

    /// <summary>
    /// Whether a shell may offer <c>auto_upload</c> for a row.
    ///
    /// False for the bucket. The daemon refuses that mode for it in two
    /// independent places, so a control offering it would invite a contributor
    /// to believe they had armed something that cannot be armed, and the
    /// refusal would be silent. Omitting the choice is the honest answer;
    /// offering it disabled still puts an arming affordance on a row that has
    /// none.
    ///
    /// <c>ignore</c> and ask-first remain available either way: the bucket can
    /// be silenced even though it cannot be armed.
    /// </summary>
    public static bool MayOfferAutoUpload(bool isUnresolvedBucket) => !isUnresolvedBucket;

    /// <summary>
    /// The modes a shell may put in front of a contributor for a row, in the
    /// order a picker should show them.
    ///
    /// Expressed here rather than in a view because it is a correctness rule
    /// about what the daemon will accept, not a presentation choice, and
    /// because a view is the one place in this codebase nothing can execute.
    /// </summary>
    public static string[] OfferableModes(bool isUnresolvedBucket) =>
        isUnresolvedBucket
            ? new[] { "ask", "ignore" }
            : new[] { "ask", "auto_upload", "ignore" };
}
