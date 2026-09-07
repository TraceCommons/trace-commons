using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What one coding tool's row on the model-calls screen says about it.
///
/// <para>
/// The labels are the daemon's own strings and the mapping onto these values
/// is the shared C ABI's, not this shell's. A value this build does not know
/// is <see cref="Unknown"/>, which is the safe direction: the dangerous claim
/// here is <see cref="Answering"/>, and a state a later daemon grows must
/// never be painted as it.
/// </para>
/// </summary>
public enum HarnessState
{
    /// <summary>A state this build has no words for. Claims nothing.</summary>
    Unknown,

    /// <summary>The tool's own settings send its calls wherever they went before.</summary>
    NotConnected,

    /// <summary>
    /// The tool's settings name this computer and nothing has arrived from
    /// it. A value in a file is not evidence that a call was ever answered.
    /// </summary>
    ConnectedNoCalls,

    /// <summary>
    /// A call arrived and was answered here. The only value that means the
    /// tool works, and the only one that may be painted as working.
    /// </summary>
    Answering,

    /// <summary>
    /// A call arrived in this tool's protocol family and more than one
    /// connected tool speaks that family, so it cannot be attributed to
    /// either. Its own value, not a flavour of <see cref="Answering"/>.
    /// </summary>
    ActivityShared,
}

/// <summary>
/// What working out one tool's edit turned out to be.
/// </summary>
/// <remarks>
/// The branch that matters is <see cref="Unparseable"/> against
/// <see cref="Noop"/>: one is "nothing to change", the other is "we refused to
/// rewrite a file we could not read, and it needs a human".
/// <see cref="Changes"/> alone is committable, and <see cref="Changes"/> alone
/// comes back with a plan id.
/// </remarks>
public enum HarnessPlanOutcome
{
    Unknown,
    Changes,
    Noop,
    Unparseable,
    NotInstalled,
    EntryUnusable,
    NoConfigPath,
}

/// <summary>
/// A setting the contributor already had a value in, which was left exactly
/// as it was.
/// </summary>
/// <remarks>
/// Reported, never offered. Nothing in this shell may pair one of these with
/// an action that takes the slot: "fill an empty slot but leave a full one
/// alone" is one of the three rules that make editing somebody else's config
/// file acceptable at all.
/// </remarks>
public sealed record HarnessOccupiedSlot
{
    [JsonPropertyName("slot")]
    public string Slot { get; init; } = string.Empty;

    [JsonPropertyName("current")]
    public string Current { get; init; } = string.Empty;
}

/// <summary>One coding tool, as <c>harness_list</c> describes it.</summary>
public sealed record HarnessRow
{
    [JsonPropertyName("id")]
    public string Id { get; init; } = string.Empty;

    /// <summary>
    /// The tool's own name, as the shared crate reports it. No shell and no
    /// copy constant spells one: a tool renamed upstream must not go on being
    /// called the old thing here.
    /// </summary>
    [JsonPropertyName("name")]
    public string Name { get; init; } = string.Empty;

    [JsonPropertyName("installed")]
    public bool Installed { get; init; }

    /// <summary>
    /// Whether the tool's settings file names this computer. Proof about a
    /// file, and no evidence at all about a call -- see
    /// <see cref="HarnessState"/>.
    /// </summary>
    [JsonPropertyName("connected")]
    public bool Connected { get; init; }

    /// <summary>
    /// The file an action here would change, or null when none was located.
    /// Shown to the contributor and never written to a log line.
    /// </summary>
    [JsonPropertyName("config_path")]
    public string? ConfigPath { get; init; }

    /// <summary>
    /// The command that does the same thing outside this app, verbatim. A
    /// contributor who would rather not have an app edit their files gets the
    /// way to do it themselves.
    /// </summary>
    [JsonPropertyName("connect_command")]
    public string ConnectCommand { get; init; } = string.Empty;

    /// <summary>The protocol family, or null when the tool declares none.</summary>
    [JsonPropertyName("family")]
    public string? Family { get; init; }

    [JsonPropertyName("state")]
    public string StateLabel { get; init; } = string.Empty;

