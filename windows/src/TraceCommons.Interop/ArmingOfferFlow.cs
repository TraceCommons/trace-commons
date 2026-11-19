using System;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The arming offer's words, decoded from <c>tc_arming_offer_copy_json</c>
/// (<c>project_copy::arming_offer_copy</c>), as the Mac decodes them
/// (<c>ProjectArmingCopy</c>, macos/Sources/TCShellCore/ProjectArmingCopy.swift).
/// </summary>
/// <remarks>
/// Arming is the strongest thing this window can be set to do, so the card is
/// not drawn without these words: a failed decode is no card, never a
/// C#-written one (ProjectArmingCopy.swift:13-15).
/// </remarks>
public sealed record ProjectArmingCopy(string Evidence, string Question, string Confirm, string Decline, string Body)
{
    /// <summary>The core's words for <paramref name="offer"/>, or null.</summary>
    public static ProjectArmingCopy? For(ArmingOffer offer) =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_arming_offer_copy_json(
            offer.ProjectLabel, (uint)Math.Max(0, offer.ContributedCount))));

    /// <summary>
    /// The payload, or null when it does not parse or one of the card's fields
    /// is missing or empty (ProjectArmingCopy.swift:55-66).
    /// </summary>
    public static ProjectArmingCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            string? Field(string name) =>
                root.ValueKind == JsonValueKind.Object
                && root.TryGetProperty(name, out JsonElement value)
                && value.ValueKind == JsonValueKind.String
                && value.GetString() is { Length: > 0 } text
                    ? text
                    : null;

            return Field("evidence") is { } evidence
                && Field("question") is { } question
                && Field("confirm") is { } confirm
                && Field("decline") is { } decline
                && Field("body") is { } body
                    ? new ProjectArmingCopy(evidence, question, confirm, decline, body)
                    : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>Where the offer's Enable is (<c>ArmingOfferStep</c>, ArmingOfferMotion.swift:5-8).</summary>
public enum ArmingOfferStep
{
    /// Sent to the daemon; Enable is locked.
    Arming,

    /// The daemon armed the project; the mark is a check until the card leaves.
    Armed,
}

/// <summary>
/// The arming offer's card and its two answers, settled against the action
/// pressed (checklist L1), as the Mac runs them
/// (<c>acceptArmingOffer</c>, <c>declineArmingOffer</c>, <c>clearArmingOffer</c>,
/// AppModel.swift:2219-2258 at origin/main 409066936).
/// </summary>
/// <remarks>
/// <para>
/// The card leaves only on a confirmed reply. Enable locks from the press
/// until the card goes; a refusal unlocks it and keeps the card, with the
/// core's <c>project_mode_failed</c> line (AppModel.swift:4749-4754). A thrown
/// transport failure is a refusal, never an escape from an <c>async void</c>
/// click handler.
/// </para>
/// <para>
/// Each press carries its identity: the project, the moment, and the account
/// generation it was pressed under. A reply settles only that press. One that
/// lands after the offer moved on, or after an account boundary
/// (<see cref="AccountBoundary"/>), changes nothing.
/// </para>
/// <para>
/// <c>policy-write-failed</c> and <c>queue-write-failed</c> come back after
/// the daemon has already armed the folder in memory
/// (<c>handle_set_project_mode</c>, crates/trace-commons-contributor/src/daemon/ipc.rs).
/// The Mac shows its refusal line for every failure and does not re-read; here
/// those two also re-read, so the daemon's next <c>arming_suggestion</c>
/// decides whether the card stays, and the line clears with it.
/// </para>
/// </remarks>
public sealed class ArmingOfferFlow
{
    /// <summary>The two refusals that follow a change the daemon already made in memory.</summary>
    public static readonly string[] WrittenInMemory = { "policy-write-failed", "queue-write-failed" };

    /// <summary>How long the armed card stays before it leaves (<c>ArmingOfferMotion.hold</c>, ArmingOfferMotion.swift:18).</summary>
    public static readonly TimeSpan Hold = TimeSpan.FromMilliseconds(900);

    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<Task> _refresh;
    private readonly Func<TimeSpan, Task> _delay;
    private readonly Func<string> _refusalLine;
    private readonly Func<DateTimeOffset> _now;
    private long _generation;
    private bool _declining;
    private string? _refusedProject;

    /// <param name="call">The daemon call: method, params JSON.</param>
    /// <param name="refresh">Re-reads the projects, the offer and the change log.</param>
    /// <param name="delay">Waits out <see cref="Hold"/>.</param>
    /// <param name="refusalLine">The core's <c>settings.project_mode_failed</c>.</param>
    /// <param name="now">The clock a press is stamped with.</param>
    public ArmingOfferFlow(
        Func<string, string, Task<DaemonResponse>> call,
        Func<Task> refresh,
        Func<TimeSpan, Task> delay,
        Func<string> refusalLine,
        Func<DateTimeOffset>? now = null)
    {
        _call = call;
        _refresh = refresh;
        _delay = delay;
        _refusalLine = refusalLine;
        _now = now ?? (() => DateTimeOffset.UtcNow);
    }

    /// <summary>Raised on every change of <see cref="Offer"/>, <see cref="Step"/> or <see cref="Refusal"/>.</summary>
    public event Action? Changed;

    /// <summary>The offer on screen; null draws no card.</summary>
    public ArmingOffer? Offer { get; private set; }

    /// <summary>Where Enable is; null before a press.</summary>
    public ArmingOfferStep? Step { get; private set; }

    /// <summary>The core's line after a refusal; empty otherwise.</summary>
    public string Refusal { get; private set; } = string.Empty;

