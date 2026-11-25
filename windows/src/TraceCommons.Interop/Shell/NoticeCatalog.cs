using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The groups the Mac's notices fall into (MonitorNotices.swift:138-160).
/// </summary>
public enum NoticeGroup
{
    /// <summary>About this app or this account, on every tab (<c>appItems</c>).</summary>
    App,
    /// <summary>What waits on an answer about the traces (<c>tracesOffers</c>).</summary>
    TracesOffers,
    /// <summary>History's own offers (<c>historyOffers</c>): the verdicts offer.</summary>
    HistoryOffers,
    /// <summary>What is happening to the traces now (<c>tracesNow</c>).</summary>
    TracesNow,
    /// <summary>The Private AI tab's own (<c>privateAIItems</c>).</summary>
    PrivateAI,
    /// <summary>A failed or finished action, in front of the rest (<c>actionItems</c>).</summary>
    Action,

    /// <summary>
    /// The traces being contributed, kept or dismissed on Traces and History.
    /// </summary>
    Transfers,

    /// <summary>An approval that can still be undone, shown in front while the hold is live.</summary>
    ApprovalUndo,
}

/// <summary>
/// What a card's X does (MonitorNotices.swift:26-34).
/// </summary>
public enum NoticeClose
{
    /// <summary>No X: the card goes only when its cause does.</summary>
    None,
    /// <summary>
    /// The card had a button whose only effect was to put it away; the X does
    /// that and the button is gone.
    /// </summary>
    PutsAway,
    /// <summary>
    /// Answering is its own decision, so the answer buttons stay and the X
    /// only closes the card until the window is opened again.
    /// </summary>
    ForNow,
    /// <summary>
    /// The X answers as the card's decline does, named with the decline's
    /// word, and the decline button stays (MonitorNotices.swift:752-761:
    /// the offer is asked once, so closing it is an answer).
    /// </summary>
    Declines,
}

/// <summary>Which word names a card's X (<see cref="NoticeCatalog.CloseWordFor"/>).</summary>
public enum CloseWord
{
    /// <summary>The core's <c>close</c>.</summary>
    Close,
    /// <summary>The offer's own decline word.</summary>
    Decline,
    /// <summary>The legacy offer's own dismissal word.</summary>
    LegacyDismiss,
    /// <summary><c>close_notice_still_sends</c>: closing lets the approval send.</summary>
    StillSends,
    /// <summary>The Traces words' <c>dismiss_action</c>.</summary>
    Dismiss,
}

/// <summary>One card the Windows window shows today, re-housed in the stack.</summary>
public sealed record NoticeKind(string Id, NoticeGroup Group, NoticeClose Close);

/// <summary>
/// The cards this shell already had before the Monitor window, each placed
/// where the Mac places its own card of the same kind, with the Mac's X rule.
/// No card is added here: a Mac card Windows does not show is a new notice on
/// Windows and needs the owner's own yes (copy guide rule 9, spec W2-D4).
/// </summary>
public static class NoticeCatalog
{
    /// <summary>The update offer: about this app, every tab; no X, as before (a closed one is an update that never happens).</summary>
    public static readonly NoticeKind Update = new("update", NoticeGroup.App, NoticeClose.None);

    /// <summary>After a legacy identity moved (MonitorNotices.swift:230-233): the X is its acknowledgement.</summary>
    public static readonly NoticeKind LegacyNotice = new("legacy-notice", NoticeGroup.App, NoticeClose.PutsAway);

    /// <summary>The offer to move a legacy identity (MonitorNotices.swift:244-247): the X is its dismissal.</summary>
    public static readonly NoticeKind LegacyOffer = new("legacy-offer", NoticeGroup.App, NoticeClose.PutsAway);

    /// <summary>
    /// The first-contribution note (MonitorNotices.swift:321-325): the X puts
    /// it away. The Mac keeps that for good on the machine; this shell has no
    /// per-user store for it yet, so the X keeps it closed until the window
    /// opens again.
    /// </summary>
    public static readonly NoticeKind FirstContribution = new("first-contribution", NoticeGroup.TracesOffers, NoticeClose.ForNow);

