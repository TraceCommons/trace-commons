using System;
using System.Collections.Generic;

namespace TraceCommons.Interop;

/// <summary>
/// The contributor's recent searches, kept for the life of the process and
/// never written to disk.
/// </summary>
/// <remarks>
/// <para>
/// The shared spec asks for these to persist so the second trace is one
/// keystroke, and the macOS shell persists them. This one deliberately does
/// not: <b>a recent search is the contributor's own list of the things they
/// are worried about leaking</b> -- a client's name, an internal code name, an
/// address -- and writing that list to disk creates a small file of exactly
/// the material the rest of the app works to keep on the machine's own terms.
/// In-session recall covers the case the spec argues for, which is checking
/// several traces for the same term in one sitting.
/// </para>
/// <para>
/// It holds what the contributor ASKED, not every prefix they typed on the way
/// there. See <c>PreviewSheetViewModel.RunSearch</c>, which takes the intent
/// as a parameter: live search on every keystroke is the good part and stays;
/// recording there is what filled the six slots with the prefixes of one word.
/// </para>
/// <para>
/// Process-wide static rather than per-sheet, because recall across the
/// several traces of one sitting is the whole point, and a sheet lives for one
/// of them. <see cref="Reset"/> exists so a test can start from a known list;
/// nothing in the app calls it.
/// </para>
/// </remarks>
public static class RecentSearches
{
    /// <summary>
    /// How many are kept. Six is what the strip has room for, and an older
    /// question is one the contributor can type again.
    /// </summary>
    public const int Capacity = 6;

    private static readonly object Gate = new();
    private static readonly List<string> Terms = new();

    /// <summary>The list, most recent first.</summary>
    public static IReadOnlyList<string> Current
    {
        get
        {
            lock (Gate)
            {
                return Terms.ToArray();
            }
        }
    }

    /// <summary>
    /// Records one committed term and returns the new list.
    /// </summary>
    /// <remarks>
    /// Repeating a term moves it to the front rather than adding a second
    /// copy: it is the same question asked again, and two rows of it would
    /// cost a slot that could hold a different one.
    ///
    /// A blank term is not a question and is not recorded.
    /// </remarks>
    public static IReadOnlyList<string> Remember(string term)
    {
        ArgumentNullException.ThrowIfNull(term);

        string trimmed = term.Trim();
        lock (Gate)
        {
            if (trimmed.Length == 0)
            {
                return Terms.ToArray();
            }

            Terms.RemoveAll(existing => string.Equals(existing, trimmed, StringComparison.Ordinal));
            Terms.Insert(0, trimmed);
            while (Terms.Count > Capacity)
            {
                Terms.RemoveAt(Terms.Count - 1);
            }

            return Terms.ToArray();
        }
    }

    /// <summary>Empties the list. A test seam; the app never calls it.</summary>
    public static void Reset()
    {
        lock (Gate)
        {
            Terms.Clear();
        }
    }
}
