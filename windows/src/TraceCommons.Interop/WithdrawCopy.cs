using System;
using System.Collections.Generic;
using System.Globalization;

namespace TraceCommons.Interop;

/// <summary>
/// Withdrawal's words, and the one decision behind them.
///
/// This lives in the interop assembly rather than in a view model for the
/// same reason <see cref="ReadGate"/> does: it is a safety property of the
/// shell, not a presentation detail, and here it is exercised by tests on a
/// machine that cannot build WinUI at all.
///
/// Withdrawal is the one place in this product where a plausible-sounding
/// phrase becomes a false promise about erasure, so the three confirmation
/// bodies are NOT this shell's to write. They are fixed in
/// <c>docs/contributor-daemon-ipc-v1_1.md</c>'s "Canonical confirmation copy"
/// table, reproduced here word for word, and
/// <c>tests/TraceCommons.Interop.Tests/WithdrawCopyTests.cs</c> compares them
/// whole against that table so a paraphrase, a shortening or a "tightening"
/// fails the build. The Linux shell carries the identical constants in
/// <c>crates/trace-commons-contributor-gtk/src/copy.rs</c>; the two shells
/// must not diverge.
///
/// Five rules come with the table, and each is honoured somewhere below:
///
/// <list type="number">
/// <item>Never a generic "withdrawn" -- <see cref="ResultSentence"/> always
/// names what the tier that actually applied did.</item>
/// <item>Never claim more erasure than the tier achieved -- which is why
/// <see cref="Confirmation"/> shows an <c>accepted</c> trace BOTH commons
/// bodies rather than picking the gentler one.</item>
/// <item>Withdrawal does not reverse settled credit -- <see cref="CreditNote"/>,
/// and nothing here implies otherwise.</item>
/// <item><c>not_found</c> must not disclose which -- <see cref="NotFound"/>.</item>
/// <item>Bulk withdrawal spans tiers -- <see cref="NoBulk"/> says why this
/// shell does not offer it.</item>
/// </list>
///
/// <para><b>Why the confirmation cannot simply state the tier.</b> The server
/// computes <c>distribution_reach</c> DURING the withdrawal, from live export
/// membership. It arrives in the response, and the confirmation has to be
/// shown before that response exists. All this machine holds is the record's
/// <c>status</c>, so the confirmation is keyed on that instead -- see
/// <see cref="WithdrawStage"/>.</para>
/// </summary>
public static class WithdrawCopy
{
    /// <summary>
    /// <c>distribution_reach</c> as the server spells it.
    /// </summary>
    /// <remarks>
    /// Wire strings rather than a typed enum: this shell only ever looks a
    /// tier up to find its sentence, and an unrecognised one is reported as
    /// unrecognised (see <see cref="ResultSentence"/>) rather than failing to
    /// parse. A tier from a newer server must not be able to crash the one
    /// screen that reports what a deletion achieved.
    /// </remarks>
    public const string ReachNotDistributed = "not_distributed";

    public const string ReachCommonsNotDistributed = "commons_not_distributed";

    public const string ReachCommonsDistributed = "commons_distributed";

    /// <summary>Canonical copy for <c>not_distributed</c>, verbatim.</summary>
    public const string BodyNotDistributed =
        "This trace never entered the commons. Withdrawing deletes it. Nothing was distributed "
        + "and nothing needs recalling.";

    /// <summary>Canonical copy for <c>commons_not_distributed</c>, verbatim.</summary>
    public const string BodyCommonsNotDistributed =
        "This trace is in the commons but has not been included in any published export or "
        + "benchmark yet. Withdrawing deletes it and excludes it from everything published from "
        + "here on.";

    /// <summary>
    /// Canonical copy for <c>commons_distributed</c>, verbatim. The clause
    /// from "but copies" onward is the one sentence in this feature that must
    /// never be softened, shortened, or quietly dropped.
    /// </summary>
    public const string BodyCommonsDistributed =
        "This trace has already been included in a published export or benchmark. Withdrawing "
        + "deletes our copy and excludes it from everything published from here on, but copies "
        + "that have already been distributed cannot be recalled. Withdrawing does not undo that.";