    /// <summary>The arming offer: Not now stays, the X closes it for now (MonitorNotices.swift:350-356).</summary>
    public static NoticeKind ArmingOffer(string projectId) => new("arming-offer:" + projectId, NoticeGroup.TracesOffers, NoticeClose.ForNow);

    /// <summary>
    /// Every grant the daemon voided, as one stack entry. On the Mac each void
    /// is its own card whose X is its acknowledgement (MonitorNotices.swift:450-480);
    /// this shell draws them as one list with an Acknowledge button on each,
    /// so the entry itself has no X. The voids come first in the consent
    /// notices (MonitorNotices.swift:452-454).
    /// </summary>
    public static readonly NoticeKind GrantVoids = new("void", NoticeGroup.TracesNow, NoticeClose.None);

    /// <summary>
    /// Every arming rewording, as one stack entry with an Acknowledge button
    /// on each (the Mac's X, MonitorNotices.swift:482-508); after the voids.
    /// </summary>
    public static readonly NoticeKind ArmingRewordings = new("reworded", NoticeGroup.TracesNow, NoticeClose.None);

    /// <summary>A consent gate hold: no X, it goes when the hold does (MonitorNotices.swift:511-513).</summary>
    public static readonly NoticeKind GateHeld = new("gate-held", NoticeGroup.TracesNow, NoticeClose.None);

    /// <summary>
    /// Sessions held while the privacy witness is busy. No X: nothing to
    /// answer, and it goes when the hold does.
    /// </summary>
    public static readonly NoticeKind WitnessCapacity = new("witness-capacity", NoticeGroup.TracesNow, NoticeClose.None);

    /// <summary>
    /// The core's health. No X in this shell: the label does not say whether
    /// it is a waiting safeguard (closable on the Mac) or a down core (not).
    /// </summary>
    public static readonly NoticeKind Health = new("health", NoticeGroup.TracesNow, NoticeClose.None);

    /// <summary>The id prefix of an approval's undo card (MonitorNotices.swift:655).</summary>
    public const string UndoPrefix = "approval-undo:";

    /// <summary>
    /// An approval's undo (MonitorNotices.swift:651-662): the X lets it send,
    /// as Dismiss did. Its id holds the approved entries', so a second
    /// approval is a new card that slides in, as on the Mac.
    /// </summary>
    public static NoticeKind Undo(IEnumerable<string> entryIds) =>
        new(UndoPrefix + string.Join(",", entryIds), NoticeGroup.ApprovalUndo, NoticeClose.PutsAway);

    private const string TransferPrefix = "transfer:";

    /// <summary>A trace's transfer card: its X closes it alone (TraceTransferCard.swift:151-160).</summary>
    public static NoticeKind Transfer(string transferId) => new(TransferPrefix + transferId, NoticeGroup.Transfers, NoticeClose.PutsAway);

    /// <summary>Whether <paramref name="id"/> is a transfer card.</summary>
    public static bool IsTransfer(string id) => id.StartsWith(TransferPrefix, StringComparison.Ordinal);

    /// <summary>The transfer a card's id names.</summary>
    public static string TransferId(string id) => id[TransferPrefix.Length..];

    /// <summary>Whether <paramref name="id"/> is an approval's undo card.</summary>
    public static bool IsUndo(string id) => id.StartsWith(UndoPrefix, StringComparison.Ordinal);

    /// <summary>
    /// A one-time offer to turn a notification kind on
    /// (MonitorNotices.swift:695-725, W17-D2): the idle offer with the traces
    /// offers, the verdicts offer with History's own. Not now closes it
    /// until the window opens again; No thanks answers it.
    /// </summary>
    public static NoticeKind NotifyOffer(string kind) =>
        new("notify-offer:" + kind, kind == "verdicts_landed" ? NoticeGroup.HistoryOffers : NoticeGroup.TracesOffers, NoticeClose.ForNow);

