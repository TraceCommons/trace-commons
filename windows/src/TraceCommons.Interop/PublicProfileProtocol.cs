using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The shape all three profile methods answer with, mirroring
/// <c>profile_value</c> in
/// <c>crates/trace-commons-contributor/src/daemon/profile.rs</c>.
///
/// <c>set_public_profile</c>, <c>clear_public_profile</c> and
/// <c>get_public_profile</c> deliberately share one payload on the daemon's
/// side so a client parses one thing whichever call it made, and this type is
/// that one thing.
/// </summary>
/// <remarks>
/// <para>What the daemon reports is a LOCAL CACHE, and it says so: there is no
/// <c>GET /v1/community/profile</c>, so the daemon has nowhere to read a
/// contributor's own row back from and reports what this device last
/// successfully wrote instead. It reflects what this machine last did, not
/// what the server holds now, and it is not an authorization input
/// anywhere.</para>
///
/// <para>The handle and bio here are public by construction. They may be
/// displayed; they still never reach a log line.</para>
/// </remarks>
public sealed class PublicProfileResult
{
    /// <summary>
    /// The daemon's own verdict on whether this contributor is listed.
    /// </summary>
    /// <remarks>
    /// This is what decides, rather than the shell inferring a verdict from
    /// the presence of a handle: the field exists to answer exactly this
    /// question, and a client that answered it some other way would be a
    /// second opinion about who is public.
    /// </remarks>
    [JsonPropertyName("on_roster")]
    public bool OnRoster { get; set; }

    [JsonPropertyName("handle")]
    public string? Handle { get; set; }

    [JsonPropertyName("bio")]
    public string? Bio { get; set; }

    [JsonPropertyName("public_since")]
    public string? PublicSince { get; set; }

    /// <summary>
    /// Null by contract today: the daemon knows the ingest origin it uploads
    /// to, not the origin the community website serves profiles from, and
    /// says so rather than inventing a link that would not resolve.
    /// </summary>
    /// <remarks>
    /// Read anyway, so the affordance can appear the day something supplies
    /// one, and so this client does not have to change shape when it does.
    /// </remarks>
    [JsonPropertyName("public_url")]
    public string? PublicUrl { get; set; }

    /// <summary>
    /// Whether the daemon wrote its own local copy of the profile.
    /// </summary>
    /// <remarks>
    /// <b>This is not whether the call worked.</b> By the time this field
    /// exists at all the server has already taken (or released) the handle;
    /// the flag reports only the local cache write that followed. A claim with
    /// <c>handle_persisted: false</c> is a PUBLISHED profile, and
    /// <see cref="PublicProfileCopy.PublishedSentence"/> is what says so.
    ///
    /// Absent means the daemon did not report it. Read as true, matching the
    /// Linux shell: the only thing the false branch adds is a warning about
    /// this window's own cache, and a daemon that said nothing has not earned
    /// that warning being shown.
    /// </remarks>
    [JsonPropertyName("handle_persisted")]
    public bool? HandlePersisted { get; set; }

    /// <summary>
    /// Present and true on a <c>clear_public_profile</c> answer.
    /// </summary>
    [JsonPropertyName("withdrawn")]
    public bool Withdrawn { get; set; }

    /// <summary>
    /// <see cref="HandlePersisted"/> with the absent case resolved. See its
    /// remarks for why absent reads as true.
    /// </summary>
    public bool CachedLocally => HandlePersisted ?? true;

    /// <summary>
    /// The handle to display, or null when this contributor is not listed.
    /// </summary>
    /// <remarks>
    /// Gated on <see cref="OnRoster"/> rather than on the handle being
    /// non-empty, so the daemon's verdict is the only thing that decides
    /// whether this window draws someone as public.
    /// </remarks>
    public string? ListedHandle =>
        OnRoster && !string.IsNullOrEmpty(Handle) ? Handle : null;

    /// <summary>Absent and empty are the same thing: no bio was published.</summary>
    public string PublishedBio => Bio ?? string.Empty;

    /// <summary>
    /// Parses a profile answer, or returns null when it cannot be read.
    /// </summary>
    /// <remarks>
    /// Null rather than an exception, for the same reason
    /// <see cref="PreviewSummary.Parse"/> does it: a panel that cannot read
    /// the answer has to fall back to saying so, and a second failure channel
    /// for what is one condition only multiplies the branches at the call
    /// site.
    /// </remarks>
    public static PublicProfileResult? Parse(JsonElement? result)
    {
        if (result is not { } element)
        {
            return null;
        }

        try
        {
            return element.Deserialize<PublicProfileResult>(DaemonProtocol.SerializerOptions);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// "On the roster since March 4, 2026", or null when the daemon reported
    /// no date.
    /// </summary>
    /// <remarks>
    /// Rendered here rather than in XAML because <c>x:Bind</c> does not
    /// implicitly call ToString, so every figure this app draws has to leave
    /// the view model as a string. Parsed leniently and shown in local time;
    /// a <c>public_since</c> this build cannot parse produces no line at all
    /// rather than a wrong one.
    /// </remarks>
    public string? OnRosterSinceLine()
    {
        if (!DateTimeOffset.TryParse(
                PublicSince,
                CultureInfo.InvariantCulture,
                DateTimeStyles.RoundtripKind,
                out DateTimeOffset since))
        {
            return null;
        }

        return PublicProfileCopy.OnRosterSince(
            since.ToLocalTime().ToString("MMMM d, yyyy", CultureInfo.CurrentCulture));
    }
}

/// <summary>
/// The parameters for <c>set_public_profile</c>.
/// </summary>
public static class PublicProfileRequest
{
    /// <summary>
    /// Serializes a claim.
    /// </summary>
    /// <remarks>
    /// <para><b>The bio key is always present.</b> The server upserts with
    /// <c>bio = excluded.bio</c>, so the <c>PUT</c> replaces the whole profile
    /// and there is no partial update to express; the daemon refuses an
    /// omitted <c>bio</c> outright rather than guessing. An empty box is
    /// <c>null</c>, not <c>""</c> -- a contributor who cleared the field is
    /// saying they want no bio, and that is what this sends.</para>
    ///
    /// <para>The handle is sent as typed, minus surrounding whitespace.
    /// Nothing here validates it: the daemon and the server share one copy of
    /// the handle rules, and a second copy in this shell is how a handle this
    /// window accepts becomes one the server refuses. A refusal comes back as
    /// a fixed label and is translated by
    /// <see cref="PublicProfileCopy.FailureSentence"/>.</para>
    /// </remarks>
    public static string Serialize(string? handle, string? bio)
    {
        string trimmedBio = (bio ?? string.Empty).Trim();

        return JsonSerializer.Serialize(new Dictionary<string, object?>
        {
            ["handle"] = (handle ?? string.Empty).Trim(),
            ["bio"] = trimmedBio.Length == 0 ? null : trimmedBio,
        });
    }
}
