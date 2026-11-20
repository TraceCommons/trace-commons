using System;

namespace TraceCommons.Interop;

/// <summary>
/// The account a consent write belongs to, as the Mac tracks it
/// (<c>settleAutomaticGrantOwner</c> and <c>accountReadGeneration</c>,
/// AppModel.swift:1514 and :4701 at origin/main 83016f544; the Linux port is
/// <c>GrantOwner</c>, crates/trace-commons-contributor-gtk/src/ui/consent_settings.rs:113).
/// </summary>
/// <remarks>
/// A status read settles the owner. A status naming a different account, a
/// sign-out or an unenroll starts a new generation; a write pressed under
/// the old one is reported refused and nothing it answers is applied
/// (checklist L2).
/// </remarks>
public sealed class AccountGeneration
{
    /// <summary>
    /// Raised by <see cref="Boundary"/>: a sign-out or unenroll finished in
    /// some window, and every window's consent presses from before it are
    /// void. The app holds one instance for all its windows.
    /// </summary>
    public event Action? BoundaryCrossed;

    /// <summary>Raised on every new generation.</summary>
    /// <summary>
    /// Raised on every new generation: a status naming another account, or
    /// a <see cref="Boundary"/>. Settings sections that list the account's
    /// own things (devices, payouts, referrals) read them again on it.
    /// </summary>
    public event Action? Changed;

    private bool _seen;
    private string? _scope;

    /// <summary>The generation a press is stamped with.</summary>
    public long Current { get; private set; }

    /// <summary>
    /// A status read answered with <paramref name="scope"/>. True when it
    /// named a different account than the last answered read, which is a
    /// boundary.
    /// </summary>
    public bool Observe(string? scope)
    {
        bool changed = _seen && _scope != scope;
        _seen = true;
        _scope = scope;
        if (changed)
        {
            Current++;
            Changed?.Invoke();
        }

        return changed;
    }

    /// <summary>A sign-out or unenroll finished: writes pressed before it are not applied.</summary>
    public void Boundary()
    {
        Current++;
        BoundaryCrossed?.Invoke();
        Changed?.Invoke();
    }

    /// <summary>Whether a press stamped <paramref name="pressedUnder"/> may still be applied.</summary>
    public bool IsCurrent(long pressedUnder) => pressedUnder == Current;
}
