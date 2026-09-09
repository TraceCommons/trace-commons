namespace TraceCommons.Interop;

/// <summary>
/// What joining a commons with a NEAR AI login says, across the C ABI.
/// </summary>
/// <remarks>
/// <para>
/// <b>The way in that needs no wallet.</b> A contributor cannot produce an
/// admissible receipt without a NEAR AI account in the first place, so
/// requiring a wallet as well is a second onboarding for an identity they
/// already hold. Both paths are offered; neither is removed.
/// </para>
/// <para>
/// <b>Ten control names, ten sentences, and this class branches on none of
/// them.</b> The daemon's label goes straight through. A <c>switch</c> here
/// would be an eleventh table that agrees with the shared one until it does
/// not.
/// </para>
/// <para>
/// <b>The three that refuse before anything is spent must not be run
/// together.</b> <c>near_ai_enroll_no_session</c> means sign in first,
/// <c>near_ai_enroll_commons_unreachable</c> means the network, and
/// <c>near_ai_enroll_commons_unsupported</c> means this commons does not
/// offer the path at all. A contributor told the wrong one debugs the wrong
/// thing.
/// </para>
/// </remarks>
public static class NearAiEnrollSurface
{
    /// <summary>The one daemon state meaning a usable sign-in is kept here.</summary>
    /// <remarks>
    /// Read only to decide whether a control that REQUIRES a sign-in may be
    /// offered. <c>daemon::nearai_credential::LABEL_CREDENTIAL_PRESENT</c> is
    /// the definition; this is the shell's single mirror of it.
    /// </remarks>
    public const string CredentialStatePresent = "present";

    /// <summary>The generic label, for a failure this build cannot name.</summary>
    public const string Unavailable = "near_ai_enroll_unavailable";

    /// <summary>
    /// The sentence for one control name. Null only on a caught Rust panic.
    /// </summary>
    public static string? Line(string? label) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_near_ai_enroll_line(label));

    /// <summary>
    /// How firmly it reads, as a raw <c>TC_PRIVATE_INFERENCE_TONE_*</c>
    /// value. Not every failure is a wall: not being signed in yet is
    /// attention, and being already joined is clear.
    /// </summary>
    public static PrivateInferenceTone Tone(string? label) =>
        PrivateInferenceSurface.FromAbiTone(NativeMethods.tc_near_ai_enroll_tone(label));
}
