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
    /// <summary>What is happening to the traces now (<c>tracesNow</c>).</summary>
    TracesNow,
    /// <summary>The Private AI tab's own (<c>privateAIItems</c>).</summary>
    PrivateAI,
    /// <summary>A failed or finished action, in front of the rest (<c>actionItems</c>).</summary>
    Action,
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

    /// <summary>A voided grant: the X is its acknowledgement; Turn back on stays (MonitorNotices.swift:460-463).</summary>
    public static NoticeKind GrantVoid(string id) => new("void:" + id, NoticeGroup.TracesNow, NoticeClose.PutsAway);

    /// <summary>An arming rewording: the X is its acknowledgement; Ask stays (MonitorNotices.swift:489-492).</summary>
    public static NoticeKind ArmingRewording(string id) => new("reworded:" + id, NoticeGroup.TracesNow, NoticeClose.PutsAway);

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

    /// <summary>An approval's undo: Dismiss is the X (MonitorNotices.swift:658-662).</summary>
    public static readonly NoticeKind Undo = new("undo", NoticeGroup.TracesNow, NoticeClose.PutsAway);

    /// <summary>The offer to answer model calls: the X declines it (MonitorNotices.swift:751-761).</summary>
    public static readonly NoticeKind PrivateAIOffer = new("private-ai-offer", NoticeGroup.PrivateAI, NoticeClose.Declines);

    /// <summary>One fixed sentence about the last decision: the X clears it (MonitorNotices.swift:266-269).</summary>
    public static readonly NoticeKind ActionNotice = new("action-notice", NoticeGroup.Action, NoticeClose.PutsAway);

    /// <summary>
    /// The groups a place draws, back first (MonitorNotices.swift:140-161).
    /// History's own offers and Home's terms hold have no card in this shell.
    /// </summary>
    public static IReadOnlyList<NoticeGroup> Groups(NoticePlace place) => place switch
    {
        NoticePlace.Traces or NoticePlace.History =>
            new[] { NoticeGroup.App, NoticeGroup.TracesOffers, NoticeGroup.TracesNow, NoticeGroup.Action },
        NoticePlace.PrivateAI => new[] { NoticeGroup.App, NoticeGroup.PrivateAI, NoticeGroup.Action },
        _ => new[] { NoticeGroup.App, NoticeGroup.Action },
    };

    /// <summary>
    /// The cards a place shows, back first and front last: the present cards
    /// of each group the place draws, in group order, each group keeping the
    /// order it was given in. A card of a group the place does not draw is
    /// not shown there.
    /// </summary>
    public static IReadOnlyList<NoticeKind> Order(NoticePlace place, IEnumerable<NoticeKind> present)
    {
        var cards = present.ToArray();
        return Groups(place).SelectMany(group => cards.Where(card => card.Group == group)).ToArray();
    }

    /// <summary>
    /// The cards a place shows once the ones closed for now are taken out.
    /// </summary>
    public static IReadOnlyList<NoticeKind> Shown(NoticePlace place, IEnumerable<NoticeKind> present, IReadOnlySet<string> closed) =>
        Order(place, present).Where(card => !closed.Contains(card.Id)).ToArray();
}