    /// <summary>Enable is pressed once: locked from the press until the card goes (ArmingOfferMotion.swift:31-34).</summary>
    public bool EnableLocked => Step is not null;

    /// <summary>
    /// Whether the card's last departure was an armed card leaving after its
    /// hold, so it leaves on the arming exit spring. Set before
    /// <see cref="Changed"/> fires on every change of <see cref="Offer"/>.
    /// </summary>
    public bool LeftArmed { get; private set; }

    /// <summary>Not now is locked while its own call is out, and while Enable's is.</summary>
    public bool DeclineLocked => _declining || Step is not null;

    /// <summary>
    /// A fresh <c>arming_suggestion</c> answer: the card the person pressed
    /// stays until its own exit (<c>offerAfterRefresh</c>, ArmingOfferMotion.swift:47-54).
    /// A refusal line leaves once the offer it was about is gone.
    /// </summary>
    public void OnRefreshed(ArmingOffer? answered)
    {
        ArmingOffer? next = Step is null ? answered : Offer;
        bool changed = !Equals(next, Offer);
        if (changed)
        {
            LeftArmed = false;
        }

        Offer = next;
        if (_refusedProject is not null && next?.ProjectId != _refusedProject)
        {
            changed |= ClearRefusal();
        }

        if (changed)
        {
            Changed?.Invoke();
        }
    }

    /// <summary>
    /// A sign-out, unenroll or a status naming another account: the card goes,
    /// and every reply still out is from an older account and is not applied.
    /// </summary>
    public void AccountBoundary()
    {
        _generation++;
        _declining = false;
        LeftArmed = false;
        Offer = null;
        Step = null;
        ClearRefusal();
        Changed?.Invoke();
    }

    /// <summary>Enable: arms the offered project with <c>set_project_mode</c> <c>auto_upload</c>.</summary>
    public async Task AcceptAsync()
    {
        if (Offer is not { } pressed || Step is not null || _declining)
        {
            return;
        }

        var press = new Press(pressed, _now(), _generation);
        Step = ArmingOfferStep.Arming;
        ClearRefusal();
        Changed?.Invoke();

        DaemonError? error = await SendAsync(
            DaemonProtocol.Methods.SetProjectMode,
            $$"""{"project_id":{{JsonSerializer.Serialize(pressed.ProjectId)}},"mode":"auto_upload"}""").ConfigureAwait(true);
        if (!IsCurrent(press))
        {
            return;
        }

        if (error is not null)
        {
            Step = null;
            Refuse(pressed.ProjectId);
            Changed?.Invoke();
            if (Array.IndexOf(WrittenInMemory, error.Message) >= 0)
            {
                await RefreshQuietlyAsync().ConfigureAwait(true);
            }

            return;
        }

        Step = ArmingOfferStep.Armed;
        Changed?.Invoke();
        // The hold starts at the reply and the re-read runs beside it, as the
        // Mac's do (AppModel.swift:2228-2235): a slow or failed re-read never
        // keeps the checked card up longer, and never strands it.
        Task reread = RefreshQuietlyAsync();
        try
        {
            await _delay(Hold).ConfigureAwait(true);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            // A cancelled hold still ends with the card gone.
        }
        finally
        {
            if (IsCurrent(press) && Step == ArmingOfferStep.Armed)
            {
                LeftArmed = true;
                Offer = null;
                Step = null;
                Changed?.Invoke();
            }
        }

        _ = reread;
    }

    /// <summary>The re-read, whose failure is the next refresh's to report, never the click's.</summary>
    private async Task RefreshQuietlyAsync()
    {
        try
        {
            await _refresh().ConfigureAwait(true);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            System.Diagnostics.Trace.TraceWarning("arming re-read failed: " + ex.GetType().Name);
        }
    }

    /// <summary>
    /// Not now: the daemon silences the offer for thirty days
    /// (<c>declineArmingOffer</c>, AppModel.swift:2241-2246). The card leaves
    /// on the daemon's yes only; a refusal keeps it.
    /// </summary>
    public async Task DeclineAsync()
    {
        if (Offer is not { } pressed || Step is not null || _declining)
        {
            return;
        }

        var press = new Press(pressed, _now(), _generation);
        _declining = true;
        Changed?.Invoke();
        DaemonError? error = await SendAsync(
            DaemonProtocol.Methods.DeclineArming,
            $$"""{"project_id":{{JsonSerializer.Serialize(pressed.ProjectId)}}}""").ConfigureAwait(true);
        if (press.Generation != _generation)
        {
            return;
        }

        _declining = false;
        if (error is null && IsCurrent(press))
        {
            LeftArmed = false;
            Offer = null;
            ClearRefusal();
        }

        Changed?.Invoke();
    }

    private bool IsCurrent(Press press) =>
        press.Generation == _generation && Equals(Offer, press.Offer);

    private void Refuse(string projectId)
    {
        Refusal = _refusalLine();
        _refusedProject = projectId;
    }

    private bool ClearRefusal()
    {
        bool had = Refusal.Length > 0 || _refusedProject is not null;
        Refusal = string.Empty;
        _refusedProject = null;
        return had;
    }

    /// <summary>The call's error, read whether it came back as data or was thrown.</summary>
    private async Task<DaemonError?> SendAsync(string method, string parameters)
    {
        try
        {
            DaemonResponse response = await _call(method, parameters).ConfigureAwait(true);
            return response.Error;
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            return new DaemonError { Code = "transport", Message = "transport-failed" };
        }
    }

    private sealed record Press(ArmingOffer Offer, DateTimeOffset At, long Generation);
}
