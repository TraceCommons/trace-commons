// Abstract composition of current source behavior at 8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6.
// Standalone projection, not a mechanically composed product or a refinement proof.
module end_to_end {
  type Trace = {
    revision: int, queue: str, consent: int, preview: bool, pinRevision: int,
    pinConsent: int, pinAccount: int, pinDevice: int, pinAvailable: bool,
    sent: bool, requestAccount: int, requestDevice: int, staged: bool,
    objectPresent: bool, orphan: bool, receipt: bool, localReceipt: bool,
    receiptAccount: int, receiptDevice: int, route: int, admission: str, submissionStatus: str,
    phase: str, runState: str, worker: int, expiredWorker: int, privacy: str,
    reviewApproved: bool, reviewAssessed: bool, scoreReviewed: bool, indexCommand: bool,
    indexWriteInFlight: bool, indexPresent: bool, eligible: bool,
    instrumentKind: str, settleGuardRead: bool, settleGuardOperable: bool,
    instrumentInFlight: bool, instrumentDispatched: bool, instrumentResultReady: bool, instrumentAuthorityCurrent: bool,
    instrumentExternalAccepted: bool, instrumentRecorded: bool, instrumentForfeited: bool, instrumentUnreconciled: bool,
    earned: int, pipelineNear: str, nearExternalAccepted: bool,
    withdrawRequest: int, withdrawLocal: bool, withdrawn: bool, cleanupDue: bool,
    cached: str, cachedCredit: int, cacheStale: bool, visible: str, visibleCredit: int
  }
  type AccountRail = {accrued: bool, linked: bool, state: str, externalAccepted: bool}
  val Ids = Set(0, 1)
  pure val emptyTrace: Trace = {
    revision: 0, queue: "pending", consent: 0, preview: false, pinRevision: -1,
    pinConsent: -1, pinAccount: -1, pinDevice: -1, pinAvailable: false,
    sent: false, requestAccount: -1, requestDevice: -1, staged: false,
    objectPresent: false, orphan: false, receipt: false, localReceipt: false,
    receiptAccount: -1, receiptDevice: -1, route: -1, admission: "none", submissionStatus: "none",
    phase: "none", runState: "none", worker: -1, expiredWorker: -1, privacy: "unread",
    reviewApproved: false, reviewAssessed: false, scoreReviewed: false, indexCommand: false,
    indexWriteInFlight: false, indexPresent: false, eligible: false,
    instrumentKind: "none", settleGuardRead: false, settleGuardOperable: false,
    instrumentInFlight: false, instrumentDispatched: false, instrumentResultReady: false, instrumentAuthorityCurrent: false,
    instrumentExternalAccepted: false, instrumentRecorded: false, instrumentForfeited: false, instrumentUnreconciled: false,
    earned: 0, pipelineNear: "none", nearExternalAccepted: false,
    withdrawRequest: -1, withdrawLocal: false, withdrawn: false, cleanupDue: false,
    cached: "unreported", cachedCredit: 0, cacheStale: true,
    visible: "unreported", visibleCredit: 0
  }
  pure val emptyAccountRail: AccountRail = {accrued: false, linked: false, state: "none", externalAccepted: false}
  var s: {
    app: bool, window: bool, daemon: bool, account: int, device: int,
    enrolled: bool, accountSession: bool, deviceAuthority: int -> bool,
    allowance: bool, online: bool, selected: int, activeBundle: int,
    pipelineDispatchAllowed: bool, accountRailEnabled: bool, accountRailAllowed: bool,
    traces: int -> Trace, accountRail: int -> AccountRail,
    pollRequested: bool, historyRead: bool, readTrace: int,
    readAccount: int, readStatus: str, readCredit: int, outcome: str
  }
  val vars = s
  def frame: str = {
    val t = s.traces.get(s.selected)
    if (not(s.app)) "e2e_stopped"
    else if (not(s.window)) "e2e_closed"
    else if (not(s.daemon)) "e2e_offline"
    else if (s.account == -1) "e2e_signed_out"
    else if (t.visible == "withdrawn" or t.visible == "revoked") "e2e_withdrawn"
    else if (t.visibleCredit > 0) "e2e_credit"
    else if (t.localReceipt) "e2e_history"
    else if (t.queue == "sending") "e2e_sending"
    else if (t.queue == "retry") "e2e_retry"
    else if (t.queue == "approved") "e2e_approved"
    else if (t.preview) "e2e_preview"
    else "e2e_pending"
  }
  pure def controls(frame: str): Set[str] = {
    val common = Set("Select another trace", "Change account", "Close window", "Quit", "Refresh history")
    if (frame == "e2e_stopped") Set("Launch app")
    else if (frame == "e2e_closed") Set("Open window", "Quit")
    else if (frame == "e2e_offline") Set("Restart daemon", "Close window", "Quit")
    else if (frame == "e2e_signed_out") Set("Sign in", "Close window", "Quit")
    else if (frame == "e2e_pending") common.union(Set("Preview trace", "Change consent", "Link account payout"))
    else if (frame == "e2e_preview") common.union(Set("Approve preview", "Change consent"))
    else if (frame == "e2e_approved") common.union(Set("Change consent"))
    else if (Set("e2e_history", "e2e_credit").contains(frame)) common.union(Set("Withdraw", "Link account payout"))
    else common
  }
  // Submission verdict is durable state independent of phase/run progress.
  def serverStatus(t: Trace): str =
    if (t.submissionStatus == "revoked") "revoked"
    else if (t.submissionStatus == "received") "submitted"
    else t.submissionStatus
  action init = s' = {
    app: true, window: true, daemon: true, account: -1, device: 0,
    enrolled: false, accountSession: false, deviceAuthority: Ids.mapBy(_ => true),
    allowance: true, online: true, selected: 0, activeBundle: 0,
    pipelineDispatchAllowed: true, accountRailEnabled: false, accountRailAllowed: true,
    traces: Ids.mapBy(_ => emptyTrace), accountRail: Ids.mapBy(_ => emptyAccountRail),
    pollRequested: false, historyRead: false, readTrace: 0,
    readAccount: -1, readStatus: "unreported", readCredit: 0, outcome: "initialized"
  }
  action YouSignIn = {
    nondet a = Ids.oneOf()
    all { controls(frame).contains("Sign in"),
      s' = {...s, account: a, enrolled: true, accountSession: true, outcome: "signed_in"} }
  }
  action YouChangeAccount = {
    nondet a = Set(-1, 0, 1).oneOf()
    all { controls(frame).contains("Change account"), a != s.account,
      s' = {...s, account: a, enrolled: a != -1, accountSession: a != -1, outcome: "account_changed"} }
  }
  action YouSelectAnotherTrace = {
    all { controls(frame).contains("Select another trace"),
      s' = {...s, selected: 1 - s.selected, outcome: "selected_trace_changed"} }
  }
  action YouCloseWindow = {
    all { controls(frame).contains("Close window"),
      s' = {...s, window: false, outcome: "window_closed_daemon_kept"} }
  }
  action YouOpenWindow = {
    all { controls(frame).contains("Open window"),
      s' = {...s, window: true, outcome: "window_opened"} }
  }
  action YouQuitApp = {
    all { controls(frame).contains("Quit"),
      s' = {...s, app: false, window: false, daemon: false, historyRead: false, outcome: "quit_after_stop"} }
  }
  action YouLaunchApp = {
    all { controls(frame).contains("Launch app"),
      s' = {...s, app: true, window: true, daemon: true, outcome: "launched"} }
  }
  action YouRestartDaemon = {
    all { controls(frame).contains("Restart daemon"),
      s' = {...s, daemon: true, outcome: "daemon_restarted"} }
  }
  action TheDaemonStops = {
    all { s.daemon,
      s' = {...s, daemon: false, outcome: "daemon_stopped"} }
  }
  action TheNetworkChanges = {
    all {
      s' = {...s, online: not(s.online), outcome: "network_changed"} }
  }
  action TheDeviceAuthorityChanges = {
    nondet d = Ids.oneOf()
    all {
      s' = {...s, deviceAuthority: s.deviceAuthority.set(d, not(s.deviceAuthority.get(d))), outcome: "device_authority_changed"} }
  }
  action TheAllowanceChanges = {
    all {
      s' = {...s, allowance: not(s.allowance), outcome: "allowance_changed"} }
  }
  action TheAccountSessionExpires = {
    all { s.accountSession,
      s' = {...s, accountSession: false, outcome: "account_session_expired"} }
  }
  action TheDeviceChanges = {
    all {
      s' = {...s, device: 1 - s.device, outcome: "device_changed"} }
  }
  action TheOperatorActivatesAnotherBundle = {
    all {
      s' = {...s, activeBundle: 1 - s.activeBundle, outcome: "new_receipts_use_other_bundle"} }
  }
  action TheOperatorChangesPayoutControls = {
    nondet railEnabled = Set(false, true).oneOf()
    nondet allowed = Set(false, true).oneOf()
    all {
      s' = {...s, pipelineDispatchAllowed: not(s.pipelineDispatchAllowed), accountRailEnabled: railEnabled, accountRailAllowed: allowed, outcome: "payout_controls_changed"} }
  }
  action YouPreviewTrace = {
    val t = s.traces.get(s.selected)
    all { controls(frame).contains("Preview trace"), s.enrolled, not(t.receipt),
      s' = {...s, traces: s.traces.set(s.selected, {...t, preview: true, pinRevision: t.revision, pinConsent: t.consent, pinAccount: s.account, pinDevice: s.device, pinAvailable: true}), outcome: "preview_stored"} }
  }
  action YouApproveTrace = {
    val t = s.traces.get(s.selected)
    all { controls(frame).contains("Approve preview"), s.enrolled, t.preview, t.pinAvailable, t.pinRevision == t.revision, t.pinConsent == t.consent, t.pinAccount == s.account, t.pinDevice == s.device,
      s' = {...s, traces: s.traces.set(s.selected, {...t, queue: "approved"}), outcome: "approved_manual_preview"} }
  }
  action YouChangeConsent = {
    val t = s.traces.get(s.selected)
    all { controls(frame).contains("Change consent"),
      s' = {...s, traces: s.traces.set(s.selected, {...t, consent: 1 - t.consent, queue: if (t.queue == "approved") "pending" else t.queue, preview: false}), outcome: "consent_changed"} }
  }
  action TheSourceRevisionChanges = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.revision == 0,
      s' = {...s, traces: s.traces.set(i, {...t, revision: 1}), outcome: "source_revision_changed"} }
  }
  action ThePinnedArtifactDisappears = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.pinAvailable,
      s' = {...s, traces: s.traces.set(i, {...t, pinAvailable: false}), outcome: "pin_unavailable"} }
  }
  action TheDaemonInvalidatesApproval = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { s.daemon, Set("approved", "retry").contains(t.queue), not(t.pinAvailable) or t.pinRevision != t.revision or t.pinConsent != t.consent or t.pinAccount != s.account or t.pinDevice != s.device,
      s' = {...s, traces: s.traces.set(i, {...t, queue: "pending", preview: false}), outcome: "approval_reoffered"} }
  }
  action TheDaemonStartsSend = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { s.daemon, s.enrolled, s.account != -1, s.allowance, s.deviceAuthority.get(s.device), Set("approved", "retry").contains(t.queue), t.pinAvailable, t.pinRevision == t.revision, t.pinConsent == t.consent, t.pinAccount == s.account, t.pinDevice == s.device, not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, queue: "sending", sent: true, requestAccount: t.pinAccount, requestDevice: t.pinDevice}), outcome: "send_started_same_key_and_pinned_identity"} }
  }
  action TheTransportFails = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.sent, t.queue == "sending",
      s' = {...s, traces: s.traces.set(i, {...t, queue: "retry"}), outcome: "transport_failed_receipt_may_exist"} }
  }
  action TheServerStagesCiphertext = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.sent, s.online, not(t.receipt), not(t.staged), s.deviceAuthority.get(t.requestDevice), s.allowance, not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, staged: true, objectPresent: true, route: s.activeBundle}), outcome: "ciphertext_staged_bundle_pinned"} }
  }
  action TheReceiptCommitFails = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.staged, not(t.receipt),
      s' = {...s, traces: s.traces.set(i, {...t, staged: false, orphan: true, queue: "retry"}), outcome: "object_exists_without_receipt"} }
  }
  action TheServerCommitsReceiptAndAdmission = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet decision = Set("admitted", "quarantined", "rejected").oneOf()
    all { t.staged, not(t.receipt), not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, receipt: true, staged: false, orphan: false, receiptAccount: t.requestAccount, receiptDevice: t.requestDevice, admission: decision, submissionStatus: if (decision == "admitted") "received" else if (decision == "quarantined") "quarantined" else "rejected", phase: if (decision == "rejected") "done" else "review", runState: if (decision == "rejected") "complete" else "pending"}), outcome: "durable_receipt_and_synchronous_admission"} }
  }
  action TheServerReplaysReceipt = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.sent, t.receipt, not(t.localReceipt), Set("pending", "sending", "retry").contains(t.queue),
      s' = {...s, traces: s.traces.set(i, {...t, queue: "sending"}), outcome: "same_receipt_replayed_regardless_active_bundle"} }
  }
  action TheUploadResponseArrives = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.receipt, t.sent, s.daemon, t.queue == "sending",
      s' = {...s, traces: s.traces.set(i, {...t, localReceipt: true, queue: "uploaded", cached: if (t.cached == "unreported") "submitted" else t.cached}), outcome: "processing_receipt_observed_not_verdict"} }
  }
  action TheServerCleansOrphan = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.orphan, not(t.receipt), not(t.staged),
      s' = {...s, traces: s.traces.set(i, {...t, orphan: false, objectPresent: false}), outcome: "uncommitted_object_cleaned"} }
  }
  action TheWorkerClaimsPhase = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet w = Ids.oneOf()
    all { t.receipt, Set("pending", "retry").contains(t.runState), t.phase != "done",
      s' = {...s, traces: s.traces.set(i, {...t, runState: "leased", worker: w, settleGuardRead: false}), outcome: "phase_leased"} }
  }
  action TheWorkerLeaseExpires = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.runState == "leased",
      s' = {...s, traces: s.traces.set(i, {...t, runState: "retry", expiredWorker: t.worker, worker: -1, settleGuardRead: false, instrumentAuthorityCurrent: false}), outcome: "lease_expired"} }
  }
  action TheExpiredWorkerReturns = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.expiredWorker != -1,
      s' = {...s, traces: s.traces.set(i, {...t, expiredWorker: -1}), outcome: "stale_worker_result_discarded"} }
  }
  action TheWorkerHasOperationalFailure = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.runState == "leased",
      s' = {...s, traces: s.traces.set(i, {...t, runState: "retry", worker: -1, settleGuardRead: false, instrumentAuthorityCurrent: false}), outcome: "operational_retry"} }
  }
  action TheWorkerExhaustsItsRetryBudget = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.runState == "retry",
      s' = {...s, traces: s.traces.set(i, {...t, runState: "failed", instrumentAuthorityCurrent: false, submissionStatus: if (t.submissionStatus == "received" and not(t.withdrawn)) "rejected" else t.submissionStatus, eligible: false, cleanupDue: t.indexPresent}), outcome: "retry_budget_exhausted"} }
  }
  action TheInoperableReviewOrScoreEnds = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.withdrawn, Set("review", "score").contains(t.phase), not(Set("complete", "failed").contains(t.runState)),
      s' = {...s, traces: s.traces.set(i, {...t, phase: "done", runState: "failed", worker: -1, eligible: false}), outcome: "inoperable_non_settle_work_ends_without_changing_revoked_verdict"} }
  }
  action TheReviewStartsPrivacyPass = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "review", t.runState == "leased", t.privacy == "unread",
      s' = {...s, traces: s.traces.set(i, {...t, privacy: "running"}), outcome: "review_start_privacy_pass"} }
  }
  action ThePrivacyPassFinishes = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet result = Set("low", "quarantined").oneOf()
    all { t.phase == "review", t.runState == "leased", t.privacy == "running", not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, privacy: result, submissionStatus: if (result == "quarantined" and t.submissionStatus == "received") "quarantined" else t.submissionStatus, runState: if (result == "low" and t.admission == "admitted") "leased" else "awaiting_review", worker: if (result == "low" and t.admission == "admitted") t.worker else -1}), outcome: "privacy_pass_completed_or_held"} }
  }
  action TheHumanAssessesReview = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet approve = Set(false, true).oneOf()
    all { t.phase == "review", t.runState == "awaiting_review", not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, privacy: if (approve) "low" else "rejected", submissionStatus: if (approve) t.submissionStatus else "rejected", reviewAssessed: approve, runState: if (approve) "pending" else "complete", phase: if (approve) "review" else "done"}), outcome: "human_review_assessment"} }
  }
  action TheReviewCommitsApproval = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "review", t.runState == "leased", t.privacy == "low", t.admission == "admitted" or t.reviewAssessed, not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, reviewApproved: true, submissionStatus: "accepted", phase: "score", runState: "pending", worker: -1}), outcome: "review_approved"} }
  }
  action TheScoreRequestsHumanReview = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "score", t.runState == "leased", not(t.scoreReviewed), not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, runState: "awaiting_review", worker: -1}), outcome: "score_hold_distinct_from_privacy"} }
  }
  action TheHumanAssessesScore = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "score", t.runState == "awaiting_review", not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, scoreReviewed: true, runState: "pending"}), outcome: "score_assessment_recorded"} }
  }
  action TheScoreCommitsDecision = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet include = Set(false, true).oneOf()
    all { t.phase == "score", t.runState == "leased", not(t.withdrawn), t.reviewApproved,
      s' = {...s, traces: s.traces.set(i, {...t, indexCommand: include, eligible: false, phase: "settle", runState: "pending", worker: -1}), outcome: "score_and_sealed_index_command_committed"} }
  }
  action TheIndexWriteStarts = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.indexCommand, not(t.indexWriteInFlight), not(t.indexPresent), not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, indexWriteInFlight: true}), outcome: "external_index_write_dispatched"} }
  }
  action TheIndexWriteFinishes = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.indexWriteInFlight,
      s' = {...s, traces: s.traces.set(i, {...t, indexWriteInFlight: false, indexPresent: true, cleanupDue: t.withdrawn or t.runState == "failed"}), outcome: "index_write_may_finish_after_invalidation"} }
  }
  action TheSettlementSelectsInstrument = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet kind = Set("internal_credit", "external_instrument").oneOf()
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "none", s.pipelineDispatchAllowed,
      s' = {...s, traces: s.traces.set(i, {...t, instrumentKind: kind}), outcome: "settlement_selection_persisted_even_if_now_inoperable"} }
  }
  // Source Step 6 reads once before its loop; a later withdrawal does not refresh this snapshot.
  action TheWorkerReadsSettlementGuard = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind != "none", not(t.settleGuardRead),
      not(t.indexCommand) or t.indexPresent or t.withdrawn,
      s' = {...s, traces: s.traces.set(i, {...t, settleGuardRead: true, settleGuardOperable: not(t.withdrawn)}), outcome: "settle_pass_operability_snapshot_read"} }
  }
  action TheInstrumentAdapterStarts = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "external_instrument",
      t.settleGuardRead, t.settleGuardOperable, s.pipelineDispatchAllowed,
      not(t.instrumentInFlight), not(t.instrumentResultReady), not(t.instrumentRecorded), not(t.instrumentForfeited),
      s' = {...s, traces: s.traces.set(i, {...t, instrumentInFlight: true, instrumentDispatched: true, instrumentAuthorityCurrent: true}), outcome: "external_adapter_dispatch_uses_earlier_operability_snapshot"} }
  }
  action TheInstrumentMayTakeEffectWithoutAReceipt = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.instrumentInFlight,
      s' = {...s, traces: s.traces.set(i, {...t, instrumentExternalAccepted: true}), outcome: "external_effect_can_exist_before_matching_reply"} }
  }
  action TheInstrumentAdapterAccepts = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.instrumentInFlight,
      s' = {...s, traces: s.traces.set(i, {...t, instrumentInFlight: false, instrumentExternalAccepted: true, instrumentResultReady: true}), outcome: "external_instrument_matching_reply_may_arrive_after_withdrawal"} }
  }
  action TheInstrumentAdapterReturnsUnavailable = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.instrumentInFlight,
      s' = {...s, traces: s.traces.set(i, {...t, instrumentInFlight: false, instrumentResultReady: false,
        runState: if (t.instrumentAuthorityCurrent) "retry" else t.runState,
        worker: if (t.instrumentAuthorityCurrent) -1 else t.worker,
        settleGuardRead: if (t.instrumentAuthorityCurrent) false else t.settleGuardRead, instrumentAuthorityCurrent: false}), outcome: "unavailable_reply_preserves_possible_external_effect"} }
  }
  // Non-Trace-Credit receipt recording checks the live lease, not fresh submission operability.
  action TheSettlementRecordsExternalReceipt = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "external_instrument",
      t.instrumentResultReady, t.instrumentAuthorityCurrent, not(t.instrumentRecorded), not(t.instrumentForfeited),
      s' = {...s, traces: s.traces.set(i, {...t, instrumentRecorded: true, instrumentResultReady: false}), outcome: "external_receipt_recorded_independently_of_withdrawal_or_credit"} }
  }
  action TheInstrumentReceiptRecordFails = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.instrumentResultReady, t.instrumentAuthorityCurrent, not(t.instrumentRecorded),
      s' = {...s, traces: s.traces.set(i, {...t, runState: "retry", worker: -1, settleGuardRead: false, instrumentResultReady: false, instrumentAuthorityCurrent: false}), outcome: "external_effect_survives_local_receipt_failure"} }
  }
  // Expiry invalidates the captured result authority; replacement claims never renew an old call.
  action TheStaleInstrumentReplyIsDiscarded = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.instrumentResultReady, not(t.instrumentAuthorityCurrent),
      s' = {...s, traces: s.traces.set(i, {...t, instrumentResultReady: false}), outcome: "old_adapter_reply_discarded_external_effect_uncertainty_retained"} }
  }
  // The internal ledger transaction rechecks operability; this is separate from external recording.
  action TheSettlementCommitsEarnedCredit = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet award = Set(0, 1).oneOf()
    nondet payoutSelected = Set(false, true).oneOf()
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "internal_credit",
      t.settleGuardRead, t.settleGuardOperable, s.pipelineDispatchAllowed, not(t.withdrawn), not(t.instrumentRecorded), not(t.instrumentForfeited),
      s' = {...s, traces: s.traces.set(i, {...t, earned: award, instrumentRecorded: true,
        pipelineNear: if (award == 1 and payoutSelected) "pending" else "none"}), outcome: "internal_credit_transaction_commits_without_completing_entire_run"} }
  }
  action TheInternalCreditRecheckFindsWithdrawal = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "internal_credit",
      t.settleGuardRead, t.withdrawn, not(t.instrumentRecorded), not(t.instrumentForfeited),
      s' = {...s, traces: s.traces.set(i, {...t, settleGuardOperable: false, instrumentForfeited: true}), outcome: "withdrawal_forfeits_uncommitted_internal_credit"} }
  }
  action TheSettlementForfeitsOpenInstrument = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.settleGuardRead, not(t.settleGuardOperable),
      t.instrumentKind != "none", not(t.instrumentInFlight), not(t.instrumentRecorded), not(t.instrumentForfeited),
      s' = {...s, traces: s.traces.set(i, {...t, instrumentForfeited: true,
        instrumentUnreconciled: t.instrumentKind == "external_instrument" and t.instrumentDispatched}), outcome: "open_leg_forfeited_possible_external_effect_retained_for_reconciliation"} }
  }
  action TheSettlementCommitsFromTerminalInstrument = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", s.pipelineDispatchAllowed,
      t.instrumentRecorded or t.instrumentForfeited, not(t.instrumentInFlight),
      not(t.indexCommand) or t.indexPresent or t.withdrawn,
      s' = {...s, traces: s.traces.set(i, {...t, eligible: t.indexCommand and t.indexPresent and not(t.withdrawn),
        phase: "done", runState: "complete", worker: -1}), outcome: "settlement_finishes_from_actual_recorded_or_forfeited_effects"} }
  }
  action TheSettlementNeedsOperatorResolution = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.phase == "settle", t.runState == "leased", t.instrumentKind == "external_instrument", t.instrumentRecorded,
      s' = {...s, traces: s.traces.set(i, {...t, runState: "awaiting_operator", worker: -1}), outcome: "paid_settlement_commit_refusal_keeps_recorded_external_effect"} }
  }
  action TheOperatorResolvesSettlement = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.runState == "awaiting_operator",
      s' = {...s, traces: s.traces.set(i, {...t, runState: "retry", settleGuardRead: false}), outcome: "operator_resolution_abstract"} }
  }
  action ThePipelineNearAdapterAccepts = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.pipelineNear == "pending", s.pipelineDispatchAllowed, s.online,
      s' = {...s, traces: s.traces.set(i, {...t, nearExternalAccepted: true}), outcome: "pipeline_near_external_effect_same_key"} }
  }
  action ThePipelineNearLocalRecordFails = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.pipelineNear == "pending", t.nearExternalAccepted,
      s' = {...s, traces: s.traces.set(i, {...t, pipelineNear: "pending"}), outcome: "pipeline_near_external_success_local_failure"} }
  }
  action ThePipelineNearSubmissionIsRecorded = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.pipelineNear == "pending", t.nearExternalAccepted,
      s' = {...s, traces: s.traces.set(i, {...t, pipelineNear: "submitted"}), outcome: "pipeline_near_submitted_locally"} }
  }
  action ThePipelineNearConfirmationArrives = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    nondet result = Set("submitted", "confirmed", "failed").oneOf()
    all { t.pipelineNear == "submitted", s.online,
      s' = {...s, traces: s.traces.set(i, {...t, pipelineNear: result}), outcome: "pipeline_near_confirmation_even_when_dispatch_suspended"} }
  }
  action TheAccountRailAccrues = {
    nondet a = Ids.oneOf()
    val r = s.accountRail.get(a)
    all {
      s' = {...s, accountRail: s.accountRail.set(a, {...r, accrued: true, state: if (r.state == "none") "pending" else r.state}), outcome: "independent_account_or_referral_accrual"} }
  }
  action YouLinkAccountPayout = {
    val r = if (s.account == -1) emptyAccountRail else s.accountRail.get(s.account)
    all { controls(frame).contains("Link account payout"), s.account != -1, s.accountSession,
      s' = {...s, accountRail: s.accountRail.set(s.account, {...r, linked: true}), outcome: "account_payout_link_confirmed_abstract"} }
  }
  action TheAccountRailAdapterAccepts = {
    nondet a = Ids.oneOf()
    val r = s.accountRail.get(a)
    all { s.accountRailEnabled, s.accountRailAllowed, s.online, r.linked, r.accrued, r.state == "pending",
      s' = {...s, accountRail: s.accountRail.set(a, {...r, externalAccepted: true}), outcome: "account_near_ai_external_effect_same_key"} }
  }
  action TheAccountRailLocalRecordFails = {
    nondet a = Ids.oneOf()
    val r = s.accountRail.get(a)
    all { r.state == "pending", r.externalAccepted,
      s' = {...s, outcome: "account_near_ai_external_success_local_failure"} }
  }
  action TheAccountRailSubmissionIsRecorded = {
    nondet a = Ids.oneOf()
    val r = s.accountRail.get(a)
    all { r.state == "pending", r.externalAccepted,
      s' = {...s, accountRail: s.accountRail.set(a, {...r, state: "submitted"}), outcome: "account_near_ai_submission_recorded"} }
  }
  action TheAccountRailConfirmationArrives = {
    nondet a = Ids.oneOf()
    val r = s.accountRail.get(a)
    nondet result = Set("submitted", "confirmed", "failed").oneOf()
    all { r.state == "submitted", s.online,
      s' = {...s, accountRail: s.accountRail.set(a, {...r, state: result}), outcome: "account_rail_confirmation_abstract"} }
  }
  action YouWithdrawTrace = {
    val t = s.traces.get(s.selected)
    all { controls(frame).contains("Withdraw"), s.accountSession, t.localReceipt, t.receiptAccount == s.account, not(t.withdrawn), t.withdrawRequest == -1,
      s' = {...s, traces: s.traces.set(s.selected, {...t, withdrawRequest: s.account}), outcome: "withdrawal_requested_by_account"} }
  }
  action TheServerAcceptsWithdrawal = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.withdrawRequest == t.receiptAccount, t.withdrawRequest != -1, not(t.withdrawn), s.online,
      s' = {...s, traces: s.traces.set(i, {...t, withdrawn: true, submissionStatus: "revoked", eligible: false, cleanupDue: true, withdrawRequest: -1, withdrawLocal: true}), outcome: "logical_withdrawal_committed"} }
  }
  action TheWithdrawalReplyArrives = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.withdrawn, t.withdrawLocal, t.localReceipt, s.daemon,
      s' = {...s, traces: s.traces.set(i, {...t, cached: "withdrawn", cacheStale: false, withdrawLocal: false}), outcome: "local_history_marks_confirmed_withdrawal"} }
  }
  action TheAccountWithdrawsThroughTheWeb = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.receipt, not(t.withdrawn),
      s' = {...s, traces: s.traces.set(i, {...t, withdrawn: true, submissionStatus: "revoked", eligible: false, cleanupDue: true}), outcome: "external_account_withdrawal"} }
  }
  action ThePhysicalCleanupRuns = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.cleanupDue, not(t.indexWriteInFlight),
      s' = {...s, traces: s.traces.set(i, {...t, indexPresent: false, cleanupDue: false}), outcome: "index_cleanup_after_logical_exclusion"} }
  }
  action TheRetentionPolicyAllowsObjectPurge = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { t.withdrawn, t.objectPresent,
      s' = {...s, traces: s.traces.set(i, {...t, objectPresent: false}), outcome: "physical_object_purge_later_abstract"} }
  }
  action YouRequestHistoryRefresh = {
    val t = s.traces.get(s.selected)
    all { controls(frame).contains("Refresh history"), not(s.historyRead),
      s' = {...s, pollRequested: true, historyRead: true, readTrace: s.selected, readAccount: s.account, readStatus: t.cached, readCredit: t.cachedCredit, outcome: "poll_requested_cache_read_in_flight"} }
  }
  action TheDaemonPollsServerHistory = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { s.daemon, s.online, t.localReceipt, t.receiptAccount == s.account,
      s' = {...s, traces: s.traces.set(i, {...t, cached: serverStatus(t), cachedCredit: t.earned, cacheStale: false}), outcome: "server_status_joined_into_daemon_cache"} }
  }
  action TheHistoryPollFails = {
    nondet i = Ids.oneOf()
    val t = s.traces.get(i)
    all { s.daemon, not(s.online), t.localReceipt,
      s' = {...s, traces: s.traces.set(i, {...t, cacheStale: true}), outcome: "offline_cached_history_retained"} }
  }
  action TheShellReadsUpdatedCache = {
    val t = s.traces.get(s.selected)
    all { s.app, s.daemon, not(s.historyRead),
      s' = {...s, historyRead: true, readTrace: s.selected, readAccount: s.account, readStatus: t.cached, readCredit: t.cachedCredit, outcome: "history_ipc_snapshot_started"} }
  }
  action TheHistoryReplyArrives = {
    val t = s.traces.get(s.readTrace)
    all { s.app, s.historyRead,
      s' = {...s, historyRead: false, pollRequested: false, traces: s.traces.set(s.readTrace, {...t, visible: s.readStatus, visibleCredit: s.readCredit}), outcome: if (s.readAccount == s.account) "history_published" else "late_history_published_after_account_change"} }
  }
  action step = any {
    YouSignIn,
    YouChangeAccount,
    YouSelectAnotherTrace,
    YouCloseWindow,
    YouOpenWindow,
    YouQuitApp,
    YouLaunchApp,
    YouRestartDaemon,
    TheDaemonStops,
    TheNetworkChanges,
    TheDeviceAuthorityChanges,
    TheAllowanceChanges,
    TheAccountSessionExpires,
    TheDeviceChanges,
    TheOperatorActivatesAnotherBundle,
    TheOperatorChangesPayoutControls,
    YouPreviewTrace,
    YouApproveTrace,
    YouChangeConsent,
    TheSourceRevisionChanges,
    ThePinnedArtifactDisappears,
    TheDaemonInvalidatesApproval,
    TheDaemonStartsSend,
    TheTransportFails,
    TheServerStagesCiphertext,
    TheReceiptCommitFails,
    TheServerCommitsReceiptAndAdmission,
    TheServerReplaysReceipt,
    TheUploadResponseArrives,
    TheServerCleansOrphan,
    TheWorkerClaimsPhase,
    TheWorkerLeaseExpires,
    TheExpiredWorkerReturns,
    TheWorkerHasOperationalFailure,
    TheWorkerExhaustsItsRetryBudget,
    TheInoperableReviewOrScoreEnds,
    TheReviewStartsPrivacyPass,
    ThePrivacyPassFinishes,
    TheHumanAssessesReview,
    TheReviewCommitsApproval,
    TheScoreRequestsHumanReview,
    TheHumanAssessesScore,
    TheScoreCommitsDecision,
    TheIndexWriteStarts,
    TheIndexWriteFinishes,
    TheSettlementSelectsInstrument,
    TheWorkerReadsSettlementGuard,
    TheInstrumentAdapterStarts,
    TheInstrumentMayTakeEffectWithoutAReceipt,
    TheInstrumentAdapterAccepts,
    TheInstrumentAdapterReturnsUnavailable,
    TheSettlementRecordsExternalReceipt,
    TheInstrumentReceiptRecordFails,
    TheStaleInstrumentReplyIsDiscarded,
    TheSettlementCommitsEarnedCredit,
    TheInternalCreditRecheckFindsWithdrawal,
    TheSettlementForfeitsOpenInstrument,
    TheSettlementCommitsFromTerminalInstrument,
    TheSettlementNeedsOperatorResolution,
    TheOperatorResolvesSettlement,
    ThePipelineNearAdapterAccepts,
    ThePipelineNearLocalRecordFails,
    ThePipelineNearSubmissionIsRecorded,
    ThePipelineNearConfirmationArrives,
    TheAccountRailAccrues,
    YouLinkAccountPayout,
    TheAccountRailAdapterAccepts,
    TheAccountRailLocalRecordFails,
    TheAccountRailSubmissionIsRecorded,
    TheAccountRailConfirmationArrives,
    YouWithdrawTrace,
    TheServerAcceptsWithdrawal,
    TheWithdrawalReplyArrives,
    TheAccountWithdrawsThroughTheWeb,
    ThePhysicalCleanupRuns,
    TheRetentionPolicyAllowsObjectPurge,
    YouRequestHistoryRefresh,
    TheDaemonPollsServerHistory,
    TheHistoryPollFails,
    TheShellReadsUpdatedCache,
    TheHistoryReplyArrives
  }
}