    /// <summary>
    /// Credit is not clawed back, and this says only that -- nothing about
    /// how much, when it settles, or what it is worth.
    /// </summary>
    public const string CreditNote = "Credit already recorded stays.";

    public const string Question = "Withdraw this trace?";

    public const string Withdraw = "Withdraw";

    /// <summary>
    /// Used wherever the contributor is being asked to accept a limit rather
    /// than confirm an unambiguous outcome.
    /// </summary>
    public const string WithdrawAnyway = "Withdraw anyway";

    public const string Cancel = "Keep it";

    /// <summary>
    /// The row-level progress label while a withdrawal is in flight. Present
    /// tense, because nothing has happened yet.
    /// </summary>
    public const string Withdrawing = "Withdrawing…";

    /// <summary>
    /// What this window says before showing the two commons bodies it cannot
    /// choose between. Its job is to make clear that the choosing happens on
    /// the server, not here.
    /// </summary>
    public const string AmbiguityInTheCommons =
        "This trace is in the commons. Whether it has already gone into a published export or "
        + "benchmark is decided on the server, and this window cannot tell from here which of "
        + "these two applies:";

    public const string AmbiguityUnknown =
        "This window does not recognise what stage this trace reached, so it cannot rule out "
        + "the furthest one:";

    /// <summary>
    /// Withdrawal is authenticated by an account session, which this build
    /// has no way to obtain.
    /// </summary>
    /// <remarks>
    /// This is the failure contributors will actually hit, not an edge case:
    /// <c>daemon/withdraw.rs</c> answers <c>account-session-required</c>
    /// before ever attempting the call, always, because the daemon holds a
    /// device key and never an account session. So it is rendered as this
    /// whole explanatory sentence rather than as a bare label. It leads with
    /// the fact that nothing happened -- a contributor must not walk away
    /// from a failed withdrawal believing their trace was taken back.
    /// </remarks>
    public const string AccountSessionRequired =
        "Nothing was withdrawn and nothing was deleted. Withdrawal is an account-level act, so "
        + "it is authenticated by your Trace Commons account rather than by this device -- that "
        + "is what lets you withdraw a trace after losing the machine that sent it. This build "
        + "has no account sign-in yet, so it cannot make the request.";

    /// <summary>
    /// The daemon's label for "the server has no record of this submission
    /// for this account".
    /// </summary>
    /// <remarks>
    /// The server answers identically whether the submission belongs to
    /// someone else or does not exist at all, so that accounts cannot be
    /// enumerated, and this window must not undo that by guessing out loud.
    /// So this sentence says neither.
    /// </remarks>
    public const string NotFound =
        "Nothing was withdrawn and nothing was deleted. There is no trace with that id under "
        + "your account.";

    /// <summary>
    /// Why there is no "withdraw all of these" button, said where a
    /// contributor would look for one.
    /// </summary>
    /// <remarks>
    /// Rule 6 permits bulk only if the confirmation can say the selected
    /// traces may fall into different tiers and that some may already have
    /// been distributed. There is a second problem on top of that one, and it
    /// is the reason bulk is left out rather than worded around:
    /// <c>withdraw_bulk</c> reports only <c>withdrawn</c> and <c>failed</c>
    /// counts, so afterwards there is no per-trace tier to report and rule 1
    /// cannot be honoured at all.
    /// </remarks>
    public const string NoBulk =
        "There is no button here that withdraws all of them at once. The bulk call reports only "
        + "how many succeeded, never what happened to any one trace, and it chooses what to "
        + "withdraw from this machine's copy of your history, which can be out of date -- so it "
        + "could not tell you afterwards which of these had already been distributed. Withdraw "
        + "them one at a time below and each one tells you what it actually did.";

