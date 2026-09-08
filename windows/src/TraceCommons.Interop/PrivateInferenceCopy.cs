using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// Every fixed sentence on the private-inference offer and settings card, as
/// <c>tc_private_inference_copy</c> exports it.
///
/// A pure carrier. Nothing here is authored in this shell: the three shells
/// print one offer, and the paragraph most at risk of being paraphrased is
/// the one saying what turning the switch on exposes.
///
/// Every property defaults to the empty string so a malformed payload cannot
/// throw during deserialisation; <see cref="PrivateInferenceSurface.Parse"/>
/// then refuses the whole object rather than handing a screen a blank where a
/// sentence should be.
/// </summary>
public sealed record PrivateInferenceCopy
{
    [JsonPropertyName("offer_title")]
    public string OfferTitle { get; init; } = string.Empty;

    [JsonPropertyName("offer_what")]
    public string OfferWhat { get; init; } = string.Empty;

    /// <summary>
    /// What turning the switch on exposes. The one sentence this surface will
    /// not render without.
    /// </summary>
    [JsonPropertyName("offer_exposure")]
    public string OfferExposure { get; init; } = string.Empty;

    [JsonPropertyName("offer_no_repoint")]
    public string OfferNoRepoint { get; init; } = string.Empty;

    [JsonPropertyName("offer_accept")]
    public string OfferAccept { get; init; } = string.Empty;

    [JsonPropertyName("offer_decline")]
    public string OfferDecline { get; init; } = string.Empty;

    [JsonPropertyName("offer_asked_once")]
    public string OfferAskedOnce { get; init; } = string.Empty;

    /// <summary>
    /// The rail label for the top-level destination this surface owns.
    ///
    /// Read rather than typed. A shell that spelled the label itself would
    /// keep spelling the old one after a rename in the Rust, and this is the
    /// one word in the whole surface a contributor navigates by.
    /// </summary>
    [JsonPropertyName("destination")]
    public string Destination { get; init; } = string.Empty;

    /// <summary>The one line under the destination's title saying what it is for.</summary>
    [JsonPropertyName("subtitle")]
    public string Subtitle { get; init; } = string.Empty;

    [JsonPropertyName("settings_title")]
    public string SettingsTitle { get; init; } = string.Empty;

    [JsonPropertyName("settings_toggle")]
    public string SettingsToggle { get; init; } = string.Empty;

    [JsonPropertyName("settings_applies_at_once")]
    public string SettingsAppliesAtOnce { get; init; } = string.Empty;

    [JsonPropertyName("state_unreported")]
    public string StateUnreported { get; init; } = string.Empty;

    [JsonPropertyName("state_unknown")]
    public string StateUnknown { get; init; } = string.Empty;

    [JsonPropertyName("state_stopping")]
    public string StateStopping { get; init; } = string.Empty;

    [JsonPropertyName("state_off")]
    public string StateOff { get; init; } = string.Empty;

    [JsonPropertyName("state_running")]
    public string StateRunning { get; init; } = string.Empty;

    [JsonPropertyName("state_running_no_backends")]
    public string StateRunningNoBackends { get; init; } = string.Empty;

    [JsonPropertyName("state_running_answered_elsewhere")]
    public string StateRunningAnsweredElsewhere { get; init; } = string.Empty;

    [JsonPropertyName("state_running_destination_unknown")]
    public string StateRunningDestinationUnknown { get; init; } = string.Empty;

    [JsonPropertyName("state_running_elsewhere")]
    public string StateRunningElsewhere { get; init; } = string.Empty;

    [JsonPropertyName("state_port_in_use")]
    public string StatePortInUse { get; init; } = string.Empty;

    [JsonPropertyName("state_start_failed")]
    public string StateStartFailed { get; init; } = string.Empty;

    [JsonPropertyName("state_crashed")]
    public string StateCrashed { get; init; } = string.Empty;

    /// <summary>
    /// The extra line the quit confirmation carries while the switch is on.
    /// The rest of that dialog is authored in this shell; this sentence
    /// deliberately is not.
    /// </summary>
    [JsonPropertyName("quit_also_stops")]
    public string QuitAlsoStops { get; init; } = string.Empty;

    /// <summary>A write could not be confirmed; persistence may still have happened.</summary>
    [JsonPropertyName("write_unconfirmed")]
    public string WriteUnconfirmed { get; init; } = string.Empty;

    /// <summary>The sentence the settings card shows once the control has moved out of it.</summary>
    [JsonPropertyName("settings_moved")]
    public string SettingsMoved { get; init; } = string.Empty;

    /// <summary>The tray action while it is on. Turning it off needs no sentence in front of it.</summary>
    [JsonPropertyName("tray_turn_off")]
    public string TrayTurnOff { get; init; } = string.Empty;

    /// <summary>
    /// The tray action while it is off. Opens the screen rather than acting: turning it ON
    /// changes what anything else on this computer may send through, which is not a decision
    /// to take from a menu with the consequence off-screen.
    /// </summary>
    [JsonPropertyName("tray_open_to_turn_on")]
    public string TrayOpenToTurnOn { get; init; } = string.Empty;

