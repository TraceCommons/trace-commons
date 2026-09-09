namespace TraceCommons.Interop;

/// <summary>
/// The certificate-held list's two readings, across the C ABI.
/// </summary>
/// <remarks>
/// <para>
/// One fact -- a witness certificate is held for the bytes a queue row was
/// pinned to, true after either witness route -- read two ways: as a
/// candidate for submission by a contributor with no invite, and as a
/// cryptographically attested trace by one with an invite.
/// </para>
/// <para>
/// <b>The choice between them is behind the ABI, not here.</b> Three shells
/// each writing that choice would be three chances to swap the readings, and
/// a swapped reading tells a contributor with no invite that their session
/// carries cryptographic proof when nothing has attested it.
/// </para>
/// <para>
/// <b><c>evidenceAdmitted</c> is <c>admission_evidence_required</c> verbatim,
/// never its negation.</b> The flag is true for a contributor who signed up
/// through NEAR and therefore has no invite. Passing the negation would swap
/// both readings and compile.
/// </para>
/// <para>
/// Not <see cref="AttestationMarkSurface"/>. That answers whether a session
/// carries a copy of the model call that produced it; this answers whether a
/// certificate is held over the reviewed bytes.
/// </para>
/// </remarks>
public static class CertificateSurface
{
    /// <summary>The sentence for one row. Null on a caught Rust panic.</summary>
    public static string? RowLine(bool evidenceAdmitted) =>
        NativeMethods.TakeOwnedString(
            NativeMethods.tc_certificate_row_line(evidenceAdmitted ? 1 : 0));

    /// <summary>The heading over the list, on the same argument.</summary>
    public static string? ListTitle(bool evidenceAdmitted) =>
        NativeMethods.TakeOwnedString(
            NativeMethods.tc_certificate_list_title(evidenceAdmitted ? 1 : 0));
}
