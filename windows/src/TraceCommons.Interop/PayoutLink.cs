using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The payout-link section's words (<c>tc_payout_link_copy_json</c>,
/// <c>PayoutLinkCopy</c>, PayoutLinkSurface.swift). Every string is a draft
/// for owner review; templates carry <c>{org_name}</c>. A payload with a
/// missing or blank field is refused whole.
/// </summary>
public sealed record PayoutLinkCopy(IReadOnlyDictionary<string, string> Words)
{
    /// <summary>The payload fields this shell decodes, by wire name (<c>consumedFields</c>).</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "scope", "button", "secondary", "accessibility_name",
        "unlink_button", "unlink_accessibility_name", "unlink_body",
        "change_button", "change_accessibility_name", "status_linked", "status_none", "status_waiting", "status_unavailable",
        "status_mismatch", "sign_in", "session_ended", "no_organization", "changed", "failed",
    };

    public string Title => Words["title"];
    public string Body => Words["body"];
    public string Scope => Words["scope"];
    public string Button => Words["button"];
    public string Secondary => Words["secondary"];
    public string AccessibilityName => Words["accessibility_name"];
    public string UnlinkButton => Words["unlink_button"];
    public string UnlinkAccessibilityName => Words["unlink_accessibility_name"];
    public string UnlinkBody => Words["unlink_body"];
    public string ChangeButton => Words["change_button"];
    public string ChangeAccessibilityName => Words["change_accessibility_name"];
    public string StatusLinked => Words["status_linked"];
    public string StatusNone => Words["status_none"];
    public string StatusWaiting => Words["status_waiting"];
    public string StatusUnavailable => Words["status_unavailable"];
    public string StatusMismatch => Words["status_mismatch"];
    public string SignIn => Words["sign_in"];
    public string SessionEnded => Words["session_ended"];
    public string NoOrganization => Words["no_organization"];
    public string Changed => Words["changed"];
    public string Failed => Words["failed"];

    public static PayoutLinkCopy? Decode(string? json) =>
        CopyFields.Read(json, ConsumedFields) is { } words ? new PayoutLinkCopy(words) : null;

    public static PayoutLinkCopy? Live => LiveCopy.Value;

    private static readonly Lazy<PayoutLinkCopy?> LiveCopy = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_payout_link_copy_json())));

    public static string Render(string template, string orgName) =>
        template.Replace("{org_name}", orgName, StringComparison.Ordinal);
}

/// <summary>One organization the account may send payouts to.</summary>
public sealed record PayoutOrganization(string Id, string Name);

/// <summary>The organizations listed, with the revision the link must name.</summary>
public sealed record PayoutOrganizationListing(IReadOnlyList<PayoutOrganization> Organizations, string OrgsRevision);

/// <summary>One payout rail as <c>payout_rails_status</c> relays it (<c>PayoutRailStatus</c>).</summary>
public sealed record PayoutRailStatus(
    string RailId = PayoutLinkSurface.NearAiRailId,
    string Mode = "live",
    bool Linked = false,
    string? State = null,
    long Version = 0,
    string? RequestState = null,
    bool UnlinkRequested = false,
    bool RecipientUnavailable = false,
    string? RecipientRefCheck = null,
    string? OrganizationName = null,
    string? OrganizationId = null)
{
    internal static PayoutRailStatus? From(JsonElement e)
    {
        if (e.ValueKind != JsonValueKind.Object
            || DaemonCalls.Text(e, "rail_id") is not { } rail
            || DaemonCalls.Text(e, "mode") is not { } mode
            || !e.TryGetProperty("linked", out JsonElement linked) || linked.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || !e.TryGetProperty("version", out JsonElement version) || !version.TryGetInt64(out long v))
        {
            return null;
        }

        static bool Flag(JsonElement o, string key) =>
            o.TryGetProperty(key, out JsonElement f) && f.ValueKind == JsonValueKind.True;

        return new PayoutRailStatus(
            rail, mode, linked.ValueKind == JsonValueKind.True, DaemonCalls.Text(e, "state"), v,
            DaemonCalls.Text(e, "request_state"), Flag(e, "unlink_requested"), Flag(e, "recipient_unavailable"),
            DaemonCalls.Text(e, "recipient_ref_check"), DaemonCalls.Text(e, "organization_name"),
            DaemonCalls.Text(e, "organization_id"));
    }
}

