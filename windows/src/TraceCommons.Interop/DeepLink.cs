using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace TraceCommons.Interop;

/// <summary>
/// The <c>tracecommons://enroll?invite=…</c> URL scheme.
///
/// An issuer link cannot open a desktop app, so an invite mail carries the
/// app's own scheme with the real invite -- an issuer URL -- folded into the
/// <c>invite</c> query parameter. This is the same shape macOS parses in
/// <c>DeepLink.inviteURL</c> and the Rust shells parse in
/// <c>invite_from_deep_link</c>; all three must agree, because one invite
/// mail is sent to contributors on all three platforms.
///
/// This lives in the interop layer rather than the WinUI app for the reason
/// the whole layer exists: it is plain net8.0, so it is tested on a machine
/// that is not running Windows instead of only in CI.
/// </summary>
public static class DeepLink
{
    /// <summary>
    /// The invite inside a deep link, or <c>null</c> for anything else.
    /// </summary>
    /// <remarks>
    /// Registering a scheme handler means this question gets asked about
    /// every argument the app is ever launched with, including its own
    /// switches, so everything that is not an invite has to answer null
    /// rather than throw.
    ///
    /// Scheme and host are compared case-insensitively. A handler
    /// registration elsewhere in the system need not preserve the case
    /// anyone typed, and the other two shells accept either case too.
    /// </remarks>
    public static string? InviteFrom(string? argument)
    {
        if (string.IsNullOrWhiteSpace(argument))
        {
            return null;
        }

        if (!Uri.TryCreate(argument, UriKind.Absolute, out Uri? uri))
        {
            return null;
        }

        if (!string.Equals(uri.Scheme, "tracecommons", StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }

        // Uri.Host is already lower-cased for a well-formed authority, but
        // comparing case-insensitively costs nothing and does not depend on
        // that being true for every input.
        if (!string.Equals(uri.Host, "enroll", StringComparison.OrdinalIgnoreCase))
        {
            return null;
        }

        string invite = ParseInviteParameter(uri.Query);
        return string.IsNullOrEmpty(invite) ? null : invite;
    }

    /// <summary>
    /// Selects the cold-launch invite. A protocol payload is authoritative,
    /// even when invalid; only ordinary launches fall back to argv.
    /// Null denotes a non-protocol activation, while empty denotes a protocol
    /// activation whose payload was missing. This never performs enrollment.
    /// </summary>
    public static string? InitialInvite(string? protocolUri, IEnumerable<string> arguments)
    {
        if (protocolUri is not null)
        {
            return InviteFrom(protocolUri);
        }
        foreach (string argument in arguments)
        {
            if (InviteFrom(argument) is string invite)
            {
                return invite;
            }
        }
        return null;
    }

    /// <summary>
    /// Pulls <c>invite</c> out of a query string.
    /// </summary>
    /// <remarks>
    /// Hand-rolled rather than System.Web.HttpUtility.ParseQueryString: that
    /// lives in an assembly this project does not reference, and pulling one
    /// in to read a single parameter is not a trade worth making for a
    /// dependency that ships inside the app.
    /// </remarks>
    private static string ParseInviteParameter(string query)
    {
        if (string.IsNullOrEmpty(query))
        {
            return string.Empty;
        }

        // Uri.Query keeps the leading '?'.
        foreach (string pair in query.TrimStart('?').Split('&'))
        {
            int split = pair.IndexOf('=');
            if (split <= 0)
            {
                continue;
            }

            if (!pair.AsSpan(0, split).Equals("invite", StringComparison.Ordinal))
            {
                continue;
            }

            return Uri.UnescapeDataString(pair[(split + 1)..]);
        }

        return string.Empty;
    }
}

/// <summary>
/// The invite link itself, as the contributor crate parses it.
/// </summary>
public static class Invite
{
    /// <summary>
    /// The instance an invite names, for showing before committing, or
    /// <c>null</c> if the text is not a usable invite.
    /// </summary>
    /// <remarks>
    /// Answered by the Rust crate rather than re-parsed here, so this shell
    /// and the CLI agree on what a valid invite is. Only the host crosses
    /// the boundary: the rest of the invite is the credential, and a shell
    /// cannot leak what it was never handed.
    ///
    /// Null covers every rejection, which is what the caller wants -- the
    /// whole invite path has one failure sentence, so "not a URL" and "no
    /// code in it" must not be told apart.
    /// </remarks>
    public static string? IssuerHost(string? invite)
    {
        if (string.IsNullOrWhiteSpace(invite))
        {
            return null;
        }

        IntPtr owned = NativeMethods.tc_invite_issuer_host(invite);
        return NativeMethods.TakeOwnedString(owned);
    }
}