    /// <summary>The offer to answer model calls: the X declines it (MonitorNotices.swift:751-761).</summary>
    public static readonly NoticeKind PrivateAIOffer = new("private-ai-offer", NoticeGroup.PrivateAI, NoticeClose.Declines);

    /// <summary>One fixed sentence about the last decision: the X clears it (MonitorNotices.swift:266-269).</summary>
    public static readonly NoticeKind ActionNotice = new("action-notice", NoticeGroup.Action, NoticeClose.PutsAway);

    /// <summary>
    /// The word that names a card's X (MonitorNotices.swift): the decline's
    /// word for an offer whose X declines it (:752-761), the legacy offer's
    /// own dismissal (:244-247), the undo's "still sends" sentence while
    /// approved traces will send (:661), the core's dismiss word for the
    /// first-contribution note (:324), and <c>close</c> otherwise (:168).
    /// </summary>
    public static CloseWord CloseWordFor(NoticeKind card) =>
        card.Close == NoticeClose.Declines ? CloseWord.Decline
        : card.Id == LegacyOffer.Id ? CloseWord.LegacyDismiss
        : IsUndo(card.Id) ? CloseWord.StillSends
        : card.Id == FirstContribution.Id ? CloseWord.Dismiss
        : CloseWord.Close;

    /// <summary>
    /// The groups a place draws, back first (MonitorNotices.swift:140-161).
    /// Home's terms hold has no card in this shell.
    /// </summary>
    public static IReadOnlyList<NoticeGroup> Groups(NoticePlace place) => place switch
    {
        NoticePlace.Traces =>
            new[] { NoticeGroup.App, NoticeGroup.TracesOffers, NoticeGroup.TracesNow, NoticeGroup.Action, NoticeGroup.Transfers, NoticeGroup.ApprovalUndo },
        NoticePlace.History =>
            new[] { NoticeGroup.App, NoticeGroup.TracesOffers, NoticeGroup.HistoryOffers, NoticeGroup.TracesNow, NoticeGroup.Action, NoticeGroup.Transfers, NoticeGroup.ApprovalUndo },
        NoticePlace.HomeHistory => new[] { NoticeGroup.App, NoticeGroup.HistoryOffers, NoticeGroup.Action },
        NoticePlace.PrivateAI => new[] { NoticeGroup.App, NoticeGroup.PrivateAI, NoticeGroup.Action },
        _ => new[] { NoticeGroup.App, NoticeGroup.Action },
    };

    /// <summary>
    /// The cards a place shows, back first and front last: the present cards
    /// of each group the place draws, in group and rank order. Cards of the
    /// same rank keep their input order.
    /// </summary>
    public static IReadOnlyList<NoticeKind> Order(NoticePlace place, IEnumerable<NoticeKind> present)
    {
        var cards = present.ToArray();
        return Groups(place)
            .SelectMany(group => cards.Where(card => card.Group == group).OrderBy(card => Rank(card.Id)))
            .ToArray();
    }

    /// <summary>
    /// The order inside the traces-now group. Consent acknowledgements sit
    /// in front of the holds and health banner so they remain reachable
    /// while a persistent banner has no close action. Cards of other groups
    /// keep their input order.
    /// </summary>
    private static int Rank(string id) => id switch
    {
        "gate-held" => 1,
        "witness-capacity" => 2,
        "health" => 3,
        "void" => 4,
        "reworded" => 5,
        _ => 0,
    };

    /// <summary>
    /// The cards a place shows once the ones closed for now are taken out.
    /// </summary>
    public static IReadOnlyList<NoticeKind> Shown(NoticePlace place, IEnumerable<NoticeKind> present, IReadOnlySet<string> closed) =>
        Order(place, present).Where(card => !closed.Contains(card.Id)).ToArray();
}