    /// <summary>The heading over the list of tools found on this computer.</summary>
    [JsonPropertyName("harnesses_title")]
    public string HarnessesTitle { get; init; } = string.Empty;

    /// <summary>
    /// The line under that heading. It says the choice is made one tool at a
    /// time, and that the list is what this app knows how to look for rather
    /// than a claim about every tool that exists.
    /// </summary>
    [JsonPropertyName("harnesses_what")]
    public string HarnessesWhat { get; init; } = string.Empty;

    /// <summary>
    /// What the amount <see cref="HarnessSurface.SpendSentence"/> names does
    /// and does not cover: only calls answered on this computer, and not work
    /// a monthly plan has already paid for. Drawn beside that sentence and
    /// only when that sentence is drawn -- a scope line on its own qualifies
    /// a figure that is not on screen.
    /// </summary>
    [JsonPropertyName("harnesses_spend_scope")]
    public string HarnessesSpendScope { get; init; } = string.Empty;

    [JsonPropertyName("harness_not_connected")]
    public string HarnessNotConnected { get; init; } = string.Empty;

    /// <summary>
    /// Settings are right and nothing has arrived yet. Never drawn the same
    /// way as <see cref="HarnessAnswering"/>: a value in a file is not
    /// evidence that a call was ever answered.
    /// </summary>
    [JsonPropertyName("harness_connected_nothing_seen")]
    public string HarnessConnectedNothingSeen { get; init; } = string.Empty;

    /// <summary>The only per-harness state that means a call was answered.</summary>
    [JsonPropertyName("harness_answering")]
    public string HarnessAnswering { get; init; } = string.Empty;

    [JsonPropertyName("harness_connect")]
    public string HarnessConnect { get; init; } = string.Empty;

    [JsonPropertyName("harness_disconnect")]
    public string HarnessDisconnect { get; init; } = string.Empty;

    /// <summary>The heading over the preview shown before anything is written.</summary>
    [JsonPropertyName("harness_preview_title")]
    public string HarnessPreviewTitle { get; init; } = string.Empty;

    [JsonPropertyName("harness_preview_confirm")]
    public string HarnessPreviewConfirm { get; init; } = string.Empty;

    [JsonPropertyName("harness_preview_cancel")]
    public string HarnessPreviewCancel { get; init; } = string.Empty;

    /// <summary>
    /// A slot that already had a value in it, which was left alone. Reported,
    /// never offered: this must not be drawn as a fault to be cleared, and no
    /// shell may pair it with an action that takes the slot.
    /// </summary>
    [JsonPropertyName("harness_slot_taken")]
    public string HarnessSlotTaken { get; init; } = string.Empty;

    /// <summary>A tool holding an old setting in a process that is still running.</summary>
    [JsonPropertyName("harness_needs_restart")]
    public string HarnessNeedsRestart { get; init; } = string.Empty;

    /// <summary>No tool was found, said in terms of what was looked for.</summary>
    [JsonPropertyName("harnesses_none_found")]
    public string HarnessesNoneFound { get; init; } = string.Empty;

    /// <summary>A settings file that could not be read, and was therefore refused.</summary>
    [JsonPropertyName("harness_unreadable_config")]
    public string HarnessUnreadableConfig { get; init; } = string.Empty;

    /// <summary>
    /// A tool this app could not find on this computer.
    /// </summary>
    /// <remarks>
    /// Rendered INSTEAD of <see cref="HarnessNotConnected"/> and never beside
    /// it: that sentence says a tool's own settings still send its calls
    /// wherever they went before, which is a claim about the settings of
    /// something that is not on this computer. The row is listed and says
    /// this rather than being hidden, because a tool left out of the list
    /// cannot be told apart from a tool this app was never taught about.
    /// </remarks>
    [JsonPropertyName("harness_not_installed")]
    public string HarnessNotInstalled { get; init; } = string.Empty;

    /// <summary>A plan that found the file already saying what was wanted.</summary>
    [JsonPropertyName("harness_plan_nothing_to_change")]
    public string HarnessPlanNothingToChange { get; init; } = string.Empty;

    /// <summary>This app's own description of a tool did not survive checking.</summary>
    [JsonPropertyName("harness_plan_entry_unusable")]
    public string HarnessPlanEntryUnusable { get; init; } = string.Empty;

    /// <summary>This build could not work out where a tool keeps its settings.</summary>
    [JsonPropertyName("harness_plan_no_config_path")]
    public string HarnessPlanNoConfigPath { get; init; } = string.Empty;

    // ---------------------------------------------------------------------
    // The NEAR AI credential.
    //
    // A key minted at a third party and kept on this machine. NOTHING HERE
    // CARRIES A HOLE FOR ONE: no sentence names a key, a prefix, an id or an
    // account, and no shell may add a field that would.
    // ---------------------------------------------------------------------

    [JsonPropertyName("credential_title")]
    public string CredentialTitle { get; init; } = string.Empty;

    [JsonPropertyName("credential_what")]
    public string CredentialWhat { get; init; } = string.Empty;