/// <summary>A payout-link refusal, routed from the daemon's label (<c>PayoutLinkRefusal</c>).</summary>
public enum PayoutLinkRefusal
{
    SignIn,
    SessionEnded,
    Changed,
    Failed,
}

public static class PayoutLinkRefusals
{
    public static PayoutLinkRefusal From(string label) => label switch
    {
        "payout-link-near-ai-sign-in-required" or "payout-link-near-ai-session-expired"
            or "payout-link-near-ai-session-changed" => PayoutLinkRefusal.SignIn,
        "account-session-required" => PayoutLinkRefusal.SessionEnded,
        "payout-link-orgs-changed" or "payout-link-changed" or "version_conflict" or "org_not_owned"
            or "payout_link_change_pending" => PayoutLinkRefusal.Changed,
        _ => PayoutLinkRefusal.Failed,
    };
}

/// <summary>The Mac's payout-link picker state (<c>PayoutPicker</c>).</summary>
public sealed class PayoutPicker
{
    public abstract record Phase
    {
        public sealed record Idle : Phase;
        public sealed record Loading : Phase;
        public sealed record Choosing : Phase;
        public sealed record NoOrganization : Phase;
        public sealed record Working : Phase;
        public sealed record Refused(PayoutLinkRefusal Refusal) : Phase;
    }

    public Phase Current { get; private set; } = new Phase.Idle();
    public PayoutOrganizationListing? Listing { get; private set; }
    public string? SelectedId { get; private set; }
    public bool Changing { get; private set; }

    public IReadOnlyList<PayoutOrganization> Rows => Listing?.Organizations ?? Array.Empty<PayoutOrganization>();

    public PayoutOrganization? Selected => SelectedId is null ? null : Rows.FirstOrDefault(o => o.Id == SelectedId);

    public void BeginLoading() => Current = new Phase.Loading();

    public void BeginChange()
    {
        Changing = true;
        SelectedId = null;
        Current = new Phase.Loading();
    }

    public void Listed(PayoutOrganizationListing listing)
    {
        Listing = listing;
        SelectedId = null;
        Current = listing.Organizations.Count == 0 ? new Phase.NoOrganization() : new Phase.Choosing();
    }

    public void Select(string? id)
    {
        if (Current is not Phase.Choosing || id is null || Rows.All(o => o.Id != id))
        {
            return;
        }

        SelectedId = id;
    }

    public void DismissSelection()
    {
        SelectedId = null;
        if (Changing)
        {
            Changing = false;
            Listing = null;
            Current = new Phase.Idle();
            return;
        }

        if (Current is Phase.Choosing || Current == new Phase.Refused(PayoutLinkRefusal.Failed))
        {
            Current = Rows.Count == 0 ? new Phase.Idle() : new Phase.Choosing();
        }
    }

    public void BeginWorking()
    {
        if (Selected is null || Current is not Phase.Choosing)
        {
            return;
        }

        Current = new Phase.Working();
    }

    public void Refuse(PayoutLinkRefusal refusal)
    {
        SelectedId = null;
        if (refusal != PayoutLinkRefusal.Failed)
        {
            Listing = null;
            Changing = false;
        }

        Current = new Phase.Refused(refusal);
    }

    public void Finished()
    {
        SelectedId = null;
        Changing = false;
        Listing = null;
        Current = new Phase.Idle();
    }

    /// <summary>The consent for the selected organization, shown before Choose.</summary>
    public sealed record Consent(string OrgName, string Body, string Scope, string Button, string AccessibilityName, string Secondary);

    public Consent? ConsentFor(PayoutLinkCopy copy)
    {
        if (Selected is not { } org || Current is not (Phase.Choosing or Phase.Working))
        {
            return null;
        }

        return new Consent(
            org.Name,
            PayoutLinkCopy.Render(copy.Body, org.Name),
            PayoutLinkCopy.Render(copy.Scope, org.Name),
            copy.Button,
            PayoutLinkCopy.Render(copy.AccessibilityName, org.Name),
            copy.Secondary);
    }
}