    /// <summary>
    /// When a call in this tool's family last arrived, as the daemon spelled
    /// it, or null.
    /// </summary>
    /// <remarks>
    /// Carried as the daemon spelled it. The sentence that says when is
    /// assembled in the Rust and crosses the ABI as
    /// <c>tc_harness_last_call_line</c>; see
    /// <see cref="HarnessSurface.LastCallSentence(string?, DateTimeOffset)"/>.
    /// Composing one here would be a fourth place the wording could drift.
    /// </remarks>
    [JsonPropertyName("last_call_at")]
    public string? LastCallAt { get; init; }

    [JsonPropertyName("can_connect")]
    public bool? CanConnectReported { get; init; }

    [JsonPropertyName("can_disconnect")]
    public bool? CanDisconnectReported { get; init; }

    /// <summary>
    /// Whether the connect action may be offered. The daemon's answer when it
    /// gave one, and the shared branch table's otherwise -- both come from the
    /// same function, so a shell may read either.
    /// </summary>
    public bool CanConnect =>
        CanConnectReported
        ?? HarnessSurface.ActionAvailable(HarnessSurface.Connect, Installed, Connected);

    /// <summary>
    /// Whether the disconnect action may be offered.
    /// </summary>
    /// <remarks>
    /// A tool that is not installed is still shown, and still offers this:
    /// uninstalling a coding tool does not remove the line we put in its
    /// config file, and "remove only what we put there" is worth nothing if
    /// the control that does the removing is hidden.
    /// </remarks>
    public bool CanDisconnect =>
        CanDisconnectReported
        ?? HarnessSurface.ActionAvailable(HarnessSurface.Disconnect, Installed, Connected);

    /// <summary>This row's state, through the shared branch table.</summary>
    public HarnessState State => HarnessSurface.State(StateLabel);

    /// <summary>Whether this row may be painted as working. Answering alone.</summary>
    public bool ReadsAsWorking => HarnessSurface.ReadsAsWorking(State);

    public bool HasConfigPath => !string.IsNullOrEmpty(ConfigPath);
}

/// <summary>The whole <c>harness_list</c> answer.</summary>
public sealed record HarnessListing
{
    public static readonly HarnessListing Empty = new();

    /// <summary>
    /// Whether a signed catalog was loaded. False means the list is the tools
    /// this build ships knowing about, which is a fact about the build and
    /// not about the machine.
    /// </summary>
    [JsonPropertyName("catalog_present")]
    public bool CatalogPresent { get; init; }

    /// <summary>The port this computer answers model calls on, or null.</summary>
    [JsonPropertyName("destination_port")]
    public ushort? DestinationPort { get; init; }

    [JsonPropertyName("harnesses")]
    public IReadOnlyList<HarnessRow> Harnesses { get; init; } = Array.Empty<HarnessRow>();

    /// <summary>
    /// Whether the ledger answered at all. False is no evidence about any
    /// tool, which is not the same thing as "no calls yet" and must not be
    /// drawn as it.
    /// </summary>
    public bool ActivityReadable { get; init; }
}

/// <summary>An edit that has been worked out and not made.</summary>
public sealed record HarnessPlan
{
    [JsonPropertyName("id")]
    public string Id { get; init; } = string.Empty;

    [JsonPropertyName("action")]
    public string ActionLabel { get; init; } = string.Empty;

    [JsonPropertyName("outcome")]
    public string OutcomeLabel { get; init; } = string.Empty;

    /// <summary>
    /// The daemon's permission to write this file once, or null. The shell
    /// hands it back and adds nothing to it: it cannot construct a write of
    /// its own.
    /// </summary>
    [JsonPropertyName("plan_id")]
    public string? PlanId { get; init; }

    [JsonPropertyName("path")]
    public string? Path { get; init; }

    /// <summary>What would change, in the shared crate's words. Shown verbatim.</summary>
    [JsonPropertyName("changes")]
    public IReadOnlyList<string> Changes { get; init; } = Array.Empty<string>();