    /// <summary>
    /// What obtaining one costs, in all three of its consequences: a browser
    /// opens, the contributor signs in with a company that is not this app,
    /// and a key is minted and kept here.
    /// </summary>
    /// <remarks>
    /// The counterpart of <see cref="OfferExposure"/> and held to the same
    /// rule: it must be on screen wherever the obtain action is offered.
    /// <c>NearAiCredentialSurface.ActionPreamble</c> is what pairs them, so
    /// no shell decides for itself that a button may appear without it.
    /// </remarks>
    [JsonPropertyName("credential_cost")]
    public string CredentialCost { get; init; } = string.Empty;

    [JsonPropertyName("credential_obtain")]
    public string CredentialObtain { get; init; } = string.Empty;

    [JsonPropertyName("credential_cancel")]
    public string CredentialCancel { get; init; } = string.Empty;

    [JsonPropertyName("credential_forget")]
    public string CredentialForget { get; init; } = string.Empty;

    /// <summary>
    /// That forgetting is local: the key stays valid at the service until the
    /// contributor removes it in their own account there. Drawn wherever the
    /// forget action is, for the same reason the cost sentence is drawn
    /// wherever the obtain action is.
    /// </summary>
    [JsonPropertyName("credential_forget_explains")]
    public string CredentialForgetExplains { get; init; } = string.Empty;

    [JsonPropertyName("credential_absent")]
    public string CredentialAbsent { get; init; } = string.Empty;

    [JsonPropertyName("credential_obtaining")]
    public string CredentialObtaining { get; init; } = string.Empty;

    [JsonPropertyName("credential_failed")]
    public string CredentialFailed { get; init; } = string.Empty;

    [JsonPropertyName("credential_cancelled")]
    public string CredentialCancelled { get; init; } = string.Empty;

    [JsonPropertyName("credential_present")]
    public string CredentialPresent { get; init; } = string.Empty;

    /// <summary>
    /// A state label this build has never heard of. It says the state could
    /// not be read, and it must never degrade to
    /// <see cref="CredentialAbsent"/>: that is a claim about what this
    /// machine holds, and inventing it is how a contributor is invited to
    /// sign in a second time.
    /// </summary>
    [JsonPropertyName("credential_unknown")]
    public string CredentialUnknown { get; init; } = string.Empty;

    /// <summary>
    /// A daemon that does not answer the question at all. Distinct from
    /// <see cref="CredentialUnknown"/> and, like it, not a claim that no key
    /// is kept here.
    /// </summary>
    [JsonPropertyName("credential_unreported")]
    public string CredentialUnreported { get; init; } = string.Empty;

    /// <summary>
    /// Why a connect control is not on offer: this app would host the
    /// answering and holds no key.
    /// </summary>
    /// <remarks>
    /// Drawn ONLY through <see cref="HarnessSurface.CredentialNotice"/> and
    /// never on this shell's own reading of a boolean. The unread case is a
    /// third fact rather than a false one: a daemon that predates the gate
    /// connects tools without a sign-in, and telling somebody otherwise would
    /// be false.
    /// </remarks>
    [JsonPropertyName("harness_needs_credential")]
    public string HarnessNeedsCredential { get; init; } = string.Empty;

    /// <summary>Every sentence for the complete-payload check, not a rendering order.</summary>
    public string[] Sentences =>
        new[]
        {
            OfferTitle,
            OfferWhat,
            OfferExposure,
            OfferNoRepoint,
            OfferAccept,
            OfferDecline,
            OfferAskedOnce,
            Destination,
            Subtitle,
            SettingsTitle,
            SettingsToggle,
            SettingsAppliesAtOnce,
            StateUnreported,
            StateUnknown,
            StateStopping,
            StateOff,
            StateRunning,
            StateRunningNoBackends,
            StateRunningAnsweredElsewhere,
            StateRunningDestinationUnknown,
            StateRunningElsewhere,
            StatePortInUse,
            StateStartFailed,
            StateCrashed,
            QuitAlsoStops,
            WriteUnconfirmed,
            SettingsMoved,
            TrayTurnOff,
            TrayOpenToTurnOn,
            HarnessesTitle,
            HarnessesWhat,
            HarnessesSpendScope,
            HarnessNotConnected,
            HarnessConnectedNothingSeen,
            HarnessAnswering,
            HarnessConnect,
            HarnessDisconnect,
            HarnessPreviewTitle,
            HarnessPreviewConfirm,
            HarnessPreviewCancel,
            HarnessSlotTaken,
            HarnessNeedsRestart,
            HarnessesNoneFound,
            HarnessUnreadableConfig,
            HarnessNotInstalled,
            HarnessPlanNothingToChange,
            HarnessPlanEntryUnusable,
            HarnessPlanNoConfigPath,
            CredentialTitle,
            CredentialWhat,
            CredentialCost,
            CredentialObtain,
            CredentialCancel,
            CredentialForget,
            CredentialForgetExplains,
            CredentialAbsent,
            CredentialObtaining,
            CredentialFailed,
            CredentialCancelled,
            CredentialPresent,
            CredentialUnknown,
            CredentialUnreported,
            HarnessNeedsCredential,
        };
}