/// <summary>What the payout-link card shows (<c>PayoutLinkSurface</c>).</summary>
public sealed class PayoutLinkSurface
{
    public const string NearAiRailId = "near_ai";

    public PayoutLinkSurface(PayoutLinkCopy copy)
    {
        Copy = copy;
    }

    public PayoutLinkCopy Copy { get; }

    /// <summary>The NEAR AI rail, when the commons lists it live or as a dry run.</summary>
    public static PayoutRailStatus? NearAiRail(IEnumerable<PayoutRailStatus> rails) =>
        rails.FirstOrDefault(r => r.RailId == NearAiRailId && (r.Mode == "live" || r.Mode == "dry_run"));

    /// <summary>Only an ended session is said when the status read fails; anything else hides the card.</summary>
    public static PayoutLinkRefusal? ShownStatusRefusal(Exception error) =>
        error is DaemonRefusedException r && PayoutLinkRefusals.From(r.Label) == PayoutLinkRefusal.SessionEnded
            ? PayoutLinkRefusal.SessionEnded
            : null;

    public string? StatusLine(PayoutRailStatus rail)
    {
        if (rail.RecipientRefCheck == "mismatch")
        {
            return Copy.StatusMismatch;
        }

        if (rail.RecipientUnavailable)
        {
            return Copy.StatusUnavailable;
        }

        if (!rail.Linked || rail.UnlinkRequested)
        {
            return Copy.StatusNone;
        }

        return rail.State switch
        {
            "active" => Named(Copy.StatusLinked, rail),
            "waiting" => Named(Copy.StatusWaiting, rail),
            _ => Copy.StatusNone,
        };
    }

    private static string? Named(string template, PayoutRailStatus rail) =>
        rail.OrganizationName is { } name ? PayoutLinkCopy.Render(template, name) : null;

    public bool NeedsOrgName(PayoutRailStatus rail) => StatusLine(rail) is null;

    public static bool CanChoose(PayoutRailStatus rail) =>
        rail.RecipientRefCheck == "mismatch" || rail.RecipientUnavailable || !rail.Linked || rail.UnlinkRequested;

    public static bool CanUnlink(PayoutRailStatus rail) =>
        rail.Linked && !rail.UnlinkRequested && rail.State == "active" && rail.OrganizationName is not null;

    public static bool CanChange(PayoutRailStatus rail) =>
        rail.Linked && !rail.UnlinkRequested && rail.State == "active"
        && rail.RecipientRefCheck != "mismatch" && !rail.RecipientUnavailable;

    public static bool ShowsChangeButton(PayoutRailStatus rail, PayoutPicker picker)
    {
        if (!CanChange(rail))
        {
            return false;
        }

        if (!picker.Changing)
        {
            return true;
        }

        return picker.Current is not (PayoutPicker.Phase.Loading or PayoutPicker.Phase.Choosing or PayoutPicker.Phase.Working);
    }

    public static bool ShowsPicker(PayoutRailStatus rail, PayoutPicker picker) => CanChoose(rail) || picker.Changing;

    public string? UnlinkAccessibilityName(PayoutRailStatus rail) =>
        rail.OrganizationName is { } name ? PayoutLinkCopy.Render(Copy.UnlinkAccessibilityName, name) : null;

    public string RefusalLine(PayoutLinkRefusal refusal) => refusal switch
    {
        PayoutLinkRefusal.SignIn => Copy.SignIn,
        PayoutLinkRefusal.SessionEnded => Copy.SessionEnded,
        PayoutLinkRefusal.Changed => Copy.Changed,
        _ => Copy.Failed,
    };
}

/// <summary>The payout IPC calls (<c>PayoutLinkClient</c>), with the Mac's method names and parameters.</summary>
public sealed class PayoutLinkClient
{
    public const string StatusMethod = "payout_rails_status";
    public const string OrgsMethod = "payout_link_orgs";
    public const string StartMethod = "payout_link_start";
    public const string UnlinkMethod = "payout_link_unlink";
    public const string ConfirmMethod = "payout_link_confirm";

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public PayoutLinkClient(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
    }

