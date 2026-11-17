using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>What the contributor decided, handed back to the queue window.</summary>
public sealed class PreviewDecision
{
    public const string ApprovalFailedMessage = "That couldn't be approved just now. Nothing has been sent.";

    public const string ApprovalUnconfirmedMessage = "Approval could not be confirmed. Check the trace status.";

    private PreviewDecision(PreviewOutcome outcome, ApprovalHold? hold, string? message)
    {
        Outcome = outcome;
        Hold = hold;
        Message = message;
    }

    public PreviewOutcome Outcome { get; }

    /// <summary>
    /// The daemon's hold on an approval, or null when it granted none. Null
    /// means no undo may be offered, which the queue window says plainly
    /// rather than drawing a button that would fail.
    /// </summary>
    public ApprovalHold? Hold { get; }

    /// <summary>A fixed sentence for a decision the daemon refused.</summary>
    public string? Message { get; }

    public static PreviewDecision Approved(ApprovalHold? hold) =>
        hold is null
            ? Failed(ApprovalUnconfirmedMessage)
            : hold.Approved == 0
                ? Failed(hold.Toast.Line)
                : new(PreviewOutcome.Approved, hold, null);

    public static PreviewDecision Dismissed() => new(PreviewOutcome.Dismissed, null, null);

    public static PreviewDecision Failed(string message) =>
        new(PreviewOutcome.Failed, null, message);
}

public enum PreviewOutcome
{
    Approved,
    Dismissed,
    Failed,
}
