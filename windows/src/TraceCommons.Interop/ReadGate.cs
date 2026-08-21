using System;

namespace TraceCommons.Interop;

/// <summary>
/// What the preview sheet requires, and what it says, at the moment of
/// consent.
///
/// <para>
/// <b>This used to be a read gate.</b> Contribute waited on three things: a
/// pinned preview, "Exactly what would be sent" having actually been on
/// screen, and an acknowledgement ticked by hand. Two of them are gone.
/// </para>
/// <para>
/// The checkbox was removed as friction. The transcript-shown condition
/// went with it, for a reason worth recording: a queue row's Submit
/// approves the same session with no preview opened at all, so the gate
/// never stood between anybody and a blind approval -- it only charged a
/// click to the one contributor who chose to look. A control that taxes
/// the careful path and stops nobody is not a safety property.
/// </para>
/// <para>
/// <b>What did not go is the claim.</b> <see cref="Statement"/> carries both
/// halves of what the checkbox made a contributor assert -- scrubbing is
/// pattern-based and may have missed something, and nothing here can tell
/// whether anyone read anything -- and the sheet prints it above Contribute
/// where the tick used to be asked for.
/// </para>
/// <para>
/// And <see cref="HasPinnedPreview"/> stayed. That was never friction: an
/// approval binds to the envelope a preview pinned, and a preview that
/// failed or was built without an enrollment pinned nothing. Nothing here
/// is persisted and nothing starts set, so every sheet starts from zero and
/// "the bytes the contributor was shown" means this session's bytes.
/// </para>
/// <para>
/// It lives in the interop assembly rather than in a WinUI view model on
/// purpose: it is testable here on a machine that cannot build WinUI at
/// all. The macOS shell holds the same rule and the same sentence in
/// <c>TCShellCore/ReadGate.swift</c>, the Linux shell in
/// <c>crates/trace-commons-contributor-gtk/src/copy.rs</c>, and a Rust test
/// reads this file to prove the sentence has not drifted.
/// </para>
/// </summary>
public sealed class ReadGate
{
    /// <summary>
    /// The sentence that replaced the acknowledgement checkbox.
    ///
    /// One line, one escaped literal, on purpose: the Rust parity test
    /// scans this file for this exact text, and a line break or a string
    /// concatenation here would defeat it.
    /// </summary>
    public const string Statement = "\"Exactly what would be sent\" is the exact text that would leave this machine. Pattern-based scrubbing may have missed something in it, and nothing here checks that you looked.";

    /// <summary>Tooltip on an armed Contribute.</summary>
    public const string ReadyHelp = "Sends this session. Nothing else.";

    /// <summary>
    /// Why Contribute is off on a preview that was built without an
    /// enrollment. Nothing was pinned, so there is nothing to bind an
    /// approval to; saying so beats a button that fails when pressed.
    /// </summary>
    public const string UnenrolledHelp =
        "This device isn't connected yet, so this preview was built without your identity "
        + "and nothing here can be contributed.";

    /// <summary>Raised whenever the one condition changes.</summary>
    public event Action? Changed;

    /// <summary>
    /// Whether a real preview is loaded and pinned. Set from
    /// <see cref="PreviewSummary.Enrolled"/>; a failed or unenrolled preview
    /// leaves it false.
    /// </summary>
    public bool HasPinnedPreview { get; private set; }

    /// <summary>The one question the sheet asks this object.</summary>
    public bool CanContribute => HasPinnedPreview;

    /// <summary>The tooltip that explains the current answer.</summary>
    public string Help => CanContribute ? ReadyHelp : UnenrolledHelp;

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
    /// Clears the pin.
    ///
    /// Called when the sheet moves to a different session. A pin that
    /// carried over would let the second session be approved against the
    /// first one's envelope.
    /// </summary>
    public void Reset() => SetPinnedPreview(false);
}
