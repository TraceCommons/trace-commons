using System;

namespace TraceCommons.Interop;

/// <summary>
/// The consent invariant, as state: <b>an approval must cover exactly the
/// bytes the contributor was shown.</b>
///
/// Contribute is the one irreversible control in the product, and blind
/// approval of a real transcript is the unrecoverable misclick. So it is not
/// enabled by anything the app knows -- it is enabled by three things the
/// contributor did, all of which are held here:
///
/// <list type="number">
///   <item>a real, pinned preview exists to approve against
///     (<see cref="HasPinnedPreview"/>),</item>
///   <item>"Exactly what would be sent" has actually been on screen
///     (<see cref="TranscriptShown"/>),</item>
///   <item>the acknowledgement has been ticked by hand
///     (<see cref="Acknowledged"/>).</item>
/// </list>
///
/// The gate is deliberately "first screen plus explicit acknowledgement" and
/// not "scrolled to the end". Real pilot traces run to 169 KB; a
/// scroll-to-the-bottom gate on that is a long drag, and one people defeat by
/// throwing the scrollbar at the end verifies nothing while reading, to
/// everyone downstream, as though it verified reading. This one claims only
/// what it can establish, and <see cref="Footnote"/> concedes the rest out
/// loud -- a gate that overstated what it checked would be worse than no gate.
///
/// It lives in the interop assembly rather than in a WinUI view model on
/// purpose: this is the safety property of the Windows shell, and it is
/// testable here on a machine that cannot build WinUI at all. The macOS sheet
/// holds the same two flags and the Linux sheet's <c>sync_contribute</c> ANDs
/// the same three conditions.
///
/// Nothing here is persisted and nothing starts set. The gate lives and dies
/// with one sheet showing one session, so every entry starts it from zero --
/// which is what makes "the bytes the contributor was shown" mean this
/// session's bytes rather than some earlier session's.
/// </summary>
public sealed class ReadGate
{
    /// <summary>The first requirement, before it has been met.</summary>
    public const string OpenPrompt = "Open \"Exactly what would be sent\" and look at it.";

    /// <summary>The first requirement, once met.</summary>
    public const string Opened = "You have opened \"Exactly what would be sent\".";

    /// <summary>
    /// The second requirement. It is the contributor saying the thing the app
    /// cannot say for them, which is why it is a box they tick rather than a
    /// sentence they are shown.
    /// </summary>
    public const string Acknowledgement =
        "I have looked at what would be sent, and I understand scrubbing is "
        + "pattern-based and may have missed something.";

    /// <summary>
    /// What the gate concedes. Shown while Contribute is still off, which is
    /// exactly when someone is looking for the reason it is off.
    /// </summary>
    public const string Footnote =
        "Contribute stays off until both are done. Looking at the first screen is what "
        + "this checks — it cannot check that you read all of it, and it does not claim to.";

    /// <summary>Tooltip on an armed Contribute.</summary>
    public const string ReadyHelp = "Sends this session. Nothing else.";

    /// <summary>Tooltip on a Contribute the gate is still holding shut.</summary>
    public const string BlockedHelp =
        "Open \"Exactly what would be sent\" and tick the acknowledgement first.";

    /// <summary>
    /// Why Contribute is off on a preview that was built without an
    /// enrollment. Nothing was pinned, so there is nothing to bind an
    /// approval to; saying so beats a button that fails when pressed.
    /// </summary>
    public const string UnenrolledHelp =
        "This device isn't connected yet, so this preview was built without your identity "
        + "and nothing here can be contributed.";

    private bool _acknowledged;

    /// <summary>Raised whenever any of the three conditions changes.</summary>
    public event Action? Changed;

    /// <summary>
    /// Whether a real preview is loaded and pinned. Set from
    /// <see cref="PreviewSummary.Enrolled"/>; a failed or unenrolled preview
    /// leaves it false.
    /// </summary>
    public bool HasPinnedPreview { get; private set; }

    /// <summary>
    /// Whether the redacted transcript has actually been put on screen. Set
    /// once, by the transcript view itself rather than by whatever navigated
    /// to it, so it records display and not intent.
    /// </summary>
    public bool TranscriptShown { get; private set; }

    /// <summary>
    /// The acknowledgement.
    ///
    /// Setting it to true before the transcript has been shown is IGNORED
    /// rather than honoured, because order is part of the invariant: ticking
    /// a box about bytes nobody has seen is not an acknowledgement of
    /// anything. The UI also disables the control, but the rule is enforced
    /// here so it holds regardless of which shell drives it.
    /// </summary>
    public bool Acknowledged
    {
        get => _acknowledged;
        set
        {
            bool next = value && TranscriptShown;
            if (_acknowledged == next)
            {
                return;
            }

            _acknowledged = next;
            Changed?.Invoke();
        }
    }

    /// <summary>The one question the sheet asks this object.</summary>
    public bool CanContribute => HasPinnedPreview && TranscriptShown && Acknowledged;

    /// <summary>The tooltip that explains the current answer.</summary>
    public string Help =>
        CanContribute ? ReadyHelp
        : !HasPinnedPreview ? UnenrolledHelp
        : BlockedHelp;

    /// <summary>The first gate line, in the state it is actually in.</summary>
    public string OpenedLine => TranscriptShown ? Opened : OpenPrompt;

    /// <summary>
    /// Records that a pinned preview is available. A summary that failed to
    /// parse, or one built without an enrollment, must pass false.
    /// </summary>
    public void SetPinnedPreview(bool pinned)
    {
        if (HasPinnedPreview == pinned)
        {
            return;
        }

        HasPinnedPreview = pinned;
        Changed?.Invoke();
    }

    /// <summary>
    /// Records that the redacted transcript has been displayed. Idempotent,
    /// and one-way for the life of the gate: what it records is that the
    /// bytes were put in front of someone, and navigating away does not
    /// unshow them.
    /// </summary>
    public void MarkTranscriptShown()
    {
        if (TranscriptShown)
        {
            return;
        }

        TranscriptShown = true;
        Changed?.Invoke();
    }

    /// <summary>
    /// Clears every condition.
    ///
    /// Called when the sheet moves to a different session. A gate that
    /// carried over would let the second session inherit the first one's
    /// consent, which is precisely the thing this class exists to prevent.
    /// </summary>
    public void Reset()
    {
        if (!HasPinnedPreview && !TranscriptShown && !_acknowledged)
        {
            return;
        }

        HasPinnedPreview = false;
        TranscriptShown = false;
        _acknowledged = false;
        Changed?.Invoke();
    }
}