    /// <summary>
    /// The daemon labels that mean not-found.
    /// </summary>
    /// <remarks>
    /// Unreachable today -- <c>daemon/withdraw.rs</c> collapses every failure
    /// into <c>withdraw-failed</c> -- and handled anyway, because the day
    /// that label is passed through is not the day to be inventing this
    /// sentence.
    /// </remarks>
    public static readonly string[] NotFoundLabels =
    {
        "not-found",
        "not_found",
        "submission-not-found",
    };

    /// <summary>
    /// The label the daemon returns when it has no account session. Matched
    /// on the error's <c>message</c>, which is where
    /// <c>ERR_ACCOUNT_SESSION_REQUIRED</c> rides; the <c>code</c> is the
    /// generic <c>unavailable</c>.
    /// </summary>
    public const string AccountSessionRequiredLabel = "account-session-required";

    /// <summary>
    /// The canonical body for a tier, or null for a tier this build has never
    /// heard of.
    /// </summary>
    public static string? CanonicalBody(string? reach) => reach switch
    {
        ReachNotDistributed => BodyNotDistributed,
        ReachCommonsNotDistributed => BodyCommonsNotDistributed,
        ReachCommonsDistributed => BodyCommonsDistributed,
        _ => null,
    };

    /// <summary>
    /// The confirmation to show before the request, keyed on what this
    /// machine can honestly say about how far the trace got.
    /// </summary>
    public static WithdrawConfirmation Confirmation(WithdrawStage stage) => stage switch
    {
        WithdrawStage.NotInTheCommons => new WithdrawConfirmation(
            Question,
            ambiguity: null,
            bodies: new[] { BodyNotDistributed },
            gravest: null,
            CreditNote,
            confirmLabel: Withdraw),

        // Rule 2. `accepted` may resolve to either commons tier and this
        // window cannot tell which, so showing only the gentler body would be
        // claiming more erasure than may have been achieved.
        WithdrawStage.InTheCommons => new WithdrawConfirmation(
            Question,
            AmbiguityInTheCommons,
            bodies: new[] { BodyCommonsNotDistributed, BodyCommonsDistributed },
            gravest: 1,
            CreditNote,
            confirmLabel: WithdrawAnyway),

        _ => new WithdrawConfirmation(
            Question,
            AmbiguityUnknown,
            bodies: new[] { BodyCommonsDistributed },
            gravest: 0,
            CreditNote,
            confirmLabel: WithdrawAnyway),
    };

    /// <summary>
    /// What actually happened, from the tier the server applied.
    /// </summary>
    /// <remarks>
    /// Never a generic "withdrawn": the canonical body for the tier that
    /// applied is what says which of the three outcomes this was. A tier this
    /// build does not know is not smoothed into the mild answer either -- the
    /// withdrawal happened; what cannot be stated is how far the trace had
    /// travelled, so the furthest tier is not ruled out.
    /// </remarks>
    public static string ResultSentence(string? reach)
    {
        string? body = CanonicalBody(reach);
        return body is null
            ? "Withdrawn, but the server did not report which of the three tiers applied, so "
              + "this window cannot tell you whether it had already been included in a published "
              + "export or benchmark. If it had, copies that have already been distributed "
              + "cannot be recalled."
            : "Withdrawn. " + body;
    }

    /// <summary>
    /// A failure, named. Every branch opens by saying nothing happened.
    /// </summary>
    /// <remarks>
    /// The label is the daemon's fixed, content-free error message, which by
    /// contract is never a path, a token, or a response body -- so printing it
    /// cannot leak one, and leaving it out would make two different failures
    /// indistinguishable to whoever is asked to help.
    /// </remarks>
    public static string FailureSentence(string? label)
    {
        if (string.Equals(label, AccountSessionRequiredLabel, StringComparison.Ordinal))
        {
            return AccountSessionRequired;
        }

        if (label is not null && Array.IndexOf(NotFoundLabels, label) >= 0)
        {
            return NotFound;
        }

        return string.Format(
            CultureInfo.CurrentCulture,
            "Nothing was withdrawn and nothing was deleted. The request did not go through "
            + "({0}). You can try again.",
            string.IsNullOrEmpty(label) ? "no reason given" : label);
    }

