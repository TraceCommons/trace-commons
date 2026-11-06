// Physical presence and logical eligibility are deliberately independent.
module server_lifecycle {
  pure val Objects = Set(0, 1, 2)
  pure val Actors = Set(0, 1)
  pure val Exports = Set(0, 1)
  pure def owner(i: int): int = if (i < 2) 0 else 1
  pure def session(i: int): int = owner(i)
  pure val Panels = Set("lifecycle-ready", "index-writing", "index-cleanup", "withdrawal-recorded", "retention", "export-ready", "export-complete", "access-refused")
  pure def controls(frame: str): Set[str] = if (Panels.contains(frame)) Set("YouWithdrawAnOwnedSubmission", "YouRequestAnExportSnapshot", "YouCompleteYourExportSnapshot") else Set()
  var s: {
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
  }
  val vars = { s: s }
  def live(i: int): bool = s.status.get(i) == "accepted" and not(s.expired.contains(i))
  def locked(i: int): bool = s.flight == i and s.locksHeld
  // P10: Taking a trace back, revoking it or reaching expiry prevents a new
  // export from completing with that trace. Previously delivered exports are
  // not recalled. Eligibility is observed at the locked completion boundary.
  val P10 = s.delivered.forall(e => s.completionChecked.contains(e) and s.eligibleAtCompletion.get(e))
  // P11 (export seam): Another tenant's request cannot complete an export of
  // my trace. This checks the modeled tenant/requester boundary, not all RLS.
  val P11 = s.delivered.forall(e => s.completionChecked.contains(e) and s.tenantAuthorizedAtCompletion.get(e))
    and Exports.forall(e => s.snapshot.get(e) == "none" or
      s.snapshotItems.get(e).forall(i => owner(i) == s.snapshotTenant.get(e)))
  action init = s' = {
    panel: "lifecycle-ready", actor: 0, tenant: 0, status: Objects.mapBy(_ => "accepted"), expired: Set(), legalHold: Set(),
    sessionBound: Set(0, 1, 2), withdrawnSessions: Set(), command: Set(), member: Set(),
    write: Objects.mapBy(_ => "none"), physical: Set(), completeRun: Set(), payload: Objects,
    invalidation: Objects.mapBy(_ => "none"), due: Set(), cleanupClaim: -1, claimFresh: false,
    flight: -1, flightKind: "none", locksHeld: false, late: false, fence: Set(), processActive: Set(),
    propagation: Objects.mapBy(_ => "none"), allowedUse: Objects,
    snapshot: Exports.mapBy(_ => "none"), snapshotItems: Exports.mapBy(_ => Set()),
    snapshotOwner: Exports.mapBy(_ => -1), snapshotTenant: Exports.mapBy(_ => -1), delivered: Set(), exportGrant: true,
    readEligible: Set(), request: -1, error: "none", completionChecked: Set(),
    eligibleAtCompletion: Exports.mapBy(_ => false), tenantAuthorizedAtCompletion: Exports.mapBy(_ => false)
  }
  // Boundary: auth middleware resolved this request identity, not an envelope field.
  action TheAuthenticatedRequestContextChanges = {
    nondet a = Actors.oneOf()
    nondet t = Actors.oneOf()
    nondet grant = Set(true, false).oneOf()
    s' = { ...s, actor: a, tenant: t, exportGrant: grant }
  }
  action TheScoreSealsAnIndexCommand = {
    nondet i = Objects.oneOf()
    all { live(i), s' = { ...s, command: s.command.union(Set(i)), panel: "lifecycle-ready" } }
  }
  action TheSettleSelectionIncludesARevision = {
    nondet i = Objects.oneOf()
    all { live(i), s.command.contains(i), s.write.get(i) == "none",
      s' = { ...s, member: s.member.union(Set(i)), write: s.write.set(i, "pending"), panel: "index-writing" } }
  }
  action TheSettleIndexCallStarts = {
    nondet i = Objects.oneOf()
    all { s.flight == -1, s.write.get(i) == "pending", live(i),
      s' = { ...s, flight: i, flightKind: "settle", locksHeld: true, late: false, panel: "index-writing" } }
  }
  action ThePipelineRunCompletes = {
    nondet i = Objects.oneOf()
    all { live(i), Set("none", "complete", "cancelled").contains(s.write.get(i)), s' = { ...s, completeRun: s.completeRun.union(Set(i)), panel: "lifecycle-ready" } }
  }
  action TheLocalProcessRoutingChanges = {
    nondet t = Actors.oneOf()
    nondet active = Set(true, false).oneOf()
    s' = { ...s, processActive: if (active) s.processActive.union(Set(t)) else s.processActive.exclude(Set(t)) }
  }
  action TheRebuildCommitsItsFenceAndStartsAWrite = {
    nondet i = Objects.oneOf()
    all { s.flight == -1, not(s.processActive.contains(owner(i))), s.completeRun.contains(i),
      s.command.contains(i), s.member.contains(i), live(i), s.invalidation.get(i) == "none",
      s' = { ...s, fence: s.fence.union(Set(owner(i))), flight: i, flightKind: "rebuild", locksHeld: true, late: false, panel: "index-writing" } }
  }
  action TheRebuildCannotPersistItsFence = {
    nondet t = Actors.oneOf()
    all { s.flight == -1, not(s.processActive.contains(t)),
      s' = { ...s, error: "index_rebuild_fence_unavailable", panel: "access-refused" } }
  }
  action TheIndexDeadlineOrDatabaseSessionEnds = all { s.flight != -1, s.locksHeld,
    s' = { ...s, locksHeld: false, late: true, error: "index_unavailable", panel: "index-cleanup" } }
  action TheIndexWriteReturnsBeforeTheDeadline = all { s.flight != -1, s.locksHeld,
    s' = { ...s, physical: s.physical.union(Set(s.flight)),
      write: if (s.flightKind == "settle") s.write.set(s.flight, "complete") else s.write,
      fence: if (s.flightKind == "rebuild") s.fence.exclude(Set(owner(s.flight))) else s.fence,
      flight: -1, flightKind: "none", locksHeld: false, panel: "lifecycle-ready" } }
  action TheIndexWriteReportsAContentConflict = all { s.flight != -1,
    s' = { ...s, write: if (s.flightKind == "settle") s.write.set(s.flight, "failed") else s.write,
      fence: if (s.flightKind == "rebuild") s.fence.exclude(Set(owner(s.flight))) else s.fence,
      flight: -1, flightKind: "none", locksHeld: false, error: "index_key_conflict", panel: "index-cleanup" } }
  action TheFenceMarginExpires = {
    nondet t = Actors.oneOf()
    all { s.fence.contains(t), not(s.locksHeld), s' = { ...s, fence: s.fence.exclude(Set(t)), panel: "index-cleanup" } }
  }
  // A late call may write after cleanup; reopening applies only to pending/complete rows.
  action TheLateIndexWriteReturns = all { s.flight != -1, s.late, not(s.locksHeld),
    s' = { ...s, physical: s.physical.union(Set(s.flight)),
      invalidation: if (Set("pending", "complete").contains(s.invalidation.get(s.flight))) s.invalidation.set(s.flight, "pending") else s.invalidation,
      due: if (Set("pending", "complete").contains(s.invalidation.get(s.flight))) s.due.union(Set(s.flight)) else s.due,
      fence: if (s.flightKind == "rebuild") s.fence.exclude(Set(owner(s.flight))) else s.fence,
      flight: -1, flightKind: "none", late: false, panel: "index-cleanup" } }
  action TheIndexEntryIsLostWithTheIndexStore = {
    nondet i = Objects.oneOf()
    s' = { ...s, physical: s.physical.exclude(Set(i)), panel: "index-cleanup" }
  }
  action TheConsumerReadsLogicalIndexEligibility = s' = { ...s,
    readEligible: Objects.filter(i => s.physical.contains(i) and s.member.contains(i) and live(i)), panel: "lifecycle-ready" }
  action TheSourceSessionBindingIsAbsent = {
    nondet i = Objects.oneOf()
    all { s.status.get(i) == "accepted", s' = { ...s, sessionBound: s.sessionBound.exclude(Set(i)) } }
  }
  action YouWithdrawAnOwnedSubmission = {
    nondet i = Objects.oneOf()
    val affected = if (s.sessionBound.contains(i)) Objects.filter(j => session(j) == session(i) and s.sessionBound.contains(j)) else Set(i)
    all { controls(s.panel).contains("YouWithdrawAnOwnedSubmission"), owner(i) == s.actor, owner(i) == s.tenant,
      affected.forall(j => not(locked(j))),
      s' = { ...s, status: Objects.mapBy(j => if (affected.contains(j)) "withdrawn" else s.status.get(j)),
        withdrawnSessions: if (s.sessionBound.contains(i)) s.withdrawnSessions.union(Set(session(i))) else s.withdrawnSessions,
        member: s.member.exclude(affected),
        write: Objects.mapBy(j => if (affected.contains(j) and s.write.get(j) == "pending") "cancelled" else s.write.get(j)),
        invalidation: Objects.mapBy(j => if (affected.contains(j) and s.write.get(j) != "none") "pending" else s.invalidation.get(j)),
        due: s.due.union(affected.filter(j => s.write.get(j) != "none" and s.flight != j)),
        propagation: Objects.mapBy(j => if (affected.contains(j)) "pending" else s.propagation.get(j)),
        snapshot: Exports.mapBy(e => if (s.snapshotItems.get(e).intersect(affected).size() > 0) "invalidated" else s.snapshot.get(e)),
        request: i, error: "none", panel: "withdrawal-recorded" } }
  }
  action TheWithdrawalRequestNamesAnotherOwner = {
    nondet i = Objects.oneOf()
    all { owner(i) != s.actor or owner(i) != s.tenant,
      s' = { ...s, error: "not_found", request: i, panel: "access-refused" } }
  }
  action TheWithdrawnSourceSessionIsSubmittedAgain = {
    nondet t = Actors.oneOf()
    all { s.withdrawnSessions.contains(t), s' = { ...s, error: "source_session_withdrawn", panel: "access-refused" } }
  }
  action TheServerRevokesOneSubmission = {
    nondet i = Objects.oneOf()
    all { not(locked(i)),
      s' = { ...s, status: s.status.set(i, "revoked"), member: s.member.exclude(Set(i)),
        write: if (s.write.get(i) == "pending") s.write.set(i, "cancelled") else s.write,
        invalidation: if (s.write.get(i) != "none") s.invalidation.set(i, "pending") else s.invalidation,
        due: if (s.write.get(i) != "none" and s.flight != i) s.due.union(Set(i)) else s.due,
        propagation: s.propagation.set(i, "pending"),
        snapshot: Exports.mapBy(e => if (s.snapshotItems.get(e).contains(i)) "invalidated" else s.snapshot.get(e)), panel: "withdrawal-recorded" } }
  }
  action TheDatabaseClockPassesExpiry = {
    nondet i = Objects.oneOf()
    s' = { ...s, expired: s.expired.union(Set(i)), panel: "retention" }
  }
  action TheLegalHoldConfigurationChanges = {
    nondet i = Objects.oneOf()
    nondet held = Set(true, false).oneOf()
    s' = { ...s, legalHold: if (held) s.legalHold.union(Set(i)) else s.legalHold.exclude(Set(i)), panel: "retention" }
  }
  action TheRetentionWorkerMarksExpiry = {
    nondet i = Objects.oneOf()
    all { s.expired.contains(i), not(s.legalHold.contains(i)), s.status.get(i) == "accepted", not(locked(i)),
      s' = { ...s, status: s.status.set(i, "expired"), member: s.member.exclude(Set(i)),
        write: if (s.write.get(i) == "pending") s.write.set(i, "cancelled") else s.write,
        invalidation: if (s.write.get(i) != "none") s.invalidation.set(i, "pending") else s.invalidation,
        due: if (s.write.get(i) != "none" and s.flight != i) s.due.union(Set(i)) else s.due,
        snapshot: Exports.mapBy(e => if (s.snapshotItems.get(e).contains(i)) "invalidated" else s.snapshot.get(e)), panel: "retention" } }
  }
  action TheRetentionWorkerMarksPurge = {
    nondet i = Objects.oneOf()
    all { s.status.get(i) == "expired", not(s.legalHold.contains(i)), not(locked(i)),
      s' = { ...s, status: s.status.set(i, "purged"), propagation: s.propagation.set(i, "pending"), panel: "retention" } }
  }
  action ThePropagationWorkerDeletesPayloads = {
    nondet i = Objects.oneOf()
    all { s.propagation.get(i) == "pending",
      s' = { ...s, payload: s.payload.exclude(Set(i)), propagation: s.propagation.set(i, "complete"), panel: "index-cleanup" } }
  }
  action ThePropagationWorkerCannotDeletePayloads = {
    nondet i = Objects.oneOf()
    all { s.propagation.get(i) == "pending", s' = { ...s, propagation: s.propagation.set(i, "failed"), error: "payload_delete_failed", panel: "index-cleanup" } }
  }
  action ThePropagationRetryIsQueued = {
    nondet i = Objects.oneOf()
    all { s.propagation.get(i) == "failed", s' = { ...s, propagation: s.propagation.set(i, "pending"), panel: "index-cleanup" } }
  }
  action TheInvalidationFenceWaitEnds = {
    nondet i = Objects.oneOf()
    all { s.invalidation.get(i) == "pending", not(s.locksHeld), s' = { ...s, due: s.due.union(Set(i)), panel: "index-cleanup" } }
  }
  action TheInvalidationWorkerClaimsDueRemoval = {
    nondet i = Objects.oneOf()
    all { s.cleanupClaim == -1, s.invalidation.get(i) == "pending", s.due.contains(i), not(s.fence.contains(owner(i))),
      s' = { ...s, cleanupClaim: i, claimFresh: true, due: s.due.exclude(Set(i)), panel: "index-cleanup" } }
  }
  action TheInvalidationClaimExpires = all { s.cleanupClaim != -1,
    s' = { ...s, claimFresh: false, due: s.due.union(Set(s.cleanupClaim)) } }
  action TheInvalidationWorkerRemovesPhysicalEntries = all { s.cleanupClaim != -1, s.claimFresh,
    s' = { ...s, physical: s.physical.exclude(Set(s.cleanupClaim)), invalidation: s.invalidation.set(s.cleanupClaim, "complete"), cleanupClaim: -1, claimFresh: false, panel: "index-cleanup" } }
  action TheIndexIsUnavailableDuringRemoval = all { s.cleanupClaim != -1, s.claimFresh,
    s' = { ...s, cleanupClaim: -1, claimFresh: false, error: "index_invalidation_unavailable", panel: "index-cleanup" } }
  action TheStaleInvalidationResultIsIgnored = all { s.cleanupClaim != -1, not(s.claimFresh),
    s' = { ...s, cleanupClaim: -1, panel: "index-cleanup" } }
  action YouRequestAnExportSnapshot = {
    nondet e = Exports.oneOf()
    val items = Objects.filter(i => owner(i) == s.tenant and live(i) and s.completeRun.contains(i) and s.payload.contains(i) and s.allowedUse.contains(i))
    all { controls(s.panel).contains("YouRequestAnExportSnapshot"), s.actor == s.tenant, s.exportGrant, s.snapshot.get(e) == "none",
      s' = { ...s, snapshot: s.snapshot.set(e, "ready"), snapshotItems: s.snapshotItems.set(e, items), snapshotOwner: s.snapshotOwner.set(e, s.actor),
        snapshotTenant: s.snapshotTenant.set(e, s.tenant), panel: "export-ready", error: "none" } }
  }
  action TheExportIdempotencyKeyIsReplayed = {
    nondet e = Exports.oneOf()
    all { s.snapshot.get(e) != "none", s.snapshotOwner.get(e) == s.actor, s.snapshotTenant.get(e) == s.tenant,
      s' = { ...s, panel: if (s.snapshot.get(e) == "complete") "export-complete" else "export-ready" } }
  }
  action TheExportKeyNamesConflictingMetadata = {
    nondet e = Exports.oneOf()
    all { s.snapshot.get(e) != "none", s' = { ...s, error: "pipeline_export_idempotency_conflict", panel: "access-refused" } }
  }
  action TheAllowedUseEvidenceChanges = {
    nondet i = Objects.oneOf()
    s' = { ...s, allowedUse: s.allowedUse.exclude(Set(i)) }
  }
  action YouCompleteYourExportSnapshot = {
    nondet e = Exports.oneOf()
    all { controls(s.panel).contains("YouCompleteYourExportSnapshot"), s.exportGrant,
      s.snapshotOwner.get(e) == s.actor, s.snapshotTenant.get(e) == s.tenant, s.snapshot.get(e) == "ready",
      s.snapshotItems.get(e).forall(i => live(i) and s.allowedUse.contains(i)),
      s' = { ...s, completionChecked: s.completionChecked.union(Set(e)),
        eligibleAtCompletion: s.eligibleAtCompletion.set(e, s.snapshotItems.get(e).forall(i => live(i) and s.allowedUse.contains(i))),
        tenantAuthorizedAtCompletion: s.tenantAuthorizedAtCompletion.set(e,
          s.snapshotOwner.get(e) == s.actor and s.snapshotTenant.get(e) == s.tenant and
          s.snapshotItems.get(e).forall(i => owner(i) == s.tenant)),
        snapshot: s.snapshot.set(e, "complete"), delivered: s.delivered.union(Set(e)), panel: "export-complete", error: "none" } }
  }
  action TheExportCompletionRevalidationRefusesDelivery = {
    nondet e = Exports.oneOf()
    all { s.snapshotOwner.get(e) == s.actor, s.snapshotTenant.get(e) == s.tenant,
      s.snapshot.get(e) == "invalidated" or (s.snapshot.get(e) == "ready" and s.snapshotItems.get(e).exists(i => not(live(i)) or not(s.allowedUse.contains(i)))),
      s' = { ...s, error: "pipeline_export_source_inoperable", panel: "access-refused" } }
  }
  action step = any {
    TheAuthenticatedRequestContextChanges, TheScoreSealsAnIndexCommand, TheSettleSelectionIncludesARevision,
    TheSettleIndexCallStarts, ThePipelineRunCompletes, TheLocalProcessRoutingChanges,
    TheRebuildCommitsItsFenceAndStartsAWrite, TheRebuildCannotPersistItsFence,
    TheIndexDeadlineOrDatabaseSessionEnds, TheIndexWriteReturnsBeforeTheDeadline,
    TheIndexWriteReportsAContentConflict, TheFenceMarginExpires, TheLateIndexWriteReturns, TheIndexEntryIsLostWithTheIndexStore,
    TheConsumerReadsLogicalIndexEligibility, TheSourceSessionBindingIsAbsent, YouWithdrawAnOwnedSubmission, TheWithdrawalRequestNamesAnotherOwner,
    TheWithdrawnSourceSessionIsSubmittedAgain, TheServerRevokesOneSubmission,
    TheDatabaseClockPassesExpiry, TheLegalHoldConfigurationChanges, TheRetentionWorkerMarksExpiry,
    TheRetentionWorkerMarksPurge, ThePropagationWorkerDeletesPayloads, ThePropagationWorkerCannotDeletePayloads,
    ThePropagationRetryIsQueued, TheInvalidationFenceWaitEnds, TheInvalidationWorkerClaimsDueRemoval,
    TheInvalidationClaimExpires, TheInvalidationWorkerRemovesPhysicalEntries, TheIndexIsUnavailableDuringRemoval,
    TheStaleInvalidationResultIsIgnored, YouRequestAnExportSnapshot, TheExportIdempotencyKeyIsReplayed,
    TheExportKeyNamesConflictingMetadata, TheAllowedUseEvidenceChanges, YouCompleteYourExportSnapshot,
    TheExportCompletionRevalidationRefusesDelivery
  }
}
