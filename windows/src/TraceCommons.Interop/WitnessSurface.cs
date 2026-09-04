using System;

namespace TraceCommons.Interop;

/// <summary>
/// The redaction-witness surface, across the C ABI.
///
/// Nothing in this file is a word. The vocabulary crosses as JSON and the
/// sentences cross already assembled, so this shell never fills in a template
/// and there is no fourth place the wording can drift to.
///
/// THERE IS NO BOOLEAN ON THIS SURFACE AND THERE MUST NEVER BE ONE. "Is a
/// witness configured?" has two yes-answers that are opposites: a pinned
/// witness certifies every submission, and an unpinned one refuses every
/// submission before it touches the network. <see cref="TrustState"/> is the
/// one answer, with a value per condition, and nothing here collapses them.
/// </summary>
public static class WitnessSurface
{
    /// <summary>
    /// What the witness is doing, as a <c>TC_WITNESS_STATE_*</c> value.
    ///
    /// The one call to make before rendering anything about the witness. A
    /// value this build does not name must be rendered as not usable, never as
    /// <see cref="WitnessTools.StateAbsent"/>.
    /// </summary>
    public static int TrustState(string configDir) =>
        NativeMethods.tc_witness_trust_state(configDir);

    /// <summary>
    /// The whole witness configuration, or the fixed label the ABI declined
    /// with.
    /// </summary>
    /// <remarks>
    /// A null status is never "no witness": that is a successful call
    /// reporting the absent state. It is an unenrolled device, or a config
    /// that could not be read -- and the second is a refusal.
    /// </remarks>
    public static WitnessReadResult Status(string configDir)
    {
        IntPtr raw = NativeMethods.tc_witness_status_json(configDir, out IntPtr errPtr);
        string? error = NativeMethods.TakeOwnedString(errPtr);
        WitnessStatus? status = WitnessTools.ParseStatus(NativeMethods.TakeOwnedString(raw));
        return new WitnessReadResult(status, status is null ? error ?? string.Empty : null);
    }

    /// <summary>
    /// Points this device at a witness. Zero on success.
    /// </summary>
    /// <remarks>
    /// The ABI will not write an unpinned witness: an empty measurement array
    /// and an unparsable one are both refused, because either produces a
    /// client that refuses every submission from the moment it is saved.
    /// Takes effect on the next session sent, with no restart.
    /// </remarks>
    public static WitnessWriteResult Configure(
        string configDir,
        string url,
        string signingAddress,
        string measurementsJson)
    {
        int code = NativeMethods.tc_witness_configure(
            configDir,
            url,
            signingAddress,
            measurementsJson,
            out IntPtr errPtr);
        return new WitnessWriteResult(code, NativeMethods.TakeOwnedString(errPtr));
    }

    /// <summary>
    /// Removes the configured witness. One on removal, zero if there was none,
    /// minus one on failure.
    /// </summary>
    /// <remarks>
    /// This returns the client to local redaction, which is a supported mode
    /// and not a broken one -- but it is still a real change, and the sentence
    /// beside the control says so rather than presenting it as switching a
    /// setting off.
    /// </remarks>
    public static WitnessWriteResult Clear(string configDir)
    {
        int code = NativeMethods.tc_witness_clear(configDir, out IntPtr errPtr);
        return new WitnessWriteResult(code, NativeMethods.TakeOwnedString(errPtr));
    }

    /// <summary>
    /// Every fixed word on the card, or null when the call failed or the
    /// payload will not parse. The caller renders nothing rather than a word
    /// of its own.
    /// </summary>
    public static WitnessCopy? Copy() =>
        WitnessTools.ParseCopy(NativeMethods.TakeOwnedString(NativeMethods.tc_witness_copy()));

    /// <summary>
    /// The sentence for a witness state, decided by the shared branch table.
    ///
    /// Null for a state this build cannot name. A caller that gets null must
    /// render NO witness sentence rather than one of its own, and should pair
    /// that with <see cref="StateTone"/>, which fails closed on the same
    /// input.
    /// </summary>
    public static string? StateLine(int stateCode) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_witness_state_line(stateCode));

    /// <summary>
    /// The tone <see cref="StateLine"/>'s sentence is painted in.
    ///
    /// ONE BRANCH TABLE, NOT TWO: this takes what the sentence takes, so the
    /// two cannot disagree, and no caller may recover the tone by comparing
    /// the rendered sentence against anything.
    /// </summary>
    public static WitnessTone StateTone(int stateCode) =>
        WitnessTools.FromAbiTone(NativeMethods.tc_witness_state_tone(stateCode));

    /// <summary>
    /// What the last submission THIS PROCESS made did about the witness, as a
    /// sentence.
    /// </summary>
    /// <remarks>
    /// THE ONLY FORM THAT MAY BE PRINTED. The JSON form's refusal is a fixed
    /// operator label rather than wording, and its receipt count is a pair no
    /// shell may phrase itself -- when a certificate carried one this sentence
    /// already contains it, never as the word "attested" and never as a claim
    /// that a session is clean.
    ///
    /// Process-local: a freshly started app reports having seen nothing, which
    /// is the honest answer, and no file outlives a logout to show the next
    /// contributor the previous one's outcome.
    /// </remarks>
    public static string? LastResultLine() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_witness_last_result_line());

    /// <summary>
    /// The tone <see cref="LastResultLine"/>'s sentence is painted in. A
    /// refused send is a refusal and never attention: nothing was sent at all,
    /// which is not a degraded-but-working state.
    /// </summary>
    public static WitnessTone LastResultTone() =>
        WitnessTools.FromAbiTone(NativeMethods.tc_witness_last_result_tone());
}