    public async Task<IReadOnlyList<PayoutRailStatus>> StatusAsync()
    {
        JsonElement r = await DaemonCalls.ResultAsync(_call, StatusMethod, new Dictionary<string, object?>()).ConfigureAwait(true);
        if (!r.TryGetProperty("rails", out JsonElement rails) || rails.ValueKind != JsonValueKind.Array)
        {
            throw new DaemonRefusedException(DaemonCalls.Unparseable);
        }

        return rails.EnumerateArray()
            .Select(e => PayoutRailStatus.From(e) ?? throw new DaemonRefusedException(DaemonCalls.Unparseable))
            .ToArray();
    }

    public async Task<PayoutOrganizationListing> OrganizationsAsync()
    {
        JsonElement r = await DaemonCalls.ResultAsync(_call, OrgsMethod, new Dictionary<string, object?>()).ConfigureAwait(true);
        if (!r.TryGetProperty("organizations", out JsonElement orgs) || orgs.ValueKind != JsonValueKind.Array
            || DaemonCalls.Text(r, "orgs_revision") is not { } revision)
        {
            throw new DaemonRefusedException(DaemonCalls.Unparseable);
        }

        return new PayoutOrganizationListing(
            orgs.EnumerateArray()
                .Select(o => DaemonCalls.Text(o, "id") is { } id && DaemonCalls.Text(o, "name") is { } name
                    ? new PayoutOrganization(id, name)
                    : throw new DaemonRefusedException(DaemonCalls.Unparseable))
                .ToArray(),
            revision);
    }

    public Task StartLinkAsync(string railId, string organizationId, string orgsRevision, long expectedVersion) =>
        DaemonCalls.ResultAsync(_call, StartMethod, new Dictionary<string, object?>
        {
            ["rail_id"] = railId, ["organization_id"] = organizationId,
            ["orgs_revision"] = orgsRevision, ["expected_current_version"] = expectedVersion,
        });

    public Task StartUnlinkAsync(string railId, long expectedVersion) =>
        DaemonCalls.ResultAsync(_call, UnlinkMethod, new Dictionary<string, object?>
        {
            ["rail_id"] = railId, ["expected_current_version"] = expectedVersion,
        });

    public Task ConfirmLinkAsync(string railId, string organizationId) =>
        DaemonCalls.ResultAsync(_call, ConfirmMethod, new Dictionary<string, object?>
        {
            ["rail_id"] = railId, ["action"] = "link", ["organization_id"] = organizationId, ["confirmed"] = true,
        });

    public Task ConfirmUnlinkAsync(string railId) =>
        DaemonCalls.ResultAsync(_call, ConfirmMethod, new Dictionary<string, object?>
        {
            ["rail_id"] = railId, ["action"] = "unlink", ["confirmed"] = true,
        });
}

/// <summary>
/// The payout link's state over time (<c>PayoutLinkModel</c>,
/// PayoutLinkModel.swift): a refresh supersedes earlier ones, the org list
/// is read only when a name is missing or the person may choose, Choose and
/// Unlink are a start then a confirm, and every outcome re-reads the status.
/// Continuations resume on the caller's context; drive it from the UI thread.
/// </summary>
public sealed class PayoutLinkFlow
{
    private readonly PayoutLinkClient _client;
    private ulong _refreshGeneration;
    private ulong _organizationsGeneration;

    public PayoutLinkFlow(PayoutLinkClient client, PayoutLinkSurface? surface)
    {
        _client = client;
        Surface = surface;
    }

    public event Action? Changed;

    public PayoutLinkSurface? Surface { get; }
    public PayoutRailStatus? Rail { get; private set; }
    public PayoutPicker Picker { get; } = new();
    public bool Busy { get; private set; }
    public PayoutLinkRefusal? Refusal { get; private set; }
    public PayoutLinkRefusal? StatusRefusal { get; private set; }

    /// <summary>Whether the section draws a card: a rail to show, or a refusal to say.</summary>
    public bool Visible => Surface is not null && (Rail is not null || StatusRefusal is not null);

