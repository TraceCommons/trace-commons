using System;
using System.ComponentModel;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The offer to move a legacy invite identity to the contributor's NEAR AI
/// account, as the main window's notices card and the Settings section draw
/// it. One instance serves both, so they cannot disagree about whether a move
/// is running or the offer is dismissed.
/// </summary>
/// <remarks>
/// Not one sentence is written here. The words are
/// <see cref="LegacyMigrationOfferCopy"/> and the refusal lines the move
/// returns, all from the Rust across the C ABI; the controls are shown only
/// while the daemon says <c>status.legacy_invite_migration.offered</c>, and
/// only when the words arrived whole. The card, and not Settings, can be
/// dismissed for good on this device, keyed by a digest of the tenant.
/// </remarks>
public sealed class LegacyMigrationOfferViewModel : INotifyPropertyChanged
{
    private readonly LegacyMigrationDismissals _dismissals;
    private readonly LegacyMigrationOfferCopy? _copy;
    private bool _offered;
    private string? _tenantId;
    private string _invite = string.Empty;

    public LegacyMigrationOfferViewModel(
        LegacyMigrationMove move,
        LegacyMigrationDismissals? dismissals = null,
        LegacyMigrationOfferCopy? copy = null)
    {
        Move = move ?? throw new ArgumentNullException(nameof(move));
        _dismissals = dismissals ?? LegacyMigrationDismissals.Default();
        _copy = copy ?? LegacyMigrationOffers.Copy();
        Move.PropertyChanged += (_, _) => Raise();
    }

    public LegacyMigrationMove Move { get; }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>
    /// Takes the offered flag and tenant from one status read.
    /// <paramref name="migration"/> is <c>status.legacy_invite_migration</c>
    /// as the daemon sent it.
    /// </summary>
    public void SetStatus(JsonElement? migration, string? tenantId)
    {
        bool offered = LegacyMigrationOffers.IsOffered(migration);
        if (offered == _offered && tenantId == _tenantId)
        {
            return;
        }

        _offered = offered;
        _tenantId = tenantId;
        Raise();
    }

    /// <summary>The daemon offers the move and its words arrived whole.</summary>
    public bool IsAvailable => _offered && _copy is not null;

    /// <summary>The notices card: available, and not put away on this device.</summary>
    public bool ShowsCard => IsAvailable && !_dismissals.IsDismissed(_tenantId);

    public string Title => _copy?.Title ?? string.Empty;

    public string Body => _copy?.Body ?? string.Empty;

    public string Action => _copy?.Action ?? string.Empty;

    public string Dismiss => _copy?.Dismiss ?? string.Empty;

    public string Later => _copy?.Later ?? string.Empty;

    public string Cancel => _copy?.PasskeyCancel ?? string.Empty;

    public string InvitePrompt => _copy?.InvitePrompt ?? string.Empty;

    /// <summary>The paste field is shown only after the daemon asked for the invite.</summary>
    public bool NeedsInvite => Move.InviteNeeded;

    public string Invite
    {
        get => _invite;
        set
        {
            if (_invite == value)
            {
                return;
            }

            _invite = value;
            Raise();
        }
    }

    /// <summary>The move is running, or waiting on the passkey.</summary>
    public bool IsWorking => Move.Busy;

    /// <summary>The working line, or the passkey line while the browser step is open.</summary>
    public string WorkingLine => Move.AwaitingPasskey
        ? _copy?.PasskeyWaiting ?? string.Empty
        : Move.Busy ? _copy?.Working ?? string.Empty : string.Empty;

    /// <summary>The Cancel on the passkey wait. Nothing is moved.</summary>
    public bool CanCancel => Move.AwaitingPasskey;

    public bool CanMove => !Move.Busy;

    /// <summary>
    /// The core's refusal line, else the passkey step's failure, else the
    /// could-not-start line. Empty when there is none.
    /// </summary>
    public string Problem =>
        Move.Refusal
        ?? (Move.PasskeyFailed ? _copy?.PasskeyFailed
            : Move.StartFailed ? _copy?.StartFailed
            : null)
        ?? string.Empty;

    public bool HasProblem => Problem.Length > 0;

    /// <summary>Move, sending the pasted invite only once the daemon asked for it.</summary>
    public Task MoveAsync() => Move.MoveAsync(Invite);

    public Task CancelAsync() => Move.CancelPasskeyAsync();

    /// <summary>Puts the card away for good on this device. The move stays in Settings.</summary>
    public void DismissCard()
    {
        _dismissals.Dismiss(_tenantId);
        Raise();
    }

    private void Raise() =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(null));
}
