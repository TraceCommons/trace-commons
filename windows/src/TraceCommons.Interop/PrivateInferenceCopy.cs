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

    /// <summary>
    /// Enrolling this device with the NEAR AI login a contributor already has,
    /// instead of a NEAR wallet. Both paths are offered; neither is removed.
    /// </summary>
    [JsonPropertyName("near_ai_enroll_title")]
    public string NearAiEnrollTitle { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_what")]
    public string NearAiEnrollWhat { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_action")]
    public string NearAiEnrollAction { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_needs_login")]
    public string NearAiEnrollNeedsLogin { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_working")]
    public string NearAiEnrollWorking { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_done")]
    public string NearAiEnrollDone { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_already_enrolled")]
    public string NearAiEnrollAlreadyEnrolled { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_no_session")]
    public string NearAiEnrollNoSession { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_endpoint_refused")]
    public string NearAiEnrollEndpointRefused { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_token_unavailable")]
    public string NearAiEnrollTokenUnavailable { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_start_failed")]
    public string NearAiEnrollStartFailed { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_commons_unreachable")]
    public string NearAiEnrollCommonsUnreachable { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_commons_unsupported")]
    public string NearAiEnrollCommonsUnsupported { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_invalid")]
    public string NearAiEnrollInvalid { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_verification_failed")]
    public string NearAiEnrollVerificationFailed { get; init; } = string.Empty;

    [JsonPropertyName("near_ai_enroll_unavailable")]
    public string NearAiEnrollUnavailable { get; init; } = string.Empty;

    [JsonPropertyName("credential_title")]
    public string CredentialTitle { get; init; } = string.Empty;

    [JsonPropertyName("credential_what")]
    public string CredentialWhat { get; init; } = string.Empty;

    [JsonPropertyName("credential_provider_label")]
    public string CredentialProviderLabel { get; init; } = string.Empty;

    [JsonPropertyName("credential_provider_github")]
    public string CredentialProviderGithub { get; init; } = string.Empty;

    [JsonPropertyName("credential_provider_google")]
    public string CredentialProviderGoogle { get; init; } = string.Empty;

    [JsonPropertyName("credential_provider_near")]
    public string CredentialProviderNear { get; init; } = string.Empty;

    [JsonPropertyName("credential_wallet_notice")]
    public string CredentialWalletNotice { get; init; } = string.Empty;

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

    /// <summary>
    /// The four <c>eligibility</c> states a queue entry can carry, and the
    /// thirteen <c>eligibility_reason</c> labels.
    ///
    /// Rendered through <see cref="ContributionEligibilitySurface"/>, which
    /// asks the ABI rather than these properties; they are carried here so
    /// this shell's complete-payload check pins the set it was built against.
    /// </summary>
    /// <summary>The cheap checks pass and the admission marker is there. A well-founded
    /// expectation and NEVER a guarantee -- the expensive checks still run at
    /// submit, and the server decides admission.</summary>
    [JsonPropertyName("eligibility_eligible")]
    public string EligibilityEligible { get; init; } = string.Empty;

    /// <summary>Nothing the contributor does will change this session. The sentence says
    /// so plainly enough that nobody retries it.</summary>
    [JsonPropertyName("eligibility_ineligible_permanent")]
    public string EligibilityIneligiblePermanent { get; init; } = string.Empty;

    /// <summary>This session stays ineligible; a setting decides whether future ones are.
    /// The only state whose sentence names a setting -- a row stays about its
    /// own session, and advice about the next one is guidance, not status.</summary>
    [JsonPropertyName("eligibility_ineligible_configuration")]
    public string EligibilityIneligibleConfiguration { get; init; } = string.Empty;

    /// <summary>Not worked out. The fallback for a state this build cannot read, and it
    /// MUST NOT degrade to an ineligibility: "could not tell" turned into "no"
    /// invites a contributor to conclude something false about their own work.
    ///
    /// <para>
    /// Not the sentence for an ABSENT <c>eligibility</c> field either. An
    /// invited contributor has no eligibility question, and their rows carry
    /// no sentence at all. See
    /// <see cref="ContributionEligibility.Absent"/>.
    /// </para></summary>
    [JsonPropertyName("eligibility_unknown")]
    public string EligibilityUnknown { get; init; } = string.Empty;

    /// <summary>No model call was answered here while the session ran.</summary>
    [JsonPropertyName("eligibility_reason_no_call")]
    public string EligibilityReasonNoCall { get; init; } = string.Empty;

    /// <summary>The final hop kept no copy of what was said.</summary>
    [JsonPropertyName("eligibility_reason_capture_off")]
    public string EligibilityReasonCaptureOff { get; init; } = string.Empty;

    /// <summary>A restarted, cancelled or truncated stream.</summary>
    [JsonPropertyName("eligibility_reason_digest_absent")]
    public string EligibilityReasonDigestAbsent { get; init; } = string.Empty;

    /// <summary>No provider identifier, so no receipt is reachable.</summary>
    [JsonPropertyName("eligibility_reason_upstream_id_absent")]
    public string EligibilityReasonUpstreamIdAbsent { get; init; } = string.Empty;

    /// <summary>What was kept does not match what was sent.</summary>
    [JsonPropertyName("eligibility_reason_digest_mismatch")]
    public string EligibilityReasonDigestMismatch { get; init; } = string.Empty;

    /// <summary>The note saying where the kept copy lives is unreadable.</summary>
    [JsonPropertyName("eligibility_reason_reference_malformed")]
    public string EligibilityReasonReferenceMalformed { get; init; } = string.Empty;

    /// <summary>The kept copy could not be read back.</summary>
    [JsonPropertyName("eligibility_reason_bodies_unreadable")]
    public string EligibilityReasonBodiesUnreadable { get; init; } = string.Empty;

    /// <summary>The kept copy is not text this app can read.</summary>
    [JsonPropertyName("eligibility_reason_body_not_utf8")]
    public string EligibilityReasonBodyNotUtf8 { get; init; } = string.Empty;

    /// <summary>The kept copy is past the size this check will read.</summary>
    [JsonPropertyName("eligibility_reason_body_too_large")]
    public string EligibilityReasonBodyTooLarge { get; init; } = string.Empty;

    /// <summary>This computer keeps no copy of the model calls it answers.</summary>
    [JsonPropertyName("eligibility_reason_evidence_capture_off")]
    public string EligibilityReasonEvidenceCaptureOff { get; init; } = string.Empty;

    /// <summary>The final call went out without the mark a contribution needs.</summary>
    [JsonPropertyName("eligibility_reason_marker_absent")]
    public string EligibilityReasonMarkerAbsent { get; init; } = string.Empty;

    /// <summary>The final call was not written down in a readable shape.</summary>
    [JsonPropertyName("eligibility_reason_request_malformed")]
    public string EligibilityReasonRequestMalformed { get; init; } = string.Empty;

    /// <summary>The proof that goes with the final call could not be had.</summary>
    [JsonPropertyName("eligibility_reason_receipt_unavailable")]
    public string EligibilityReasonReceiptUnavailable { get; init; } = string.Empty;

    /// <summary>The model that answered does not issue the proof -- permanent.</summary>
    [JsonPropertyName("eligibility_reason_receipt_not_issued")]
    public string EligibilityReasonReceiptNotIssued { get; init; } = string.Empty;

    // The four attestation marks and the fourteen attestation reasons.
    //
    // Separate sentences from the Eligibility* properties above over the SAME
    // thirteen reason labels. Five of the eligibility sentences say the
    // session cannot be sent, which is true for an evidence-admitted
    // contributor and false for an invited one, whose session sends perfectly
    // well and merely arrives without a copy of its call. Do not render one
    // where the other belongs.

    /// <summary>The session carries a checkable copy of its last model call.</summary>
    /// <summary>
    /// The certificate-held list, in both readings.
    /// </summary>
    /// <remarks>
    /// The list is driven by the queue entry's <c>holds_certificate</c>,
    /// true after either witness route. Which sentence a row gets follows
    /// the contributor's invite status, the same status this shell already
    /// reads for the eligibility surface, and never the attestation mark
    /// below. Holds a certificate and was attested are different facts.
    /// </remarks>
    [JsonPropertyName("certificate_row_candidate")]
    public string CertificateRowCandidate { get; init; } = string.Empty;

    [JsonPropertyName("certificate_row_attested")]
    public string CertificateRowAttested { get; init; } = string.Empty;

    [JsonPropertyName("certificate_list_candidate")]
    public string CertificateListCandidate { get; init; } = string.Empty;

    [JsonPropertyName("certificate_list_attested")]
    public string CertificateListAttested { get; init; } = string.Empty;

    [JsonPropertyName("certificate_list_empty")]
    public string CertificateListEmpty { get; init; } = string.Empty;

    [JsonPropertyName("attestation_attested")]
    public string AttestationAttested { get; init; } = string.Empty;

    /// <summary>It carries none, and nothing the contributor changes adds one.</summary>
    [JsonPropertyName("attestation_unattested_permanent")]
    public string AttestationUnattestedPermanent { get; init; } = string.Empty;

    /// <summary>It carries none; a setting decides whether future ones will.</summary>
    [JsonPropertyName("attestation_unattested_configuration")]
    public string AttestationUnattestedConfiguration { get; init; } = string.Empty;

    /// <summary>Not worked out. Never a stand-in for "no copy".</summary>
    [JsonPropertyName("attestation_unknown")]
    public string AttestationUnknown { get; init; } = string.Empty;

    /// <summary>No model call was answered here while the session ran.</summary>
    [JsonPropertyName("attestation_reason_no_call")]
    public string AttestationReasonNoCall { get; init; } = string.Empty;

    /// <summary>The final call was answered without keeping a copy.</summary>
    [JsonPropertyName("attestation_reason_capture_off")]
    public string AttestationReasonCaptureOff { get; init; } = string.Empty;

    /// <summary>The final call did not finish cleanly.</summary>
    [JsonPropertyName("attestation_reason_digest_absent")]
    public string AttestationReasonDigestAbsent { get; init; } = string.Empty;

    /// <summary>Nothing was written down that would let anyone check it.</summary>
    [JsonPropertyName("attestation_reason_upstream_id_absent")]
    public string AttestationReasonUpstreamIdAbsent { get; init; } = string.Empty;

    /// <summary>The kept copy disagrees with the record of it.</summary>
    [JsonPropertyName("attestation_reason_digest_mismatch")]
    public string AttestationReasonDigestMismatch { get; init; } = string.Empty;

    /// <summary>The note saying where the copy lives is not a valid one.</summary>
    [JsonPropertyName("attestation_reason_reference_malformed")]
    public string AttestationReasonReferenceMalformed { get; init; } = string.Empty;

    /// <summary>The kept copy could not be read back from this computer.</summary>
    [JsonPropertyName("attestation_reason_bodies_unreadable")]
    public string AttestationReasonBodiesUnreadable { get; init; } = string.Empty;

    /// <summary>The kept copy is not text the app can carry unchanged.</summary>
    [JsonPropertyName("attestation_reason_body_not_utf8")]
    public string AttestationReasonBodyNotUtf8 { get; init; } = string.Empty;

    /// <summary>The kept copy is larger than the app will carry.</summary>
    [JsonPropertyName("attestation_reason_body_too_large")]
    public string AttestationReasonBodyTooLarge { get; init; } = string.Empty;

    /// <summary>This computer keeps no copy of the model calls it answers.</summary>
    [JsonPropertyName("attestation_reason_evidence_capture_off")]
    public string AttestationReasonEvidenceCaptureOff { get; init; } = string.Empty;

    /// <summary>The final call went out without the mark a copy is checked against.</summary>
    [JsonPropertyName("attestation_reason_marker_absent")]
    public string AttestationReasonMarkerAbsent { get; init; } = string.Empty;

    /// <summary>The final call was not written down in a readable shape.</summary>
    [JsonPropertyName("attestation_reason_request_malformed")]
    public string AttestationReasonRequestMalformed { get; init; } = string.Empty;

    /// <summary>The proof that goes with the final call could not be had.</summary>
    [JsonPropertyName("attestation_reason_receipt_unavailable")]
    public string AttestationReasonReceiptUnavailable { get; init; } = string.Empty;

    /// <summary>The model that answered does not issue a copy-of-call proof -- permanent.</summary>
    [JsonPropertyName("attestation_reason_receipt_not_issued")]
    public string AttestationReasonReceiptNotIssued { get; init; } = string.Empty;

    /// <summary>The heading over the balance row.</summary>
    [JsonPropertyName("balance_title")]
    public string BalanceTitle { get; init; } = string.Empty;

    [JsonPropertyName("funding_title")]
    public string FundingTitle { get; init; } = string.Empty;
    [JsonPropertyName("funding_what")]
    public string FundingWhat { get; init; } = string.Empty;
    [JsonPropertyName("funding_manage")]
    public string FundingManage { get; init; } = string.Empty;
    [JsonPropertyName("funding_refresh")]
    public string FundingRefresh { get; init; } = string.Empty;
    [JsonPropertyName("funding_unavailable")]
    public string FundingUnavailable { get; init; } = string.Empty;

    /// <summary>What the figures on the row are a fact about.</summary>
    [JsonPropertyName("balance_what")]
    public string BalanceWhat { get; init; } = string.Empty;

    /// <summary>No sign-in is kept here, so there is nothing to read.</summary>
    [JsonPropertyName("balance_no_session")]
    public string BalanceNoSession { get; init; } = string.Empty;

    /// <summary>The stored sign-in was not accepted. The one state with a recovery.</summary>
    [JsonPropertyName("balance_session_expired")]
    public string BalanceSessionExpired { get; init; } = string.Empty;

    /// <summary>The account has no organization, and a balance belongs to one.</summary>
    [JsonPropertyName("balance_no_organization")]
    public string BalanceNoOrganization { get; init; } = string.Empty;

    /// <summary>The read failed. A fact about the read, never about the money.</summary>
    [JsonPropertyName("balance_unavailable")]
    public string BalanceUnavailable { get; init; } = string.Empty;

    /// <summary>A state this build has no words for. Never degrades to a known one.</summary>
    [JsonPropertyName("balance_unknown")]
    public string BalanceUnknown { get; init; } = string.Empty;

    /// <summary>A daemon that does not answer this at all.</summary>
    [JsonPropertyName("balance_unreported")]
    public string BalanceUnreported { get; init; } = string.Empty;

    /// <summary>What a null remaining figure says INSTEAD of $0.00.</summary>
    [JsonPropertyName("balance_no_remaining")]
    public string BalanceNoRemaining { get; init; } = string.Empty;

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
            CredentialProviderLabel,
            CredentialProviderGithub,
            CredentialProviderGoogle,
            CredentialProviderNear,
            CredentialWalletNotice,
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
            EligibilityEligible,
            EligibilityIneligiblePermanent,
            EligibilityIneligibleConfiguration,
            EligibilityUnknown,
            EligibilityReasonNoCall,
            EligibilityReasonCaptureOff,
            EligibilityReasonDigestAbsent,
            EligibilityReasonUpstreamIdAbsent,
            EligibilityReasonDigestMismatch,
            EligibilityReasonReferenceMalformed,
            EligibilityReasonBodiesUnreadable,
            EligibilityReasonBodyNotUtf8,
            EligibilityReasonBodyTooLarge,
            EligibilityReasonEvidenceCaptureOff,
            EligibilityReasonMarkerAbsent,
            EligibilityReasonRequestMalformed,
            EligibilityReasonReceiptUnavailable,
            NearAiEnrollTitle,
            NearAiEnrollWhat,
            NearAiEnrollAction,
            NearAiEnrollNeedsLogin,
            NearAiEnrollWorking,
            NearAiEnrollDone,
            NearAiEnrollAlreadyEnrolled,
            NearAiEnrollNoSession,
            NearAiEnrollEndpointRefused,
            NearAiEnrollTokenUnavailable,
            NearAiEnrollStartFailed,
            NearAiEnrollCommonsUnreachable,
            NearAiEnrollCommonsUnsupported,
            NearAiEnrollInvalid,
            NearAiEnrollVerificationFailed,
            NearAiEnrollUnavailable,
            CertificateRowCandidate,
            CertificateRowAttested,
            CertificateListCandidate,
            CertificateListAttested,
            CertificateListEmpty,
            EligibilityReasonReceiptNotIssued,
            AttestationAttested,
            AttestationUnattestedPermanent,
            AttestationUnattestedConfiguration,
            AttestationUnknown,
            AttestationReasonNoCall,
            AttestationReasonCaptureOff,
            AttestationReasonDigestAbsent,
            AttestationReasonUpstreamIdAbsent,
            AttestationReasonDigestMismatch,
            AttestationReasonReferenceMalformed,
            AttestationReasonBodiesUnreadable,
            AttestationReasonBodyNotUtf8,
            AttestationReasonBodyTooLarge,
            AttestationReasonEvidenceCaptureOff,
            AttestationReasonMarkerAbsent,
            AttestationReasonRequestMalformed,
            AttestationReasonReceiptUnavailable,
            AttestationReasonReceiptNotIssued,
            BalanceTitle,
            FundingTitle,
            FundingWhat,
            FundingManage,
            FundingRefresh,
            FundingUnavailable,
            BalanceWhat,
            BalanceNoSession,
            BalanceSessionExpired,
            BalanceNoOrganization,
            BalanceUnavailable,
            BalanceUnknown,
            BalanceUnreported,
            BalanceNoRemaining,
        };
}
