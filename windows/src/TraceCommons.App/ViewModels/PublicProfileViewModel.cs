using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The Settings screen's public-profile panel: claim a handle, edit it, or
/// leave the roster.
///
/// Driven by three methods -- <c>get_public_profile</c>,
/// <c>set_public_profile</c>, <c>clear_public_profile</c> -- every one of
/// which was already in the daemon's pinned METHODS array. Nothing here adds
/// to it.
///
/// UI-thread-affine like <see cref="MainViewModel"/>, for the same reason:
/// <see cref="DaemonHost"/> hops before it raises anything.
/// </summary>
/// <remarks>
/// <para>The words live one layer down in
/// <see cref="PublicProfileCopy"/> so they can be tested off Windows, and
/// this class does not write copy of its own. In particular it does not
/// decide what a claim with <c>handle_persisted: false</c> says -- see
/// <see cref="PublicProfileCopy.PublishedSentence"/>, and the invariant test
/// behind it.</para>
///
/// <para><b>Nothing here is logged.</b> A handle and a bio are public by
/// construction, but they are contributor identity and never reach a log
/// line. Neither does a daemon error: it arrives as a fixed label and leaves
/// as a sentence.</para>
/// </remarks>
public sealed class PublicProfileViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;
    private PublicProfileResult _profile = new();
    private string _handle = string.Empty;
    private string _bio = string.Empty;
    private string _notice = string.Empty;
    private bool _isBusy;

    public PublicProfileViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    // The section's fixed labels, surfaced rather than typed into the markup.
    //
    // Every one of them is a sentence the three shells have to word
    // identically, and XAML is the one place a "small" wording change can be
    // made without a test noticing. Bound through here, they are the same
    // constants PublicProfileCopyTests compares whole against the Linux
    // shell's copy.rs.

    public string HeadingText => PublicProfileCopy.Heading;

    public string ListHandlePubliclyText => PublicProfileCopy.ListHandlePublicly;

    public string HandleLabelText => PublicProfileCopy.HandleLabel;

    public string BioLabelText => PublicProfileCopy.BioLabel;

    public string SaveProfileText => PublicProfileCopy.SaveProfile;

    public string LeaveRosterText => PublicProfileCopy.LeaveRoster;

    public string FootnoteText => PublicProfileCopy.Footnote;

    /// <summary>
    /// Whether the daemon reports this contributor as listed.
    /// </summary>
    /// <remarks>
    /// Off the roster this section is a single control that grants nothing;
    /// on it, it is a panel of the published fields. Two surfaces rather than
    /// two states of one, exactly as the Linux shell draws them.
    /// </remarks>
    public bool IsListed => _profile.ListedHandle is not null;

    public bool IsNotListed => !IsListed;

    /// <summary>The handle box. Two-way; nothing is sent until Save.</summary>
    public string Handle
    {
        get => _handle;
        set => Set(ref _handle, value ?? string.Empty);
    }

    /// <summary>The bio box. Two-way; nothing is sent until Save.</summary>
    public string Bio
    {
        get => _bio;
        set
        {
            if (Set(ref _bio, value ?? string.Empty))
            {
                Raise(nameof(BioCounterText));
            }
        }
    }

    /// <summary>
    /// "74/280", in UTF-8 bytes.
    /// </summary>
    /// <remarks>
    /// A string rather than a figure because <c>x:Bind</c> does not implicitly
    /// call ToString. It tracks the box rather than the value the panel was
    /// loaded with, and it refuses nothing: what happens at and above the
    /// limit is the server's call, and this window does not pre-empt it.
    /// </remarks>
    public string BioCounterText => PublicProfileCopy.BioCounter(_bio);

    /// <summary>
    /// "On the roster since March 4, 2026", or empty when the daemon reported
    /// no date it could read.
    /// </summary>
    public string OnRosterSinceText => _profile.OnRosterSinceLine() ?? string.Empty;

    public bool HasOnRosterSince => OnRosterSinceText.Length > 0;

    /// <summary>
    /// One sentence about the last thing that happened. Always fixed copy.
    /// </summary>
    public string Notice
    {
        get => _notice;
        private set
        {
            if (Set(ref _notice, value))
            {
                Raise(nameof(HasNotice));
            }
        }
    }

    public bool HasNotice => _notice.Length > 0;

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (Set(ref _isBusy, value))
            {
                Raise(nameof(IsNotBusy));
            }
        }
    }

    public bool IsNotBusy => !_isBusy;

    /// <summary>
    /// Reads the profile the daemon has cached for this device.
    /// </summary>
    /// <remarks>
    /// It IS a cache and the daemon says so: there is no
    /// <c>GET /v1/community/profile</c>, so the daemon has nowhere to read a
    /// contributor's own row back from and reports what this device last
    /// successfully wrote. A read failure leaves the panel as it was and says
    /// nothing: this call changes nothing public, so there is nothing to
    /// report about the public surface, and a notice here would be this
    /// window inventing an event.
    /// </remarks>
    public async Task LoadAsync()
    {
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.GetPublicProfile)
            .ConfigureAwait(true);

        if (response.IsError)
        {
            return;
        }

        Adopt(PublicProfileResult.Parse(response.Result));
    }

    /// <summary>
    /// Claims or re-publishes the handle, and reports what happened.
    /// </summary>
    /// <param name="handle">The handle to claim.</param>
    /// <param name="bio">The bio, or empty for none.</param>
    /// <returns>
    /// The daemon's fixed error label on a refusal, or null on success. The
    /// two call sites want the refusal in different places -- the dialog keeps
    /// it beside the field being corrected, the panel puts it in the section
    /// notice -- so this reports it rather than rendering it.
    /// </returns>
    /// <remarks>
    /// <para>Save re-publishes the WHOLE profile, because that is what the
    /// <c>PUT</c> does: the handle and the bio as they stand, both of them,
    /// every time. There is no partial update to offer.</para>
    ///
    /// <para><b>The claim is not gated on the local consent-scope list.</b>
    /// The server authorizes the <c>PUT</c> against the grant ceiling on the
    /// claim, not against the scopes this device happens to have recorded;
    /// the local set can be narrower than what the credential carries, and
    /// refusing here would refuse contributors the server would have allowed.
    /// The daemon deliberately makes the same choice, and so do the CLI and
    /// the other two shells.</para>
    ///
    /// <para>Nothing here validates the handle first, for the same reason: the
    /// daemon and the server share one copy of those rules.</para>
    /// </remarks>
    public async Task<string?> ClaimAsync(string? handle, string? bio)
    {
        try
        {
            IsBusy = true;

            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.SetPublicProfile,
                    PublicProfileRequest.Serialize(handle, bio))
                .ConfigureAwait(true);

            if (response.IsError)
            {
                // The label, not the message-as-prose: the caller decides
                // where a refusal belongs on screen.
                return response.Error!.Message;
            }

            PublicProfileResult? result = PublicProfileResult.Parse(response.Result);
            Adopt(result);

            // Rendered from the daemon's answer rather than from what was
            // typed, so the panel shows what was actually published.
            //
            // `handle_persisted` is NOT whether the claim worked: the server
            // has taken the handle by the time the flag exists at all. Both
            // branches therefore report a published profile.
            Notice = PublicProfileCopy.PublishedSentence(result?.CachedLocally ?? true);
            return null;
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// Saves the boxes as they stand.
    /// </summary>
    public async Task SaveAsync()
    {
        string? refusal = await ClaimAsync(_handle, _bio).ConfigureAwait(true);
        if (refusal is not null)
        {
            Notice = PublicProfileCopy.FailureSentence(refusal);
        }
    }

    /// <summary>
    /// Withdraws public attribution.
    /// </summary>
    /// <remarks>
    /// A refusal gets its own sentence, not the claim one: after a failed
    /// withdrawal the handle is still published, and "nothing was published"
    /// would read as the opposite of what happened.
    /// </remarks>
    public async Task LeaveRosterAsync()
    {
        try
        {
            IsBusy = true;

            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.ClearPublicProfile)
                .ConfigureAwait(true);

            if (response.IsError)
            {
                Notice = PublicProfileCopy.LeaveFailureSentence(response.Error!.Message);
                return;
            }

            PublicProfileResult? result = PublicProfileResult.Parse(response.Result);
            Adopt(result);
            Notice = PublicProfileCopy.LeftRosterSentence(result?.CachedLocally ?? true);
        }
        finally
        {
            IsBusy = false;
        }
    }

    /// <summary>
    /// Takes the daemon's answer as the panel's new state.
    /// </summary>
    /// <remarks>
    /// An unreadable answer is treated as "not listed" rather than left as
    /// whatever was on screen. The alternative is a panel that keeps showing a
    /// published handle after a call whose outcome it could not read, which is
    /// the one direction this surface must not fail in.
    /// </remarks>
    private void Adopt(PublicProfileResult? result)
    {
        _profile = result ?? new PublicProfileResult();
        _handle = _profile.ListedHandle ?? string.Empty;
        _bio = _profile.PublishedBio;

        Raise(nameof(IsListed));
        Raise(nameof(IsNotListed));
        Raise(nameof(Handle));
        Raise(nameof(Bio));
        Raise(nameof(BioCounterText));
        Raise(nameof(OnRosterSinceText));
        Raise(nameof(HasOnRosterSince));
    }

    private bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        Raise(name);
        return true;
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
