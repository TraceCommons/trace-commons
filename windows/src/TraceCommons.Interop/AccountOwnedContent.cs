using System;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// Which account the window's record content belongs to, and when it must
/// all go: the Mac's <c>accountOwnedContentScope</c> with its read stamps
/// (<c>loadSessionDetail</c>, <c>reconcileAccountOwnedContent</c> and
/// <c>clearAccountOwnedContent</c>, AppModel.swift:4195-4261).
/// </summary>
/// <remarks>
/// The Mac keeps this on its one app model. Windows builds an inspector per
/// record, so each inspector's own state never sees another account's read;
/// the window holds one of these and every inspector reports its detail
/// reads to it. <see cref="Cleared"/> is raised when:
/// <list type="bullet">
/// <item>a detail names an owner other than the one held;</item>
/// <item>a read that is not stale fails with an account-ending label
/// (<see cref="HistoryDetailState.AccountEndingLabels"/>);</item>
/// <item>Windows' own account boundary is crossed
/// (<see cref="AccountBoundary"/>, wired to
/// <see cref="AccountGeneration.Changed"/>: a status naming another account,
/// a sign-out, an unenroll). The Mac has no such clear: there a record
/// reopened after a sign-out draws its old skill state until its read
/// fails. Windows clears first.</item>
/// </list>
/// A read that started before the one that last decided the owner, and
/// answers for another account, is stale (#869): it neither replaces nor
/// clears what the newer read decided.
/// </remarks>
public sealed class AccountOwnedContent
{
    // Stamps each detail read in the order it started.
    private ulong _requestSequence;

    // The stamp of the newest read that decided the owner, or cleared it.
    private ulong _scopeSequence;

    /// <summary>Raised after everything read under the account must go.</summary>
    public event Action? Cleared;

    /// <summary>The owner held; null before any read named one and after a clear.</summary>
    public string? Scope { get; private set; }

    /// <summary>A detail read is starting: its stamp, passed back with its answer.</summary>
    public ulong ReadStarted() => ++_requestSequence;

    /// <summary>
    /// A detail read answered, naming <paramref name="ownerScope"/> (null from
    /// a core that predates it). False when the read is stale: the caller
    /// shows nothing from it and says the read could not be made
    /// (<see cref="HistoryDetailState.ReadStale"/>).
    /// </summary>
    public bool ReadSucceeded(ulong sequence, string? ownerScope)
    {
        bool superseded = sequence < _scopeSequence;
        if (superseded && ownerScope is not null && ownerScope != Scope)
        {
            return false;
        }

        if (ownerScope is null)
        {
            return true;
        }

        _scopeSequence = Math.Max(_scopeSequence, sequence);
        if (Scope is { } previous && previous != ownerScope)
        {
            Clear();
        }

        Scope = ownerScope;
        return true;
    }

    /// <summary>A detail read failed with <paramref name="label"/>.</summary>
    public void ReadFailed(ulong sequence, string? label)
    {
        bool superseded = sequence < _scopeSequence;
        if (!superseded && label is not null && HistoryDetailState.AccountEndingLabels.Contains(label))
        {
            _scopeSequence = sequence;
            Clear();
        }
    }

    /// <summary>
    /// Windows' account boundary was crossed: everything goes, the next owner
    /// is a first owner, and a read started before the boundary that answers
    /// for an account is stale.
    /// </summary>
    public void AccountBoundary()
    {
        _scopeSequence = ++_requestSequence;
        Clear();
    }

    private void Clear()
    {
        Scope = null;
        Cleared?.Invoke();
    }
}