    /// <summary>
    /// Slots left exactly as the contributor had them.
    /// </summary>
    /// <remarks>
    /// NOT an outcome. This rides alongside whatever <see cref="Outcome"/>
    /// says -- a plan can carry changes and an occupied slot at once -- so it
    /// is rendered whatever the outcome was, and never as an error.
    /// </remarks>
    [JsonPropertyName("occupied")]
    public IReadOnlyList<HarnessOccupiedSlot> Occupied { get; init; } =
        Array.Empty<HarnessOccupiedSlot>();

    public HarnessPlanOutcome Outcome => HarnessSurface.Outcome(OutcomeLabel);

    /// <summary>
    /// Whether this plan may be committed: a known committable outcome AND a
    /// plan id. Both, because either alone would let an outcome from a later
    /// daemon through.
    /// </summary>
    public bool IsCommittable =>
        Outcome == HarnessPlanOutcome.Changes && !string.IsNullOrEmpty(PlanId);

    public bool HasChanges => Changes.Count > 0;

    public bool HasOccupied => Occupied.Count > 0;
}

/// <summary>An edit that was made.</summary>
public sealed record HarnessCommit
{
    [JsonPropertyName("id")]
    public string Id { get; init; } = string.Empty;

    [JsonPropertyName("action")]
    public string ActionLabel { get; init; } = string.Empty;

    [JsonPropertyName("committed")]
    public bool Committed { get; init; }

    [JsonPropertyName("path")]
    public string? Path { get; init; }

    /// <summary>
    /// The copy of the file as it was before anything touched it, or null
    /// when none was written. Written once and never overwritten, so this is
    /// not a fresh backup per change and must not be described as one.
    /// </summary>
    [JsonPropertyName("backup_path")]
    public string? BackupPath { get; init; }
}

/// <summary>
/// The harness list, across the C ABI and the daemon socket.
///
/// <para>
/// Holds no words and owns no branch. Every sentence comes from
/// <c>private_inference_copy.rs</c> through <see cref="PrivateInferenceCopy"/>,
/// and every decision -- what a state means, what an outcome means, which
/// action may be offered -- comes from the shared branch tables, because a
/// branch written three times in three languages agrees today and drifts in
/// silence tomorrow.
/// </para>
/// </summary>
public static class HarnessSurface
{
    /// <summary>The <c>harness_plan</c> action that points a tool here.</summary>
    public const string Connect = "connect";

    /// <summary>The <c>harness_plan</c> action that takes a tool back off.</summary>
    public const string Disconnect = "disconnect";

    /// <summary>A connect was asked for with nothing answering on this computer.</summary>
    private const string NoDestinationCode = "harness-no-destination";

    /// <summary>
    /// The whole list, or an empty one. Malformed input is an empty list
    /// rather than a throw, for the reason <see cref="DaemonResponse.Parse"/>
    /// gives: the caller already has to handle "the daemon did not answer
    /// usefully", and a second failure channel for the same condition only
    /// multiplies the branches.
    /// </summary>
    public static HarnessListing ParseListing(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return HarnessListing.Empty;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            JsonElement root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return HarnessListing.Empty;
            }

            var rows = new List<HarnessRow>();
            if (root.TryGetProperty("harnesses", out JsonElement harnesses) &&
                harnesses.ValueKind == JsonValueKind.Array)
            {
                foreach (JsonElement element in harnesses.EnumerateArray())
                {
                    if (ParseRow(element) is { } row)
                    {
                        rows.Add(row);
                    }
                }
            }