    public async Task RefreshAsync()
    {
        ulong generation = ++_refreshGeneration;
        if (Surface is null)
        {
            Rail = null;
            StatusRefusal = null;
            Changed?.Invoke();
            return;
        }

        try
        {
            IReadOnlyList<PayoutRailStatus> rails = await _client.StatusAsync().ConfigureAwait(true);
            if (generation != _refreshGeneration)
            {
                return;
            }

            Rail = PayoutLinkSurface.NearAiRail(rails);
            StatusRefusal = null;
        }
        catch (DaemonRefusedException e)
        {
            if (generation != _refreshGeneration)
            {
                return;
            }

            Rail = null;
            StatusRefusal = PayoutLinkSurface.ShownStatusRefusal(e);
        }

        Changed?.Invoke();
        if (Rail is { } rail && !Picker.Changing && (Surface.NeedsOrgName(rail) || PayoutLinkSurface.CanChoose(rail)))
        {
            await LoadOrganizationsAsync(generation).ConfigureAwait(true);
            if (generation != _refreshGeneration)
            {
                return;
            }

            try
            {
                IReadOnlyList<PayoutRailStatus> again = await _client.StatusAsync().ConfigureAwait(true);
                if (generation == _refreshGeneration)
                {
                    Rail = PayoutLinkSurface.NearAiRail(again);
                    Changed?.Invoke();
                }
            }
            catch (DaemonRefusedException)
            {
                // The first read stands.
            }
        }
    }

    private async Task LoadOrganizationsAsync(ulong? expectedRefresh)
    {
        if (Busy)
        {
            return;
        }

        ulong generation = ++_organizationsGeneration;
        Picker.BeginLoading();
        Changed?.Invoke();
        try
        {
            PayoutOrganizationListing listing = await _client.OrganizationsAsync().ConfigureAwait(true);
            if (generation != _organizationsGeneration || (expectedRefresh is { } r && r != _refreshGeneration))
            {
                return;
            }

            Refusal = null;
            Picker.Listed(listing);
        }
        catch (DaemonRefusedException e)
        {
            if (generation != _organizationsGeneration || (expectedRefresh is { } r && r != _refreshGeneration))
            {
                return;
            }

            PayoutLinkRefusal kind = PayoutLinkRefusals.From(e.Label);
            Refusal = kind;
            Picker.Refuse(kind);
        }

        Changed?.Invoke();
    }

    public async Task ChangeAsync()
    {
        if (Busy)
        {
            return;
        }

        Picker.BeginChange();
        await LoadOrganizationsAsync(null).ConfigureAwait(true);
    }

    public void Select(string? id)
    {
        Picker.Select(id);
        Changed?.Invoke();
    }

    public void NotNow()
    {
        Picker.DismissSelection();
        Changed?.Invoke();
    }

    public async Task ChooseAsync()
    {
        if (Rail is not { } rail || Picker.Selected is not { } org || Picker.Listing?.OrgsRevision is not { } revision || Busy)
        {
            return;
        }

        Picker.BeginWorking();
        Busy = true;
        Changed?.Invoke();
        await FinishAsync(async () =>
        {
            await _client.StartLinkAsync(PayoutLinkSurface.NearAiRailId, org.Id, revision, rail.Version).ConfigureAwait(true);
            await _client.ConfirmLinkAsync(PayoutLinkSurface.NearAiRailId, org.Id).ConfigureAwait(true);
        }).ConfigureAwait(true);
    }

    public async Task StopAsync()
    {
        if (Rail is not { } rail || Busy)
        {
            return;
        }

        Busy = true;
        Changed?.Invoke();
        await FinishAsync(async () =>
        {
            await _client.StartUnlinkAsync(PayoutLinkSurface.NearAiRailId, rail.Version).ConfigureAwait(true);
            await _client.ConfirmUnlinkAsync(PayoutLinkSurface.NearAiRailId).ConfigureAwait(true);
        }).ConfigureAwait(true);
    }

    private async Task FinishAsync(Func<Task> work)
    {
        try
        {
            await work().ConfigureAwait(true);
            Refusal = null;
            Picker.Finished();
        }
        catch (DaemonRefusedException e)
        {
            PayoutLinkRefusal kind = PayoutLinkRefusals.From(e.Label);
            Refusal = kind;
            Picker.Refuse(kind);
        }

        // Re-read: a refused change may mean the link moved underneath.
        Busy = false;
        Changed?.Invoke();
        await RefreshAsync().ConfigureAwait(true);
    }
}