    /// <summary>
    /// Whether a record gets a withdraw button.
    /// </summary>
    /// <remarks>
    /// An already-withdrawn record does not: there is nothing left to
    /// withdraw, and it stays on the list reading as withdrawn rather than
    /// being dropped or re-labelled. A record carrying no
    /// <c>submission_id</c> does not either -- <c>withdraw</c> takes exactly
    /// that id and nothing else, so the button would have nothing to send and
    /// would fail for a reason the contributor could do nothing about.
    /// </remarks>
    public static bool OffersWithdrawal(string? status, string? submissionId) =>
        !string.Equals(status, HistoryCopy.StatusWithdrawn, StringComparison.Ordinal)
        && !string.IsNullOrWhiteSpace(submissionId);
}

/// <summary>
/// What this machine can honestly say about how far a trace got, read off the
/// history record's <c>status</c>.
///
/// Not the server's tier: this is the weaker thing the client knows before it
/// asks.
/// </summary>
public enum WithdrawStage
{
    /// <summary>
    /// <c>submitted</c> or <c>quarantined</c>. <c>not_distributed</c>,
    /// exactly -- that is the server's own rule.
    /// </summary>
    NotInTheCommons,

    /// <summary>
    /// <c>accepted</c>. One of the two commons tiers, and not knowable which.
    /// </summary>
    InTheCommons,

    /// <summary>
    /// Any other status this build does not recognise. Treated as the worst
    /// case, because the furthest reach cannot be ruled out.
    /// </summary>
    Unknown,
}

/// <summary>
/// The confirmation as parts rather than one blob, so the dialog can weight
/// the body carrying the cannot-be-recalled clause and leave the rest as
/// ordinary body copy.
/// </summary>
public sealed class WithdrawConfirmation
{
    internal WithdrawConfirmation(
        string question,
        string? ambiguity,
        IReadOnlyList<string> bodies,
        int? gravest,
        string credit,
        string confirmLabel)
    {
        Question = question;
        Ambiguity = ambiguity;
        Bodies = bodies;
        Gravest = gravest;
        Credit = credit;
        ConfirmLabel = confirmLabel;
    }

    public string Question { get; }

    /// <summary>
    /// Present only where the tier is ambiguous: says so, in this shell's own
    /// words, before the canonical bodies it cannot choose between.
    /// </summary>
    public string? Ambiguity { get; }

    /// <summary>
    /// Canonical bodies that may apply, in order. One when the tier is known,
    /// two when it is not.
    /// </summary>
    public IReadOnlyList<string> Bodies { get; }

    /// <summary>
    /// Index into <see cref="Bodies"/> of the one carrying the
    /// cannot-be-recalled clause, so the dialog can weight it. Null when none
    /// does.
    /// </summary>
    public int? Gravest { get; }

    public string Credit { get; }

    /// <summary>
    /// "Withdraw" where the outcome is unambiguous, "Withdraw anyway" where
    /// the contributor is being asked to accept a limit.
    /// </summary>
    public string ConfirmLabel { get; }
}

/// <summary>
/// Reads a local history status as the stage this machine may speak about.
/// </summary>
public static class WithdrawStageExtensions
{
    public static WithdrawStage StageOf(string? status) => status switch
    {
        HistoryCopy.StatusSubmitted or HistoryCopy.StatusQuarantined
            => WithdrawStage.NotInTheCommons,
        HistoryCopy.StatusAccepted => WithdrawStage.InTheCommons,
        _ => WithdrawStage.Unknown,
    };
}