            return new HarnessListing
            {
                CatalogPresent = ReadBool(root, "catalog_present") ?? false,
                DestinationPort = ReadPort(root),
                Harnesses = rows,
                ActivityReadable =
                    root.TryGetProperty("activity", out JsonElement activity) &&
                    activity.ValueKind == JsonValueKind.Object &&
                    (ReadBool(activity, "readable") ?? false),
            };
        }
        catch (JsonException)
        {
            return HarnessListing.Empty;
        }
    }

    /// <summary>The rows alone, for a caller that wants nothing else.</summary>
    public static IReadOnlyList<HarnessRow> ParseRows(string? json) =>
        ParseListing(json).Harnesses;

    /// <summary>
    /// One worked-out plan, or null. A row missing anything a preview needs
    /// is dropped rather than guessed: a dialog that offered to write a file
    /// it could not name is exactly what the two-step exists to prevent.
    /// </summary>
    public static HarnessPlan? ParsePlan(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            HarnessPlan? plan = JsonSerializer.Deserialize<HarnessPlan>(json);
            if (plan is null || string.IsNullOrEmpty(plan.Id) || string.IsNullOrEmpty(plan.OutcomeLabel))
            {
                return null;
            }

            return plan;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>One made edit, or null.</summary>
    public static HarnessCommit? ParseCommit(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            return JsonSerializer.Deserialize<HarnessCommit>(json);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The <c>harness_plan</c> request body for one tool and direction.</summary>
    public static string SerializePlan(string id, string action) =>
        JsonSerializer.Serialize(new Dictionary<string, string> { ["id"] = id, ["action"] = action });

    /// <summary>
    /// The <c>harness_commit</c> request body.
    /// </summary>
    /// <remarks>
    /// The plan id and nothing else. Not the tool, not the action, not the
    /// path: everything the write needs is held by the daemon under that id,
    /// so a shell cannot assemble a write of its own, and cannot commit
    /// anything a contributor was not shown.
    /// </remarks>
    public static string SerializeCommit(string planId) =>
        JsonSerializer.Serialize(new Dictionary<string, string> { ["plan_id"] = planId });

    /// <summary>One <c>harness_list</c> row's state, through the shared table.</summary>
    public static HarnessState State(string? label) =>
        NativeMethods.tc_harness_state_code(label ?? string.Empty) switch
        {
            AbiStateNotConnected => HarnessState.NotConnected,
            AbiStateConnectedNoCalls => HarnessState.ConnectedNoCalls,
            AbiStateAnswering => HarnessState.Answering,
            AbiStateActivityShared => HarnessState.ActivityShared,
            _ => HarnessState.Unknown,
        };

    /// <summary>One <c>harness_plan</c> outcome, through the shared table.</summary>
    public static HarnessPlanOutcome Outcome(string? label) =>
        NativeMethods.tc_harness_plan_outcome_code(label ?? string.Empty) switch
        {
            AbiPlanChanges => HarnessPlanOutcome.Changes,
            AbiPlanNoop => HarnessPlanOutcome.Noop,
            AbiPlanUnparseable => HarnessPlanOutcome.Unparseable,
            AbiPlanNotInstalled => HarnessPlanOutcome.NotInstalled,
            AbiPlanEntryUnusable => HarnessPlanOutcome.EntryUnusable,
            AbiPlanNoConfigPath => HarnessPlanOutcome.NoConfigPath,
            _ => HarnessPlanOutcome.Unknown,
        };

    /// <summary>Whether one action may be offered for a tool in this state.</summary>
    public static bool ActionAvailable(string action, bool installed, bool connected) =>
        NativeMethods.tc_harness_action_available(action, installed ? 1 : 0, connected ? 1 : 0) != 0;

    /// <summary>
    /// Whether a row may be painted as working. <see cref="HarnessState.Answering"/>
    /// alone.
    /// </summary>
    /// <remarks>
    /// <see cref="HarnessState.ActivityShared"/> is deliberately outside this.
    /// A call did arrive, but two connected tools speak the family it arrived
    /// in and the ledger records a family, not a tool -- so the row may say
    /// what this computer did and may not say that this tool is the one doing
    /// it.
    /// </remarks>
    public static bool ReadsAsWorking(HarnessState state) => state == HarnessState.Answering;

    /// <summary>
    /// The sentence for one row's state label, or the empty string -- drawn
    /// as no line at all.
    /// </summary>
    /// <remarks>
    /// ONE TABLE, ASKED. This used to be a <c>switch</c> over the payload's
    /// fields, and the macOS and GNOME shells each held their own: three
    /// copies of one decision, agreeing today and drifting in silence
    /// tomorrow. It takes the LABEL and not the decoded state, so a state a
    /// later daemon grows never has to be spelled in C# before it can be
    /// shown.
    ///
    /// Three of the five states have a sentence and two deliberately do not.
    /// The shared table answers the empty string for
    /// <see cref="HarnessState.ActivityShared"/> rather than letting it borrow
    /// the answering sentence, which would claim an attribution the ledger
    /// cannot make -- it records a protocol family, and a family two connected
    /// tools both speak names neither of them -- or the nothing-seen one,
    /// which would be false: something did arrive.
    /// <see cref="HarnessState.Unknown"/> is the same shape for a different
    /// reason: a state this build has no words for claims nothing rather than
    /// borrowing the nearest sentence. Both render at a non-clear tone with no
    /// line, which is the honest gap.
    /// </remarks>
    public static string StateSentence(string? stateLabel) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_harness_state_line(stateLabel))
        ?? string.Empty;

    /// <summary>
    /// The sentence one row shows, which is not always its state's.
    /// </summary>
    /// <remarks>
    /// A tool that is not on this machine gets the missing-tool sentence and
    /// nothing else. Both halves matter. The row is LISTED rather than
    /// hidden, because a tool left out cannot be told apart from a tool this
    /// app was never taught about; and it may not keep
    /// <see cref="PrivateInferenceCopy.HarnessNotConnected"/>, which says a
    /// tool's own settings still send its calls wherever they went before --
    /// a claim about the settings of something that is not here. Before this
    /// the two rendered identically, with the connect button simply absent
    /// and nothing saying why.
    /// </remarks>
    public static string RowSentence(HarnessRow row, PrivateInferenceCopy copy) =>
        row.Installed ? StateSentence(row.StateLabel) : copy.HarnessNotInstalled;

    /// <summary>
    /// The sentence a plan's outcome carries, or the empty string -- drawn as
    /// no line at all.
    /// </summary>
    /// <remarks>
    /// ONE TABLE, ASKED. This used to be a
    /// <c>== HarnessPlanOutcome.Unparseable</c> branch in the view picking one
    /// of the payload's fields, and the macOS and GNOME shells each held the
    /// same arm; the four other non-committable outcomes had no sentence at
    /// all, so their preview held a title, a path, no changes, no explanation
    /// and a way out. It takes the LABEL and not the decoded outcome, so an
    /// outcome a later daemon grows never has to be spelled in C# before it
    /// can be shown.
    ///
    /// <c>changes</c> answers the empty string, and the emptiness is the
    /// point: the preview shows the changes themselves, and a sentence above
    /// them announcing that there are changes is this app narrating its own
    /// list.
    /// </remarks>
    public static string OutcomeSentence(string? outcomeLabel) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_harness_outcome_line(outcomeLabel))
        ?? string.Empty;

    /// <summary>
    /// When the last call from this tool was answered here, or the empty
    /// string.
    /// </summary>
    /// <remarks>
    /// Assembled on the far side, like the state sentence: this shell works
    /// out how many seconds ago and hands the number over. An absent or
    /// unparseable timestamp -- and one dated in the future -- crosses as a
    /// negative number, which is the shared convention for "nothing to
    /// report" and comes back empty.
    ///
    /// Before this crossed the ABI the sentence existed in Rust and was
    /// reachable only by the shell that links the crate natively, so this app
    /// drew nothing where the GNOME one drew a line.
    /// </remarks>
    public static string LastCallSentence(string? lastCallAt, DateTimeOffset now)
    {
        long secondsAgo = -1;
        if (DateTimeOffset.TryParse(
                lastCallAt,
                CultureInfo.InvariantCulture,
                DateTimeStyles.AdjustToUniversal | DateTimeStyles.AssumeUniversal,
                out DateTimeOffset at))
        {
            double elapsed = (now - at).TotalSeconds;
            secondsAgo = elapsed >= 0 && elapsed < long.MaxValue ? (long)elapsed : -1;
        }

        return NativeMethods.TakeOwnedString(NativeMethods.tc_harness_last_call_line(secondsAgo))
            ?? string.Empty;
    }

    /// <summary>
    /// Whether a refused commit leaves anything to commit again. It never
    /// does.
    /// </summary>
    /// <remarks>
    /// THIS IS ALWAYS FALSE, AND IT IS A FUNCTION SO THAT THE REASON HAS
    /// SOMEWHERE TO LIVE. The daemon takes the plan out of its store BEFORE it
    /// re-checks the file digest and before it writes, so an expired plan, a
    /// spent one, a file that moved underneath it and a write that failed all
    /// leave the same thing behind: no plan. Every one of them is closed the
    /// same way -- the preview goes, the unconfirmed-write sentence is shown,
    /// the list is read again -- and none of them is offered a retry, because
    /// there is nothing to retry. A shell that re-planned automatically here
    /// would be silently re-deciding a write on a contributor's behalf.
    /// </remarks>
    public static bool CommitIsRetryable(DaemonError? error) => false;

    /// <summary>
    /// Whether a refused plan means nothing answers on this computer yet.
    /// </summary>
    /// <remarks>
    /// A fact about this computer, not about the tool's file: no config may
    /// name a destination before there is one. The way out is the exposure
    /// question and the switch, not a retry.
    /// </remarks>
    public static bool NoDestination(DaemonError? error) => error?.Message == NoDestinationCode;

    /// <summary>
    /// Whether a connect must put the exposure question in front of the
    /// contributor first.
    /// </summary>
    /// <remarks>
    /// DELIBERATELY WIDER THAN
    /// <see cref="PrivateInferenceSurface.ShouldOffer(bool, bool, bool)"/>, and
    /// deliberately not built on it. The first-run offer asks once and records
    /// that it asked; this asks whenever a connect would REOPEN the listener.
    /// A contributor who accepted months ago and has since used the kill switch
    /// is making the exposure decision afresh -- the switch being off is the
    /// state they chose -- and a connect that quietly turned it back on would
    /// be spending an answer given about a different moment.
    ///
    /// The shape matches macOS's <c>connectNeedsExposure(listenerOn:)</c> one
    /// for one, on purpose: this is a branch about interrupting somebody, and
    /// three shells each deciding it differently is three different products.
    /// <c>ShouldOffer</c> itself is untouched.
    /// </remarks>
    public static bool ConnectNeedsExposure(bool listenerOn) => !listenerOn;

    private static HarnessRow? ParseRow(JsonElement element)
    {
        if (element.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        // A row is dropped rather than guessed. Every one of these is
        // load-bearing: an id nothing can be planned against, a name nothing
        // can be labelled with, and the two booleans the action buttons come
        // off. A default for any of them would be an invention.
        if (ReadString(element, "id") is not { Length: > 0 } id ||
            ReadString(element, "name") is not { Length: > 0 } name ||
            ReadBool(element, "installed") is not { } installed ||
            ReadBool(element, "connected") is not { } connected ||
            element.TryGetProperty("state", out JsonElement state) is false ||
            state.ValueKind != JsonValueKind.String)
        {
            return null;
        }

        return new HarnessRow
        {
            Id = id,
            Name = name,
            Installed = installed,
            Connected = connected,
            ConfigPath = ReadString(element, "config_path"),
            ConnectCommand = ReadString(element, "connect_command") ?? string.Empty,
            Family = ReadString(element, "family"),
            StateLabel = state.GetString() ?? string.Empty,
            LastCallAt = ReadString(element, "last_call_at"),
            CanConnectReported = ReadBool(element, "can_connect"),
            CanDisconnectReported = ReadBool(element, "can_disconnect"),
        };
    }

    private static string? ReadString(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static bool? ReadBool(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value)
            ? value.ValueKind switch
            {
                JsonValueKind.True => true,
                JsonValueKind.False => false,
                _ => null,
            }
            : null;

    private static ushort? ReadPort(JsonElement root) =>
        root.TryGetProperty("destination_port", out JsonElement value) &&
        value.ValueKind == JsonValueKind.Number &&
        value.TryGetUInt16(out ushort port)
            ? port
            : null;

    private const int AbiStateNotConnected = 31;
    private const int AbiStateConnectedNoCalls = 32;
    private const int AbiStateAnswering = 33;
    private const int AbiStateActivityShared = 34;

    private const int AbiPlanChanges = 41;
    private const int AbiPlanNoop = 42;
    private const int AbiPlanUnparseable = 43;
    private const int AbiPlanNotInstalled = 44;
    private const int AbiPlanEntryUnusable = 45;
    private const int AbiPlanNoConfigPath = 46;
}
