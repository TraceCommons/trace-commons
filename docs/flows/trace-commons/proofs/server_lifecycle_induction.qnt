// Proof-only finite arbitrary-state harness. Imports every original action unchanged.
module server_lifecycle_induction {
  import server_lifecycle.* from "../models/server_lifecycle"
  val Domain = Panels.contains(s.panel)
    and Actors.contains(s.actor)
    and Actors.contains(s.tenant)
    and Objects.setOfMaps(Set("accepted", "withdrawn", "revoked", "expired", "purged")).contains(s.status)
    and Objects.powerset().contains(s.expired)
    and Objects.powerset().contains(s.legalHold)
    and Objects.powerset().contains(s.sessionBound)
    and Actors.powerset().contains(s.withdrawnSessions)
    and Objects.powerset().contains(s.command)
    and Objects.powerset().contains(s.member)
    and Objects.setOfMaps(Set("none", "pending", "complete", "cancelled", "failed")).contains(s.write)
    and Objects.powerset().contains(s.physical)
    and Objects.powerset().contains(s.completeRun)
    and Objects.powerset().contains(s.payload)
    and Objects.setOfMaps(Set("none", "pending", "complete", "failed")).contains(s.invalidation)
    and Objects.powerset().contains(s.due)
    and Objects.union(Set(-1)).contains(s.cleanupClaim)
    and Set(false, true).contains(s.claimFresh)
    and Objects.union(Set(-1)).contains(s.flight)
    and Set("none", "settle", "rebuild").contains(s.flightKind)
    and Set(false, true).contains(s.locksHeld)
    and Set(false, true).contains(s.late)
    and Actors.powerset().contains(s.fence)
    and Actors.powerset().contains(s.processActive)
    and Objects.setOfMaps(Set("none", "pending", "complete", "failed")).contains(s.propagation)
    and Objects.powerset().contains(s.allowedUse)
    and Exports.setOfMaps(Set("none", "ready", "complete", "invalidated")).contains(s.snapshot)
    and Exports.setOfMaps(Objects.powerset()).contains(s.snapshotItems)
    and Exports.setOfMaps(Actors.union(Set(-1))).contains(s.snapshotOwner)
    and Exports.setOfMaps(Actors.union(Set(-1))).contains(s.snapshotTenant)
    and Exports.powerset().contains(s.delivered)
    and Set(false, true).contains(s.exportGrant)
    and Objects.powerset().contains(s.readEligible)
    and Objects.union(Set(-1)).contains(s.request)
    and Set("YouCompleteYourExportSnapshot", "YouRequestAnExportSnapshot", "YouWithdrawAnOwnedSubmission", "accepted", "access-refused", "cancelled", "complete", "expired", "export-complete", "export-ready", "failed", "index-cleanup", "index-writing", "index_invalidation_unavailable", "index_key_conflict", "index_rebuild_fence_unavailable", "index_unavailable", "invalidated", "lifecycle-ready", "none", "not_found", "payload_delete_failed", "pending", "pipeline_export_idempotency_conflict", "pipeline_export_source_inoperable", "purged", "ready", "rebuild", "retention", "revoked", "settle", "source_session_withdrawn", "withdrawal-recorded", "withdrawn").contains(s.error)
    and Exports.powerset().contains(s.completionChecked)
    and Exports.setOfMaps(Set(false, true)).contains(s.eligibleAtCompletion)
    and Exports.setOfMaps(Set(false, true)).contains(s.tenantAuthorizedAtCompletion)
  pure def strengthened(st: {
    panel: str, actor: int, tenant: int, status: int -> str, expired: Set[int], legalHold: Set[int],
    sessionBound: Set[int], withdrawnSessions: Set[int], command: Set[int], member: Set[int],
    write: int -> str, physical: Set[int], completeRun: Set[int], payload: Set[int],
    invalidation: int -> str, due: Set[int], cleanupClaim: int, claimFresh: bool,
    flight: int, flightKind: str, locksHeld: bool, late: bool, fence: Set[int],
    processActive: Set[int], propagation: int -> str, allowedUse: Set[int],
    snapshot: int -> str, snapshotItems: int -> Set[int], snapshotOwner: int -> int,
    snapshotTenant: int -> int, delivered: Set[int], exportGrant: bool,
    readEligible: Set[int], request: int, error: str,
    completionChecked: Set[int], eligibleAtCompletion: int -> bool, tenantAuthorizedAtCompletion: int -> bool
  }): bool = (st.delivered.forall(e => st.completionChecked.contains(e) and st.eligibleAtCompletion.get(e))) and (st.delivered.forall(e => st.completionChecked.contains(e) and st.tenantAuthorizedAtCompletion.get(e))) and Exports.forall(e => st.snapshot.get(e) == "none" or st.snapshotItems.get(e).forall(i => owner(i) == st.snapshotTenant.get(e)))
    and Exports.forall(e => st.snapshotItems.get(e).forall(i => owner(i) == st.snapshotTenant.get(e)))
  val Strength = strengthened(s)
  val Conclusion = P10 and P11
  action arbitraryState = {
    nondet n_panel = Panels.oneOf()
    nondet n_actor = Actors.oneOf()
    nondet n_tenant = Actors.oneOf()
    nondet n_status = Objects.setOfMaps(Set("accepted", "withdrawn", "revoked", "expired", "purged")).oneOf()
    nondet n_expired = Objects.powerset().oneOf()
    nondet n_legalHold = Objects.powerset().oneOf()
    nondet n_sessionBound = Objects.powerset().oneOf()
    nondet n_withdrawnSessions = Actors.powerset().oneOf()
    nondet n_command = Objects.powerset().oneOf()
    nondet n_member = Objects.powerset().oneOf()
    nondet n_write = Objects.setOfMaps(Set("none", "pending", "complete", "cancelled", "failed")).oneOf()
    nondet n_physical = Objects.powerset().oneOf()
    nondet n_completeRun = Objects.powerset().oneOf()
    nondet n_payload = Objects.powerset().oneOf()
    nondet n_invalidation = Objects.setOfMaps(Set("none", "pending", "complete", "failed")).oneOf()
    nondet n_due = Objects.powerset().oneOf()
    nondet n_cleanupClaim = Objects.union(Set(-1)).oneOf()
    nondet n_claimFresh = Set(false, true).oneOf()
    nondet n_flight = Objects.union(Set(-1)).oneOf()
    nondet n_flightKind = Set("none", "settle", "rebuild").oneOf()
    nondet n_locksHeld = Set(false, true).oneOf()
    nondet n_late = Set(false, true).oneOf()
    nondet n_fence = Actors.powerset().oneOf()
    nondet n_processActive = Actors.powerset().oneOf()
    nondet n_propagation = Objects.setOfMaps(Set("none", "pending", "complete", "failed")).oneOf()
    nondet n_allowedUse = Objects.powerset().oneOf()
    nondet n_snapshot = Exports.setOfMaps(Set("none", "ready", "complete", "invalidated")).oneOf()
    nondet n_snapshotItems = Exports.setOfMaps(Objects.powerset()).oneOf()
    nondet n_snapshotOwner = Exports.setOfMaps(Actors.union(Set(-1))).oneOf()
    nondet n_snapshotTenant = Exports.setOfMaps(Actors.union(Set(-1))).oneOf()
    nondet n_delivered = Exports.powerset().oneOf()
    nondet n_exportGrant = Set(false, true).oneOf()
    nondet n_readEligible = Objects.powerset().oneOf()
    nondet n_request = Objects.union(Set(-1)).oneOf()
    nondet n_error = Set("YouCompleteYourExportSnapshot", "YouRequestAnExportSnapshot", "YouWithdrawAnOwnedSubmission", "accepted", "access-refused", "cancelled", "complete", "expired", "export-complete", "export-ready", "failed", "index-cleanup", "index-writing", "index_invalidation_unavailable", "index_key_conflict", "index_rebuild_fence_unavailable", "index_unavailable", "invalidated", "lifecycle-ready", "none", "not_found", "payload_delete_failed", "pending", "pipeline_export_idempotency_conflict", "pipeline_export_source_inoperable", "purged", "ready", "rebuild", "retention", "revoked", "settle", "source_session_withdrawn", "withdrawal-recorded", "withdrawn").oneOf()
    nondet n_completionChecked = Exports.powerset().oneOf()
    nondet n_eligibleAtCompletion = Exports.setOfMaps(Set(false, true)).oneOf()
    nondet n_tenantAuthorizedAtCompletion = Exports.setOfMaps(Set(false, true)).oneOf()
    val candidate = { panel: n_panel, actor: n_actor, tenant: n_tenant, status: n_status, expired: n_expired, legalHold: n_legalHold, sessionBound: n_sessionBound, withdrawnSessions: n_withdrawnSessions, command: n_command, member: n_member, write: n_write, physical: n_physical, completeRun: n_completeRun, payload: n_payload, invalidation: n_invalidation, due: n_due, cleanupClaim: n_cleanupClaim, claimFresh: n_claimFresh, flight: n_flight, flightKind: n_flightKind, locksHeld: n_locksHeld, late: n_late, fence: n_fence, processActive: n_processActive, propagation: n_propagation, allowedUse: n_allowedUse, snapshot: n_snapshot, snapshotItems: n_snapshotItems, snapshotOwner: n_snapshotOwner, snapshotTenant: n_snapshotTenant, delivered: n_delivered, exportGrant: n_exportGrant, readEligible: n_readEligible, request: n_request, error: n_error, completionChecked: n_completionChecked, eligibleAtCompletion: n_eligibleAtCompletion, tenantAuthorizedAtCompletion: n_tenantAuthorizedAtCompletion }
    all { strengthened(candidate), s' = candidate }
  }
  action arbitraryInit = arbitraryState
  action originalInit = init
  action originalStep = step
  val InductionTarget = Strength and Domain
}
