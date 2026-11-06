// Current-behavior abstraction; no product promises selected or encoded.
module server_settlement {
  pure val Runs = Set(0, 1)
  pure val LeaseTokens = Set(0, 1, 2)
  pure val Legs = Set(0, 1, 2, 3)
  pure def runOf(leg: int): int = leg / 2
  pure def isCredit(leg: int): bool = leg % 2 == 0
  pure val Panels = Set("settlement-ready", "settlement-progress", "settlement-uncertain", "pipeline-near", "account-payout", "settlement-held")
  pure def controls(frame: str): Set[str] = if (Panels.contains(frame)) Set("YouSuspendSettlePolicy", "YouResumeSettlePolicy", "YouHaltAccountRail", "YouResumeAccountRail", "YouRetryFailedNearSubmit", "YouClearAccountHold") else Set()
  var s: {
    panel: str, selected: Set[int], operable: int -> bool, runnable: int -> bool,
    indexDone: int -> bool, settleGuard: int -> bool, guardRead: Set[int], attempted: Set[int], leaseToken: int -> int, liveLease: int -> bool, leg: int -> str,
    dispatched: Set[int], resultReady: Set[int], creditBatched: int -> bool, creditHold: Set[int], externalEffect: Set[int], ledger: Set[int], audited: Set[int],
    batch: Set[int], near: int -> str, nearEffect: Set[int], nearContract: int -> int,
    configuredContract: int, nearMode: str, submittedMode: int -> str, nearFlight: int,
    flight: int, flightToken: int, accountLine: int -> str, accountEffect: Set[int], accountConfirmed: Set[int],
    link: int -> str, owed: int -> bool, accountHold: Set[int], streamHold: Set[int], referralHold: bool,
    accountHalted: bool, accountMode: str, referralMode: str, budget: int,
    accountFlight: int, error: str
  }
  val vars = { s: s }
  val flightLeaseCurrent = if (s.flight == -1) false else
    s.liveLease.get(runOf(s.flight)) and s.flightToken == s.leaseToken.get(runOf(s.flight))
  action init = s' = {
    panel: "settlement-ready", selected: Set(), operable: Runs.mapBy(_ => true),
    runnable: Runs.mapBy(_ => true), indexDone: Runs.mapBy(_ => false), settleGuard: Runs.mapBy(_ => false), guardRead: Set(), attempted: Set(),
    leaseToken: Runs.mapBy(_ => 0), liveLease: Runs.mapBy(_ => true), leg: Legs.mapBy(_ => "unseeded"),
    dispatched: Set(), resultReady: Set(), creditBatched: Runs.mapBy(_ => true), creditHold: Set(), externalEffect: Set(), ledger: Set(), audited: Set(), batch: Set(),
    near: Runs.mapBy(_ => "none"), nearEffect: Set(), nearContract: Runs.mapBy(_ => 0),
    configuredContract: 0, nearMode: "http", submittedMode: Runs.mapBy(_ => "none"), nearFlight: -1,
    flight: -1, flightToken: -1, accountLine: Legs.mapBy(_ => "none"), accountEffect: Set(), accountConfirmed: Set(),
    link: Runs.mapBy(_ => "waiting"), owed: Legs.mapBy(_ => false), accountHold: Set(), streamHold: Set(),
    referralHold: false, accountHalted: false, accountMode: "live", referralMode: "live", budget: 2,
    accountFlight: -1, error: "none"
  }
  // Inbound from the detailed Score model: two awards, immutable operation identities.
  action TheScoreCommitsTwoInstrumentAwards = {
    nondet r = Runs.oneOf()
    all { s.leg.get(r * 2) == "unseeded",
      s' = { ...s, leg: s.leg.set(r * 2, "pending").set(r * 2 + 1, "pending"), panel: "settlement-progress" } }
  }
  action TheSettleSelectionIsPersisted = {
    nondet r = Runs.oneOf()
    all { s.leg.get(r * 2) != "unseeded", not(s.selected.contains(r)), s.runnable.get(r), s.liveLease.get(r),
      s' = { ...s, selected: s.selected.union(Set(r)), guardRead: s.guardRead.exclude(Set(r)), panel: "settlement-progress" } }
  }
  // Inbound from index dispatch, including exclude/cancel: no index work remains.
  action TheIndexDispatchIsResolved = {
    nondet r = Runs.oneOf()
    all { s.selected.contains(r), s' = { ...s, indexDone: s.indexDone.set(r, true) } }
  }
  action TheSubmissionBecomesInoperable = {
    nondet r = Runs.oneOf()
    s' = { ...s, operable: s.operable.set(r, false), panel: "settlement-held" }
  }
  action TheLeaseExpires = {
    nondet r = Runs.oneOf()
    s' = { ...s, liveLease: s.liveLease.set(r, false), guardRead: s.guardRead.exclude(Set(r)), panel: "settlement-uncertain" }
  }
  action AnotherWorkerReclaimsTheLease = {
    nondet r = Runs.oneOf()
    nondet token = LeaseTokens.oneOf()
    all { not(s.liveLease.get(r)), token != s.leaseToken.get(r),
      (s.flight == -1 or runOf(s.flight) != r or token != s.flightToken),
      s' = { ...s, leaseToken: s.leaseToken.set(r, token), liveLease: s.liveLease.set(r, true), guardRead: s.guardRead.exclude(Set(r)) } }
  }
  action YouSuspendSettlePolicy = {
    nondet r = Runs.oneOf()
    all { controls(s.panel).contains("YouSuspendSettlePolicy"), s' = { ...s, runnable: s.runnable.set(r, false), panel: "settlement-held" } }
  }
  action YouResumeSettlePolicy = {
    nondet r = Runs.oneOf()
    all { controls(s.panel).contains("YouResumeSettlePolicy"), s' = { ...s, runnable: s.runnable.set(r, true), panel: "settlement-progress" } }
  }
  // The loop reads once. External legs can still dispatch after a withdrawal races this snapshot.
  action TheWorkerReadsTheSubmissionGuard = {
    nondet r = Runs.oneOf()
    all { s.selected.contains(r), s.indexDone.get(r), s.liveLease.get(r), not(s.guardRead.contains(r)),
      s.flight != r * 2, s.flight != r * 2 + 1,
      s' = { ...s, guardRead: s.guardRead.union(Set(r)), attempted: s.attempted.exclude(Set(r * 2, r * 2 + 1)), settleGuard: s.settleGuard.set(r, s.operable.get(r)) } }
  }
  // Ending an attempt consumes its snapshot; a retry must execute the next pass's read.
  action TheWorkerEndsTheSettleAttempt = {
    nondet r = Runs.oneOf()
    all { s.guardRead.contains(r), s.flight != r * 2, s.flight != r * 2 + 1,
      s' = { ...s, guardRead: s.guardRead.exclude(Set(r)) } }
  }
  action TheWorkerDispatchesAnInstrumentOperation = {
    nondet l = Legs.oneOf()
    val r = runOf(l)
    all { s.flight == -1, s.selected.contains(r), s.indexDone.get(r), s.liveLease.get(r), s.runnable.get(r), s.guardRead.contains(r), s.settleGuard.get(r), not(s.attempted.contains(l)),
      Set("pending", "dispatched", "retry", "unreconciled").contains(s.leg.get(l)),
      s' = { ...s, dispatched: s.dispatched.union(Set(l)), attempted: s.attempted.union(Set(l)), leg: s.leg.set(l, "dispatched"), flight: l, flightToken: s.leaseToken.get(r), resultReady: s.resultReady.exclude(Set(l)), panel: "settlement-progress" } }
  }
  action TheAdapterReturnsAReceipt = all { s.flight != -1,
    s' = { ...s, resultReady: s.resultReady.union(Set(s.flight)),
      externalEffect: if (isCredit(s.flight)) s.externalEffect else s.externalEffect.union(Set(s.flight)), panel: "settlement-uncertain" } }
  action TheExternalInstrumentMayTakeEffectWithoutAReceipt = all { s.flight != -1, not(isCredit(s.flight)),
    s' = { ...s, externalEffect: s.externalEffect.union(Set(s.flight)), panel: "settlement-uncertain" } }
  action TheAdapterReturnsUnavailable = all { s.flight != -1, flightLeaseCurrent, not(s.resultReady.contains(s.flight)),
    s' = { ...s, leg: s.leg.set(s.flight, "retry"), flight: -1, error: "adapter_failed", panel: "settlement-uncertain" } }
  action TheWorkerCrashesAfterExternalSuccess = all { s.flight != -1, flightLeaseCurrent, s.resultReady.contains(s.flight),
    s' = { ...s, leg: s.leg.set(s.flight, "retry"), flight: -1, error: "local_commit_missing", panel: "settlement-uncertain" } }
  // Receiving an old reply never transfers authority from the replacement lease.
  // Leave durable progress and external uncertainty intact for the current worker's retry.
  action TheStaleAdapterResultIsDiscarded = all { s.flight != -1, not(flightLeaseCurrent),
    s' = { ...s, flight: -1, flightToken: -1, error: "stale_lease", panel: "settlement-uncertain" } }
  action TheExternalReceiptIsRecorded = all { s.flight != -1, flightLeaseCurrent, not(isCredit(s.flight)), s.resultReady.contains(s.flight), s.liveLease.get(runOf(s.flight)),
    s' = { ...s, leg: s.leg.set(s.flight, "complete"), flight: -1, error: "none", panel: "settlement-progress" } }
  // Internal credit rechecks operability under its own transaction before ledger+batch commit.
  action TheInternalCreditTransactionCommits = all { s.flight != -1, flightLeaseCurrent, isCredit(s.flight), s.resultReady.contains(s.flight),
    s.operable.get(runOf(s.flight)), s.liveLease.get(runOf(s.flight)), s.runnable.get(runOf(s.flight)),
    (not(s.creditBatched.get(runOf(s.flight))) or not(s.creditHold.contains(runOf(s.flight)))),
    s' = { ...s, leg: s.leg.set(s.flight, "complete"), ledger: s.ledger.union(Set(s.flight)),
      batch: if (s.creditBatched.get(runOf(s.flight))) s.batch.union(Set(runOf(s.flight))) else s.batch,
      near: if (s.creditBatched.get(runOf(s.flight))) s.near.set(runOf(s.flight), "pending") else s.near, flight: -1, panel: "settlement-progress" } }
  action TheDuplicateAwardIsWithheld = {
    nondet r = Runs.oneOf()
    all { s.selected.contains(r), s.guardRead.contains(r), s.liveLease.get(r), s.settleGuard.get(r), s.leg.get(r * 2) == "pending", not(s.dispatched.contains(r * 2)),
      s' = { ...s, leg: s.leg.set(r * 2, "complete_withheld"), panel: "settlement-progress" } }
  }
  action TheWorkerForfeitsAnUnfinishedLeg = {
    nondet l = Legs.oneOf()
    all { s.guardRead.contains(runOf(l)), not(s.settleGuard.get(runOf(l))), s.liveLease.get(runOf(l)),
      not(Set("unseeded", "complete", "complete_withheld", "forfeited").contains(s.leg.get(l))),
      s.flight != l,
      s' = { ...s, leg: s.leg.set(l, "forfeited"), error: if (s.dispatched.contains(l)) "submission_inoperable_after_dispatch" else "submission_inoperable", panel: "settlement-held" } }
  }
  action TheReturnedCreditFindsWithdrawal = all { s.flight != -1, flightLeaseCurrent, isCredit(s.flight), not(s.operable.get(runOf(s.flight))),
    s' = { ...s, leg: s.leg.set(s.flight, "forfeited"), settleGuard: s.settleGuard.set(runOf(s.flight), false), flight: -1, error: "submission_inoperable", panel: "settlement-held" } }
  action TheReturnedReceiptDoesNotAnswerTheOperation = all { s.flight != -1, flightLeaseCurrent,
    s' = { ...s, leg: s.leg.set(s.flight, "mismatch"), flight: -1, error: "settlement_result_mismatch", panel: "settlement-uncertain" } }
  action TheFailurePathCannotReconcileAnExternalLeg = {
    nondet l = Set(1, 3).oneOf()
    all { s.leg.get(l) == "retry", s.dispatched.contains(l),
      s' = { ...s, leg: s.leg.set(l, "unreconciled"), error: "settlement_unreconciled", panel: "settlement-uncertain" } }
  }
  action TheBoundCreditEventUsesAnUnbatchedCompatibilityRule = {
    nondet r = Runs.oneOf()
    all { s.leg.get(r * 2) == "unseeded", s' = { ...s, creditBatched: s.creditBatched.set(r, false) } }
  }
  action TheCreditAccountHoldChanges = {
    nondet r = Runs.oneOf()
    nondet held = Set(true, false).oneOf()
    s' = { ...s, creditHold: if (held) s.creditHold.union(Set(r)) else s.creditHold.exclude(Set(r)), panel: "settlement-held" }
  }
  action TheInternalCreditTransactionIsSuspended = all { s.flight != -1, flightLeaseCurrent, isCredit(s.flight),
    s.resultReady.contains(s.flight), (not(s.runnable.get(runOf(s.flight))) or (s.creditBatched.get(runOf(s.flight)) and s.creditHold.contains(runOf(s.flight)))),
    s' = { ...s, leg: s.leg.set(s.flight, "retry"), flight: -1, error: "policy_or_credit_hold", panel: "settlement-held" } }
  action TheCreditAuditIsMirrored = {
    nondet l = Set(0, 2).oneOf()
    all { s.ledger.contains(l), s' = { ...s, audited: s.audited.union(Set(l)) } }
  }
  // Pipeline NEAR is an outbox for finalized credit batches, disjoint from account payouts.
  action ThePipelineNearCallIsPrepared = {
    nondet r = Runs.oneOf()
    all { s.batch.contains(r), s.near.get(r) == "pending", s.runnable.get(r), s.nearMode != "disabled", s.nearFlight == -1,
      s' = { ...s, near: s.near.set(r, "prepared"), nearContract: s.nearContract.set(r, s.configuredContract), panel: "pipeline-near" } }
  }
  action ThePipelineNearCallIsDispatched = {
    nondet r = Runs.oneOf()
    all { s.near.get(r) == "prepared", s.runnable.get(r), s.nearContract.get(r) == s.configuredContract, s.nearMode != "disabled", s.nearFlight == -1,
      s' = { ...s, nearFlight: r, panel: "pipeline-near" } }
  }
  action ThePipelineNearAdapterTakesEffect = all { s.nearFlight != -1,
    s' = { ...s, nearEffect: s.nearEffect.union(Set(s.nearFlight)) } }
  action ThePipelineNearSubmitIsRecorded = all { s.nearFlight != -1, s.nearEffect.contains(s.nearFlight),
    s' = { ...s, near: s.near.set(s.nearFlight, "submitted"), submittedMode: s.submittedMode.set(s.nearFlight, s.nearMode), nearFlight: -1, panel: "pipeline-near" } }
  action ThePipelineNearSubmitCrashesLocally = all { s.nearFlight != -1, s.nearEffect.contains(s.nearFlight),
    s' = { ...s, nearFlight: -1, error: "near_submit_record_missing", panel: "settlement-uncertain" } }
  action ThePipelineNearSubmitFails = all { s.nearFlight != -1, not(s.nearEffect.contains(s.nearFlight)),
    s' = { ...s, near: s.near.set(s.nearFlight, "submit_failed"), nearFlight: -1, panel: "settlement-uncertain" } }
  action YouRetryFailedNearSubmit = {
    nondet r = Runs.oneOf()
    all { controls(s.panel).contains("YouRetryFailedNearSubmit"), s.near.get(r) == "submit_failed", s.runnable.get(r),
      s' = { ...s, near: s.near.set(r, "prepared"), panel: "pipeline-near" } }
  }
  // A suspended policy does not block confirmation of an already submitted line.
  action ThePipelineNearTransactionIsConfirmed = {
    nondet r = Runs.oneOf()
    all { s.near.get(r) == "submitted", s.submittedMode.get(r) == s.nearMode,
      s' = { ...s, near: s.near.set(r, "confirmed"), panel: "pipeline-near" } }
  }
  action ThePipelineNearTransactionFailsOnChain = {
    nondet r = Runs.oneOf()
    all { s.near.get(r) == "submitted", s.submittedMode.get(r) == s.nearMode,
      s' = { ...s, near: s.near.set(r, "chain_failed"), error: "near_transaction_failed", panel: "settlement-uncertain" } }
  }
  action TheNearDeploymentConfigurationChanges = {
    nondet mode = Set("disabled", "dry_run", "http").oneOf()
    nondet contract = Runs.oneOf()
    all { s.nearFlight == -1, s' = { ...s, configuredContract: contract, nearMode: mode } }
  }
  action TheStoredNearContractIsRefused = {
    nondet r = Runs.oneOf()
    all { s.near.get(r) == "prepared", s.runnable.get(r), s.nearContract.get(r) != s.configuredContract,
      s' = { ...s, near: s.near.set(r, "contract_failed"), error: "near_contract_changed", panel: "settlement-held" } }
  }
  // Abstract account accrual event: account identity and stream remain separate from pipeline op keys.
  action TheAccountAccrualPassFindsOwedCredit = {
    nondet l = Legs.oneOf()
    s' = { ...s, owed: s.owed.set(l, true), panel: "account-payout" }
  }
  action TheAccountLinkIsActivated = {
    nondet a = Runs.oneOf()
    all { s.link.get(a) == "waiting", s' = { ...s, link: s.link.set(a, "active"), panel: "account-payout" } }
  }
  action TheAccountPayoutLineIsPersisted = {
    nondet l = Legs.oneOf()
    val a = runOf(l)
    all { s.owed.get(l), s.link.get(a) == "active", s.accountLine.get(l) == "none", not(s.accountHalted),
      not(s.accountHold.contains(a)), not(s.streamHold.contains(l)), (isCredit(l) or not(s.referralHold)), s.budget == 2,
      s.accountMode == "live", (isCredit(l) or s.referralMode == "live"),
      s' = { ...s, accountLine: s.accountLine.set(l, "pending"), budget: 1, panel: "account-payout" } }
  }
  action TheAccountRailSendsThePendingLine = {
    nondet l = Legs.oneOf()
    all { s.accountLine.get(l) == "pending", s.accountFlight == -1, not(s.accountHalted), s.budget > 0,
      not(s.accountHold.contains(runOf(l))), not(s.streamHold.contains(l)), (isCredit(l) or not(s.referralHold)),
      s' = { ...s, accountFlight: l, budget: s.budget - 1, panel: "account-payout" } }
  }
  action TheAccountRailCommitsTheTransfer = all { s.accountFlight != -1,
    s' = { ...s, accountEffect: s.accountEffect.union(Set(s.accountFlight)), panel: "settlement-uncertain" } }
  action TheAccountReceiptIsRecorded = all { s.accountFlight != -1, s.accountEffect.contains(s.accountFlight), s.accountLine.get(s.accountFlight) == "pending",
    s' = { ...s, accountLine: s.accountLine.set(s.accountFlight, "confirmed"), accountConfirmed: s.accountConfirmed.union(Set(s.accountFlight)), accountFlight: -1, panel: "account-payout" } }
  action TheAccountSendCrashesBeforeLocalCommit = all { s.accountFlight != -1, s.accountEffect.contains(s.accountFlight),
    s' = { ...s, accountFlight: -1, error: "payout_store_failed", panel: "settlement-uncertain" } }
  action TheAccountPassRecoversTheMatchingReceipt = {
    nondet l = Legs.oneOf()
    all { s.accountLine.get(l) == "pending", s.accountEffect.contains(l), s.accountFlight != l, not(s.accountHalted), not(s.accountHold.contains(runOf(l))), not(s.streamHold.contains(l)), (isCredit(l) or not(s.referralHold)),
      s' = { ...s, accountLine: s.accountLine.set(l, "confirmed"), accountConfirmed: s.accountConfirmed.union(Set(l)), panel: "account-payout" } }
  }
  action ThePendingAccountLineIsEndedConcurrently = {
    nondet l = Legs.oneOf()
    all { s.accountLine.get(l) == "pending", s' = { ...s, accountLine: s.accountLine.set(l, "ended"), panel: "account-payout" } }
  }
  action TheCommittedAccountTransferNeedsAnOperator = all { s.accountFlight != -1, s.accountEffect.contains(s.accountFlight), s.accountLine.get(s.accountFlight) == "ended",
    s' = { ...s, streamHold: s.streamHold.union(Set(s.accountFlight)), accountFlight: -1, error: "payout_receipt_unrecorded", panel: "settlement-held" } }
  action TheAccountRailRefusesTheTransfer = all { s.accountFlight != -1, not(s.accountEffect.contains(s.accountFlight)),
    s' = { ...s, accountFlight: -1, error: "rail_refusal", panel: "settlement-uncertain" } }
  action TheReferralAbuseControlTrips = s' = { ...s, referralHold: true, referralMode: "dry_run", panel: "settlement-held" }
  action TheAccountLinkStartsClosing = {
    nondet a = Runs.oneOf()
    all { s.link.get(a) == "active", s' = { ...s, link: s.link.set(a, "closing"), panel: "account-payout" } }
  }
  action TheAccountLinkClosesAfterPendingWork = {
    nondet a = Runs.oneOf()
    all { s.link.get(a) == "closing", s.accountLine.get(a * 2) != "pending", s.accountLine.get(a * 2 + 1) != "pending", s.accountFlight == -1,
      s' = { ...s, link: s.link.set(a, "closed"), panel: "account-payout" } }
  }
  action YouHaltAccountRail = all { controls(s.panel).contains("YouHaltAccountRail"), s' = { ...s, accountHalted: true, panel: "settlement-held" } }
  action YouResumeAccountRail = all { controls(s.panel).contains("YouResumeAccountRail"), s' = { ...s, accountHalted: false, panel: "account-payout" } }
  action YouClearAccountHold = {
    nondet a = Runs.oneOf()
    all { controls(s.panel).contains("YouClearAccountHold"), s' = { ...s, accountHold: s.accountHold.exclude(Set(a)), streamHold: s.streamHold.exclude(Set(a * 2, a * 2 + 1)), panel: "account-payout" } }
  }
  action TheAccountStandingChanges = {
    nondet a = Runs.oneOf()
    nondet held = Set(true, false).oneOf()
    s' = { ...s, accountHold: if (held) s.accountHold.union(Set(a)) else s.accountHold.exclude(Set(a)), panel: "account-payout" }
  }
  action TheAccountWorkerRetryTickArrives = s' = { ...s, budget: 2, panel: "account-payout" }
  action step = any {
    TheScoreCommitsTwoInstrumentAwards, TheSettleSelectionIsPersisted, TheIndexDispatchIsResolved,
    TheSubmissionBecomesInoperable, TheLeaseExpires, AnotherWorkerReclaimsTheLease,
    YouSuspendSettlePolicy, YouResumeSettlePolicy, TheWorkerReadsTheSubmissionGuard, TheWorkerEndsTheSettleAttempt, TheWorkerDispatchesAnInstrumentOperation,
    TheAdapterReturnsAReceipt, TheExternalInstrumentMayTakeEffectWithoutAReceipt, TheAdapterReturnsUnavailable, TheWorkerCrashesAfterExternalSuccess,
    TheStaleAdapterResultIsDiscarded, TheExternalReceiptIsRecorded, TheInternalCreditTransactionCommits, TheDuplicateAwardIsWithheld,
    TheWorkerForfeitsAnUnfinishedLeg, TheReturnedCreditFindsWithdrawal, TheReturnedReceiptDoesNotAnswerTheOperation,
    TheFailurePathCannotReconcileAnExternalLeg, TheBoundCreditEventUsesAnUnbatchedCompatibilityRule,
    TheCreditAccountHoldChanges, TheInternalCreditTransactionIsSuspended, TheCreditAuditIsMirrored,
    ThePipelineNearCallIsPrepared, ThePipelineNearCallIsDispatched, ThePipelineNearAdapterTakesEffect,
    ThePipelineNearSubmitIsRecorded, ThePipelineNearSubmitCrashesLocally, ThePipelineNearSubmitFails,
    YouRetryFailedNearSubmit, ThePipelineNearTransactionIsConfirmed, ThePipelineNearTransactionFailsOnChain,
    TheNearDeploymentConfigurationChanges, TheStoredNearContractIsRefused,
    TheAccountAccrualPassFindsOwedCredit, TheAccountLinkIsActivated, TheAccountPayoutLineIsPersisted,
    TheAccountRailSendsThePendingLine, TheAccountRailCommitsTheTransfer, TheAccountReceiptIsRecorded,
    TheAccountSendCrashesBeforeLocalCommit, TheAccountPassRecoversTheMatchingReceipt,
    ThePendingAccountLineIsEndedConcurrently, TheCommittedAccountTransferNeedsAnOperator,
    TheAccountRailRefusesTheTransfer, TheReferralAbuseControlTrips,
    TheAccountLinkStartsClosing, TheAccountLinkClosesAfterPendingWork,
    YouHaltAccountRail, YouResumeAccountRail, YouClearAccountHold, TheAccountStandingChanges, TheAccountWorkerRetryTickArrives
  }
}
