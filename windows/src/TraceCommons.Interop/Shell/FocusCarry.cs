using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>A control in a redrawn card: its focus key and whether it can take focus.</summary>
public readonly record struct FocusCandidate(string Key, bool Enabled);

/// <summary>
/// Where keyboard focus lands when a Settings card is drawn again (W17
/// review F2; checklist O2, E2). A card drawn again replaces its controls,
/// and WinUI drops focus from a control that leaves the tree or turns
/// itself off, so a keyboard user on Pause, the Insights switch or Save
/// would lose their place on every redraw. The same control keeps focus
/// while it can take it; otherwise the nearest control that can, in the
/// same group (the key up to its last '/'), after it first, then before
/// it (Pause hands to Resume, Save back to its own field); otherwise the
/// nearest in the card; a control that is gone hands to the first that can.
/// Pure, so it is tested where WinUI is not.
/// </summary>
public static class FocusCarry
{
    /// <param name="focused">The focused control's key before the redraw; null when focus was elsewhere.</param>
    /// <param name="candidates">The redrawn card's keyed controls, in tab order.</param>
    /// <returns>The key to focus, or null to leave focus where it is.</returns>
    public static string? Target(string? focused, IReadOnlyList<FocusCandidate> candidates)
    {
        if (focused is null)
        {
            return null;
        }

        int at = -1;
        for (int i = 0; i < candidates.Count; i++)
        {
            if (candidates[i].Key == focused)
            {
                at = i;
                break;
            }
        }

        if (at < 0)
        {
            foreach (FocusCandidate candidate in candidates)
            {
                if (candidate.Enabled)
                {
                    return candidate.Key;
                }
            }

            return null;
        }

        if (candidates[at].Enabled)
        {
            return focused;
        }

        string group = Group(focused);
        return Nearest(candidates, at, c => Group(c.Key) == group)
            ?? Nearest(candidates, at, _ => true);
    }

    private static string Group(string key)
    {
        int slash = key.LastIndexOf('/');
        return slash < 0 ? string.Empty : key[..slash];
    }

    private static string? Nearest(IReadOnlyList<FocusCandidate> candidates, int at, Func<FocusCandidate, bool> inScope)
    {
        for (int i = at + 1; i < candidates.Count; i++)
        {
            if (candidates[i].Enabled && inScope(candidates[i]))
            {
                return candidates[i].Key;
            }
        }

        for (int i = at - 1; i >= 0; i--)
        {
            if (candidates[i].Enabled && inScope(candidates[i]))
            {
                return candidates[i].Key;
            }
        }

        return null;
    }
}
