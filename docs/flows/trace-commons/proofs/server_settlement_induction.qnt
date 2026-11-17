// Proof-only finite arbitrary-state harness. Imports every original action unchanged.
module server_settlement_induction {
  import server_settlement.* from "../models/server_settlement"
  val Domain = Panels.contains(s.panel)
    and Runs.powerset().contains(s.selected)
    and Runs.setOfMaps(Set(false, true)).contains(s.operable)
    and Runs.setOfMaps(Set(false, true)).contains(s.runnable)
    and Runs.setOfMaps(Set(false, true)).contains(s.indexDone)
    and Runs.setOfMaps(Set(false, true)).contains(s.settleGuard)
    and Runs.powerset().contains(s.guardRead)
    and Legs.powerset().contains(s.attempted)
    and Runs.setOfMaps(LeaseTokens).contains(s.leaseToken)
    and Runs.setOfMaps(Set(false, true)).contains(s.liveLease)
    and Legs.setOfMaps(Set("unseeded", "pending", "dispatched", "retry", "complete", "complete_withheld", "mismatch", "unreconciled", "forfeited")).contains(s.leg)
    and Legs.powerset().contains(s.dispatched)
    and Legs.powerset().contains(s.resultReady)
    and Runs.setOfMaps(Set(false, true)).contains(s.creditBatched)
    and Runs.powerset().contains(s.creditHold)
    and Legs.powerset().contains(s.externalEffect)
    and Legs.powerset().contains(s.ledger)
    and Legs.powerset().contains(s.audited)
    and Runs.powerset().contains(s.batch)
    and Runs.setOfMaps(Set("none", "pending", "prepared", "submitted", "confirmed", "submit_failed", "chain_failed", "contract_failed")).contains(s.near)
    and Runs.powerset().contains(s.nearEffect)
    and Runs.setOfMaps(Runs).contains(s.nearContract)
    and Runs.contains(s.configuredContract)
    and Set("disabled", "dry_run", "http").contains(s.nearMode)
    and Runs.setOfMaps(Set("none", "disabled", "dry_run", "http")).contains(s.submittedMode)
    and Runs.union(Set(-1)).contains(s.nearFlight)
    and Legs.union(Set(-1)).contains(s.flight)
    and LeaseTokens.union(Set(-1)).contains(s.flightToken)
    and Legs.setOfMaps(Set("none", "pending", "confirmed", "ended")).contains(s.accountLine)
    and Legs.powerset().contains(s.accountEffect)
    and Legs.powerset().contains(s.accountConfirmed)
    and Runs.setOfMaps(Set("waiting", "active", "closing", "closed")).contains(s.link)
    and Legs.setOfMaps(Set(false, true)).contains(s.owed)
    and Runs.powerset().contains(s.accountHold)
    and Legs.powerset().contains(s.streamHold)
    and Set(false, true).contains(s.referralHold)
    and Set(false, true).contains(s.accountHalted)
    and Set("live", "dry_run", "disabled").contains(s.accountMode)
    and Set("live", "dry_run", "disabled").contains(s.referralMode)
    and Set(0, 1, 2).contains(s.budget)
    and Legs.union(Set(-1)).contains(s.accountFlight)
    and Set("YouClearAccountHold", "YouHaltAccountRail", "YouResumeAccountRail", "YouResumeSettlePolicy", "YouRetryFailedNearSubmit", "YouSuspendSettlePolicy", "account-payout", "active", "adapter_failed", "chain_failed", "closed", "closing", "complete", "complete_withheld", "confirmed", "contract_failed", "disabled", "dispatched", "dry_run", "ended", "forfeited", "http", "live", "local_commit_missing", "mismatch", "near_contract_changed", "near_submit_record_missing", "near_transaction_failed", "none", "payout_receipt_unrecorded", "payout_store_failed", "pending", "pipeline-near", "policy_or_credit_hold", "prepared", "rail_refusal", "retry", "settlement-held", "settlement-progress", "settlement-ready", "settlement-uncertain", "settlement_result_mismatch", "settlement_unreconciled", "stale_lease", "submission_inoperable", "submission_inoperable_after_dispatch", "submit_failed", "submitted", "unreconciled", "unseeded", "waiting").contains(s.error)
    and Legs.setOfMaps(Set(0, 1, 2)).contains(s.creditCommits)
    and Legs.setOfMaps(Legs.union(Set(-1))).contains(s.instrumentFirstKey)
    and Legs.setOfMaps(Legs.union(Set(-1))).contains(s.instrumentLastKey)
    and Runs.setOfMaps(Runs.union(Set(-1))).contains(s.nearFirstKey)
    and Runs.setOfMaps(Runs.union(Set(-1))).contains(s.nearLastKey)
    and Runs.setOfMaps(Runs.union(Set(-1))).contains(s.nearFirstContract)
    and Legs.setOfMaps(Legs.union(Set(-1))).contains(s.accountFirstKey)
    and Legs.setOfMaps(Legs.union(Set(-1))).contains(s.accountLastKey)
  pure def strengthened(st: {
    panel: str, selected: Set[int], operable: int -> bool, runnable: int -> bool,
    indexDone: int -> bool, settleGuard: int -> bool, guardRead: Set[int], attempted: Set[int], leaseToken: int -> int, liveLease: int -> bool, leg: int -> str,
    dispatched: Set[int], resultReady: Set[int], creditBatched: int -> bool, creditHold: Set[int], externalEffect: Set[int], ledger: Set[int], audited: Set[int],
    batch: Set[int], near: int -> str, nearEffect: Set[int], nearContract: int -> int,
    configuredContract: int, nearMode: str, submittedMode: int -> str, nearFlight: int,
    flight: int, flightToken: int, accountLine: int -> str, accountEffect: Set[int], accountConfirmed: Set[int],
    link: int -> str, owed: int -> bool, accountHold: Set[int], streamHold: Set[int], referralHold: bool,
    accountHalted: bool, accountMode: str, referralMode: str, budget: int,
    accountFlight: int, error: str,
    creditCommits: int -> int, instrumentFirstKey: int -> int, instrumentLastKey: int -> int,
    nearFirstKey: int -> int, nearLastKey: int -> int, nearFirstContract: int -> int, accountFirstKey: int -> int, accountLastKey: int -> int
  }): bool = (Legs.forall(l => st.creditCommits.get(l) <= 1)
    and st.ledger.forall(l => st.creditCommits.get(l) == 1)
    and Legs.forall(l => (st.instrumentFirstKey.get(l) == -1 or
      (st.instrumentFirstKey.get(l) == l and st.instrumentLastKey.get(l) == st.instrumentFirstKey.get(l))))
    and Runs.forall(r => (st.nearFirstKey.get(r) == -1 or
      (st.nearFirstKey.get(r) == r and st.nearLastKey.get(r) == st.nearFirstKey.get(r)
        and st.nearContract.get(r) == st.nearFirstContract.get(r))))
    and Legs.forall(l => (st.accountFirstKey.get(l) == -1 or
      (st.accountFirstKey.get(l) == l and st.accountLastKey.get(l) == st.accountFirstKey.get(l)))))
    and Legs.forall(l => (isCredit(l) or st.creditCommits.get(l) == 0))
    and Legs.forall(l => st.creditCommits.get(l) == 0 or
      (st.ledger.contains(l) and st.leg.get(l) == "complete"))
    and (st.flight == -1 or st.leg.get(st.flight) == "dispatched")
    and Runs.forall(r => st.near.get(r) == "none" or st.creditCommits.get(r * 2) == 1)
    and Runs.forall(r => st.nearFirstKey.get(r) == -1 or (st.near.get(r) != "pending" and st.near.get(r) != "none"))
    and (st.nearFlight == -1 or st.creditCommits.get(st.nearFlight * 2) == 1)
    and Runs.forall(r => (st.leg.get(r * 2) == "unseeded") == (st.leg.get(r * 2 + 1) == "unseeded"))
  val Strength = strengthened(s)
  val Conclusion = P9
  action arbitraryState = {
    nondet n_panel = Panels.oneOf()
    nondet n_selected = Runs.powerset().oneOf()
    nondet n_operable = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_runnable = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_indexDone = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_settleGuard = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_guardRead = Runs.powerset().oneOf()
    nondet n_attempted = Legs.powerset().oneOf()
    nondet n_leaseToken = Runs.setOfMaps(LeaseTokens).oneOf()
    nondet n_liveLease = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_leg = Legs.setOfMaps(Set("unseeded", "pending", "dispatched", "retry", "complete", "complete_withheld", "mismatch", "unreconciled", "forfeited")).oneOf()
    nondet n_dispatched = Legs.powerset().oneOf()
    nondet n_resultReady = Legs.powerset().oneOf()
    nondet n_creditBatched = Runs.setOfMaps(Set(false, true)).oneOf()
    nondet n_creditHold = Runs.powerset().oneOf()
    nondet n_externalEffect = Legs.powerset().oneOf()
    nondet n_ledger = Legs.powerset().oneOf()
    nondet n_audited = Legs.powerset().oneOf()
    nondet n_batch = Runs.powerset().oneOf()
    nondet n_near = Runs.setOfMaps(Set("none", "pending", "prepared", "submitted", "confirmed", "submit_failed", "chain_failed", "contract_failed")).oneOf()
    nondet n_nearEffect = Runs.powerset().oneOf()
    nondet n_nearContract = Runs.setOfMaps(Runs).oneOf()
    nondet n_configuredContract = Runs.oneOf()
    nondet n_nearMode = Set("disabled", "dry_run", "http").oneOf()
    nondet n_submittedMode = Runs.setOfMaps(Set("none", "disabled", "dry_run", "http")).oneOf()
    nondet n_nearFlight = Runs.union(Set(-1)).oneOf()
    nondet n_flight = Legs.union(Set(-1)).oneOf()
    nondet n_flightToken = LeaseTokens.union(Set(-1)).oneOf()
    nondet n_accountLine = Legs.setOfMaps(Set("none", "pending", "confirmed", "ended")).oneOf()
    nondet n_accountEffect = Legs.powerset().oneOf()
    nondet n_accountConfirmed = Legs.powerset().oneOf()
    nondet n_link = Runs.setOfMaps(Set("waiting", "active", "closing", "closed")).oneOf()
    nondet n_owed = Legs.setOfMaps(Set(false, true)).oneOf()
    nondet n_accountHold = Runs.powerset().oneOf()
    nondet n_streamHold = Legs.powerset().oneOf()
    nondet n_referralHold = Set(false, true).oneOf()
    nondet n_accountHalted = Set(false, true).oneOf()
    nondet n_accountMode = Set("live", "dry_run", "disabled").oneOf()
    nondet n_referralMode = Set("live", "dry_run", "disabled").oneOf()
    nondet n_budget = Set(0, 1, 2).oneOf()
    nondet n_accountFlight = Legs.union(Set(-1)).oneOf()
    nondet n_error = Set("YouClearAccountHold", "YouHaltAccountRail", "YouResumeAccountRail", "YouResumeSettlePolicy", "YouRetryFailedNearSubmit", "YouSuspendSettlePolicy", "account-payout", "active", "adapter_failed", "chain_failed", "closed", "closing", "complete", "complete_withheld", "confirmed", "contract_failed", "disabled", "dispatched", "dry_run", "ended", "forfeited", "http", "live", "local_commit_missing", "mismatch", "near_contract_changed", "near_submit_record_missing", "near_transaction_failed", "none", "payout_receipt_unrecorded", "payout_store_failed", "pending", "pipeline-near", "policy_or_credit_hold", "prepared", "rail_refusal", "retry", "settlement-held", "settlement-progress", "settlement-ready", "settlement-uncertain", "settlement_result_mismatch", "settlement_unreconciled", "stale_lease", "submission_inoperable", "submission_inoperable_after_dispatch", "submit_failed", "submitted", "unreconciled", "unseeded", "waiting").oneOf()
    nondet n_creditCommits = Legs.setOfMaps(Set(0, 1, 2)).oneOf()
    nondet n_instrumentFirstKey = Legs.setOfMaps(Legs.union(Set(-1))).oneOf()
    nondet n_instrumentLastKey = Legs.setOfMaps(Legs.union(Set(-1))).oneOf()
    nondet n_nearFirstKey = Runs.setOfMaps(Runs.union(Set(-1))).oneOf()
    nondet n_nearLastKey = Runs.setOfMaps(Runs.union(Set(-1))).oneOf()
    nondet n_nearFirstContract = Runs.setOfMaps(Runs.union(Set(-1))).oneOf()
    nondet n_accountFirstKey = Legs.setOfMaps(Legs.union(Set(-1))).oneOf()
    nondet n_accountLastKey = Legs.setOfMaps(Legs.union(Set(-1))).oneOf()
    val candidate = { panel: n_panel, selected: n_selected, operable: n_operable, runnable: n_runnable, indexDone: n_indexDone, settleGuard: n_settleGuard, guardRead: n_guardRead, attempted: n_attempted, leaseToken: n_leaseToken, liveLease: n_liveLease, leg: n_leg, dispatched: n_dispatched, resultReady: n_resultReady, creditBatched: n_creditBatched, creditHold: n_creditHold, externalEffect: n_externalEffect, ledger: n_ledger, audited: n_audited, batch: n_batch, near: n_near, nearEffect: n_nearEffect, nearContract: n_nearContract, configuredContract: n_configuredContract, nearMode: n_nearMode, submittedMode: n_submittedMode, nearFlight: n_nearFlight, flight: n_flight, flightToken: n_flightToken, accountLine: n_accountLine, accountEffect: n_accountEffect, accountConfirmed: n_accountConfirmed, link: n_link, owed: n_owed, accountHold: n_accountHold, streamHold: n_streamHold, referralHold: n_referralHold, accountHalted: n_accountHalted, accountMode: n_accountMode, referralMode: n_referralMode, budget: n_budget, accountFlight: n_accountFlight, error: n_error, creditCommits: n_creditCommits, instrumentFirstKey: n_instrumentFirstKey, instrumentLastKey: n_instrumentLastKey, nearFirstKey: n_nearFirstKey, nearLastKey: n_nearLastKey, nearFirstContract: n_nearFirstContract, accountFirstKey: n_accountFirstKey, accountLastKey: n_accountLastKey }
    all { strengthened(candidate), s' = candidate }
  }
  action arbitraryInit = arbitraryState
  action originalInit = init
  action originalStep = step
  val InductionTarget = Strength and Domain
}
