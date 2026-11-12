// Current source behavior with observation-only monitors for selected P5/P8/P11.
module server_receipt {
  type Receipt = {present: bool, bytes: int, sourceBytes: int, sourceAttempt: int, principal: int, bundle: int, submission: int, admission: str}
  type Request = {tenant: int, principal: int, role: str, key: int, bytes: int, sourceBytes: int, submission: int,
    phase: str, bundle: int, admission: str, staged: bool, object: bool, due: bool, validatedControls: bool, completedRetry: bool, result: str, returnedReceipt: Receipt}
  val Tenants = Set(0, 1)
  val Principals = Set(0, 1)
  val Keys = Set(0, 1, 2)
  val Attempts = 0.to(3)
  pure def slot(t: int, k: int): int = t * 3 + k
  pure val emptyReceipt: Receipt = {present: false, bytes: 0, sourceBytes: 0, sourceAttempt: 0, principal: 0, bundle: 0, submission: 0, admission: "none"}
  pure val emptyRequest: Request = {tenant: 0, principal: 0, role: "contributor", key: 0, bytes: 0, sourceBytes: 0, submission: 0,
    phase: "unused", bundle: 0, admission: "none", staged: false, object: false, due: false, validatedControls: false, completedRetry: false, result: "none", returnedReceipt: emptyReceipt}
  type State = {requests: int -> Request, receipts: int -> Receipt, owner: int -> str,
    routing: int -> str, activeBundle: int -> int, runnable: int -> bool,
    controlsPresent: int -> bool, usage: Set[int], usagePrincipal: int -> int, sourceWithdrawn: Set[int], tombstones: Set[int],
    frame: str, outcome: str}
  var s: State
  type Proof = {P5: bool, P8: bool, P11: bool}
  var proof: Proof
  val vars = (s, proof)
  pure def controls(frame: str): Set[str] = Set("OperatorChangesRouting", "OperatorSelectsBundle", "OperatorChangesAdmissionControl")
  action init = {
    val nextState = {requests: Attempts.mapBy(_ => emptyRequest), receipts: 0.to(5).mapBy(_ => emptyReceipt),
    owner: 0.to(5).mapBy(_ => "none"), routing: Tenants.mapBy(_ => "pipeline"),
    activeBundle: Tenants.mapBy(_ => 0), runnable: 0.to(5).mapBy(_ => true),
    controlsPresent: Tenants.mapBy(_ => true), usage: Set(), usagePrincipal: 0.to(5).mapBy(_ => 0), sourceWithdrawn: Set(), tombstones: Set(),
    frame: "receipt_idle", outcome: "none"}
    all { s' = nextState, proof' = {P5: true, P8: true, P11: true} }
  }
  action TheAuthenticatedRequestArrives = {
    nondet a = Attempts.oneOf()
    nondet t = Tenants.oneOf()
    nondet p = Principals.oneOf()
    nondet role = Set("contributor", "reviewer", "admin").oneOf()
    nondet k = Keys.oneOf()
    nondet b = Set(0, 1).oneOf()
    nondet u = Set(0, 1).oneOf()
    val nextState = {...s, requests: s.requests.set(a, {...emptyRequest, tenant: t, principal: p, role: role, key: k,
        bytes: b, submission: u, phase: "arrived"}), frame: "receipt_precheck", outcome: "authenticated"}
    all { s.requests.get(a).phase == "unused",
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  def refusal(r: Request): str = {
    val cell = slot(r.tenant, r.key)
    val saved = s.receipts.get(cell)
    if (saved.present) {
      if (saved.principal != r.principal and r.role == "contributor") "owner_refused"
      else if (saved.bytes == r.bytes) "replayed" else "content_conflict"
    } else if (Attempts.exists(a => { val x = s.requests.get(a)
      x.staged and x.tenant == r.tenant and x.key == r.key and x.bytes != r.bytes })) "content_conflict"
    else if (s.routing.get(r.tenant) == "contained") "contained"
    else if (s.owner.get(slot(r.tenant, r.submission)) == "legacy") "legacy_owned"
    else if (s.routing.get(r.tenant) == "legacy") "not_routed"
    else if (s.tombstones.contains(slot(r.tenant, r.submission))) "tombstoned"
    else if (r.role == "contributor" and not(s.usage.contains(cell)) and s.usage.filter(x => x / 3 == r.tenant).size() >= 2) "tenant_quota_exceeded"
    else if (r.role == "contributor" and not(s.usage.contains(cell)) and s.usage.filter(x => x / 3 == r.tenant and s.usagePrincipal.get(x) == r.principal).size() >= 1) "principal_quota_exceeded"
    else "none"
  }
  action TheCompletedAdmissionRetryReadsTheExistingReceipt = {
    nondet a = Attempts.oneOf()
    nondet t = Tenants.oneOf()
    nondet p = Principals.oneOf()
    nondet role = Set("contributor", "reviewer", "admin").oneOf()
    nondet k = Keys.oneOf()
    nondet bytes = Set(0, 1).oneOf()
    val saved = s.receipts.get(slot(t, k))
    val result = if (saved.principal != p and role == "contributor") "owner_refused"
      else if (saved.bytes == bytes) "replayed" else "content_conflict"
    val nextState = {...s, requests: s.requests.set(a, {...emptyRequest, tenant: t, principal: p, role: role,
        key: k, bytes: bytes, submission: saved.submission, phase: "done", completedRetry: true, result: result,
        returnedReceipt: if (result == "replayed") saved else emptyReceipt}),
        frame: "receipt_result", outcome: result}
    all { s.requests.get(a).phase == "unused", saved.present,
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action ThePrecheckReturns = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val result = if (not(s.controlsPresent.get(r.tenant))) "control_missing" else refusal(r)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: if (result == "none") "prechecked" else "done", validatedControls: result == "none" and s.controlsPresent.get(r.tenant)}),
        frame: if (result == "none") "receipt_precheck" else "receipt_result", outcome: result}
    all { r.phase == "arrived",
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheLocalRescrubAndPreparationFinish = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "prepared", sourceBytes: r.bytes + 2}),
      frame: "receipt_precheck", outcome: "deterministic_only"}
    all { r.phase == "prechecked", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheLocalRescrubOrPreparationFails = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "done"}),
      frame: "receipt_result", outcome: "prepare_or_rescrub_failed"}
    all { r.phase == "prechecked", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheStagingTransactionRuns = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val result = refusal(r)
    val bundle = s.activeBundle.get(r.tenant)
    val ok = result == "none" and s.runnable.get(slot(r.tenant, bundle))
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: if (ok) "staged" else "done", staged: ok, bundle: bundle}),
        usage: if (ok and r.role == "contributor") s.usage.union(Set(slot(r.tenant, r.key))) else s.usage,
        usagePrincipal: if (ok and r.role == "contributor" and not(s.usage.contains(slot(r.tenant, r.key)))) s.usagePrincipal.set(slot(r.tenant, r.key), r.principal) else s.usagePrincipal,
        frame: if (ok) "receipt_staged" else "receipt_result",
        outcome: if (ok) "quota_charged_once" else if (result != "none") result else "admission_suspended"}
    all { r.phase == "prepared",
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheAttemptObjectIsPublished = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "written", object: true}), frame: "receipt_staged", outcome: "encrypted_object_written"}
    all { r.phase == "staged",
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheSynchronousAdmissionFinishes = {
    nondet a = Attempts.oneOf()
    nondet decision = Set("admit", "quarantine", "reject").oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "admitted", admission: decision}),
      frame: "receipt_staged", outcome: decision}
    all { r.phase == "written", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheFinalReceiptTransactionRuns = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val cell = slot(r.tenant, r.key)
    // Source-session check precedes the existing-run lookup in the final transaction.
    val result = if (s.sourceWithdrawn.contains(slot(r.tenant, r.principal))) "source_session_withdrawn"
      else if (s.receipts.get(cell).present) refusal(r)
      else if (s.routing.get(r.tenant) == "contained") "contained"
      else if (s.owner.get(slot(r.tenant, r.submission)) == "legacy") "legacy_owned"
      else if (s.routing.get(r.tenant) != "pipeline") "not_routed"
      else if (s.tombstones.contains(slot(r.tenant, r.submission))) "tombstoned"
      else if (not(r.staged) or r.due) "staging_missing"
      else if (not(s.runnable.get(slot(r.tenant, r.bundle)))) "admission_suspended"
      else if (s.owner.get(slot(r.tenant, r.submission)) == "pipeline") "submission_bound_to_another_key"
      else "created"
    val created = result == "created"
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: if (created) "committed" else if (result == "submission_bound_to_another_key") "orphan" else "discard", staged: not(created) and r.staged}),
        receipts: if (created) s.receipts.set(cell, {present: true, bytes: r.bytes, sourceBytes: r.sourceBytes, sourceAttempt: a, principal: r.principal,
          bundle: r.bundle, submission: r.submission, admission: r.admission}) else s.receipts,
        owner: if (created) s.owner.set(slot(r.tenant, r.submission), "pipeline") else s.owner,
        frame: "receipt_result", outcome: result}
    all { r.phase == "admitted",
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheReceiptResponseIsDelivered = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "responded"}),
      frame: "receipt_result", outcome: "client_receives_processing_receipt"}
    all { r.phase == "committed", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action ThePostCommitAuditOrResponseFails = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "response_lost"}),
      frame: "receipt_result", outcome: "durable_receipt_but_client_error_or_no_reply"}
    all { r.phase == "committed", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheRequestCrashesOrStorageFails = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: "orphan"}), frame: "receipt_orphans", outcome: "no_receipt_committed"}
    all { Set("staged", "written", "admitted").contains(r.phase),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheAttemptBecomesDue = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, due: true}), frame: "receipt_orphans", outcome: "cleanup_due"}
    all { r.staged, not(r.due), all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheDiscardOrSweepDeletesTheAttempt = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, phase: if (r.phase == "discard") "done" else r.phase, staged: false, object: false}), frame: "receipt_orphans", outcome: "object_and_staging_deleted"}
    all { r.phase == "discard" or (r.staged and r.due),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheAttemptCleanupFails = {
    nondet a = Attempts.oneOf()
    val r = s.requests.get(a)
    val nextState = {...s, requests: s.requests.set(a, {...r, due: true, staged: true}), frame: "receipt_orphans", outcome: "cleanup_retained_for_retry"}
    all { r.phase == "discard" or (r.staged and r.due),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheSourceSessionIsWithdrawn = {
    nondet t = Tenants.oneOf()
    nondet p = Principals.oneOf()
    val nextState = {...s, sourceWithdrawn: s.sourceWithdrawn.union(Set(slot(t, p)))}
    all { not(s.sourceWithdrawn.contains(slot(t, p))), all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheSubmissionIsTombstoned = {
    nondet t = Tenants.oneOf()
    nondet u = Set(0, 1).oneOf()
    val nextState = {...s, tombstones: s.tombstones.union(Set(slot(t, u)))}
    all { not(s.tombstones.contains(slot(t, u))), all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheLegacyPathClaimsAnUnownedSubmission = {
    nondet t = Tenants.oneOf()
    nondet u = Set(0, 1).oneOf()
    val nextState = {...s, owner: s.owner.set(slot(t, u), "legacy")}
    all { s.owner.get(slot(t, u)) == "none", all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action OperatorChangesRouting = {
    nondet t = Tenants.oneOf()
    nondet route = Set("legacy", "pipeline", "contained").oneOf()
    val nextState = {...s, routing: s.routing.set(t, route), outcome: "qualified_routing_intervention"}
    all { controls(s.frame).contains("OperatorChangesRouting"), route != s.routing.get(t),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action OperatorSelectsBundle = {
    nondet t = Tenants.oneOf()
    val nextState = {...s, activeBundle: s.activeBundle.set(t, 1 - s.activeBundle.get(t)), outcome: "qualified_activation_or_rollback"}
    all { controls(s.frame).contains("OperatorSelectsBundle"),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action OperatorChangesAdmissionControl = {
    nondet cell = 0.to(5).oneOf()
    val nextState = {...s, runnable: s.runnable.set(cell, not(s.runnable.get(cell))), outcome: "policy_intervention"}
    all { controls(s.frame).contains("OperatorChangesAdmissionControl"),
      all { s' = nextState, proof' = observe(s, nextState, proof) } }
  }
  action TheAuthorityOrPrivacyControlChanges = {
    nondet t = Tenants.oneOf()
    val nextState = {...s, controlsPresent: s.controlsPresent.set(t, not(s.controlsPresent.get(t)))}
    all { s' = nextState, proof' = observe(s, nextState, proof) }
  }

  // Observers do not guard, constrain, or change the source transition.
  pure def observe(before: State, after: State, old: Proof): Proof = {
    val preserved = 0.to(5).forall(cell => not(before.receipts.get(cell).present) or after.receipts.get(cell) == before.receipts.get(cell))
    val outcomes = Attempts.forall(a => {
      val r = after.requests.get(a)
      val changed = r != before.requests.get(a)
      val saved = before.receipts.get(slot(r.tenant, r.key))
      val authorized = saved.principal == r.principal or r.role != "contributor"
      val replay = not(changed and after.outcome == "replayed") or
        (saved.present and saved.bytes == r.bytes and authorized and after.receipts == before.receipts and after.usage == before.usage)
      val conflict = not(changed and saved.present and authorized and after.outcome == "content_conflict") or saved.bytes != r.bytes
      replay and conflict
    })
    val completedRetries = Attempts.forall(a => {
      val r = after.requests.get(a)
      val isRetryEvent = before.requests.get(a).phase == "unused" and r.completedRetry
      val saved = before.receipts.get(slot(r.tenant, r.key))
      val expected = if (saved.principal != r.principal and r.role == "contributor") "owner_refused"
        else if (saved.bytes == r.bytes) "replayed" else "content_conflict"
      not(isRetryEvent) or (saved.present and r.phase == "done" and r.result == expected and after.outcome == expected
        and r.submission == saved.submission
        and r.returnedReceipt == (if (expected == "replayed") saved else emptyReceipt)
        and after.receipts == before.receipts and after.owner == before.owner
        and after.usage == before.usage and after.usagePrincipal == before.usagePrincipal)
    })
    val controls = Attempts.forall(a => {
      val r = after.requests.get(a)
      val was = before.requests.get(a)
      (not(was.phase == "arrived" and r.phase == "prechecked") or before.controlsPresent.get(r.tenant)) and
        (not(was.phase == "admitted" and r.phase == "committed") or was.validatedControls)
    })
    val scopedWrites = 0.to(5).forall(cell => after.receipts.get(cell) == before.receipts.get(cell) or
      Attempts.exists(a => {
        val was = before.requests.get(a)
        val r = after.requests.get(a)
        was.phase == "admitted" and r.phase == "committed" and slot(was.tenant, was.key) == cell
      }))
    {P5: old.P5 and preserved and outcomes and completedRetries, P8: old.P8 and controls, P11: old.P11 and scopedWrites and Attempts.forall(a => {
      val was = before.requests.get(a)
      val r = after.requests.get(a)
      was.phase == "unused" or (was.tenant == r.tenant and was.principal == r.principal and was.role == r.role)
    })}
  }
  val P5 = proof.P5
  val P8 = proof.P8
  val P11 = proof.P11
  action step = any { TheAuthenticatedRequestArrives, TheCompletedAdmissionRetryReadsTheExistingReceipt, ThePrecheckReturns, TheLocalRescrubAndPreparationFinish,
    TheLocalRescrubOrPreparationFails, TheStagingTransactionRuns, TheAttemptObjectIsPublished,
    TheSynchronousAdmissionFinishes, TheFinalReceiptTransactionRuns, TheReceiptResponseIsDelivered, ThePostCommitAuditOrResponseFails, TheRequestCrashesOrStorageFails,
    TheAttemptBecomesDue, TheDiscardOrSweepDeletesTheAttempt, TheAttemptCleanupFails,
    TheSourceSessionIsWithdrawn, TheSubmissionIsTombstoned, TheLegacyPathClaimsAnUnownedSubmission,
    OperatorChangesRouting, OperatorSelectsBundle, OperatorChangesAdmissionControl, TheAuthorityOrPrivacyControlChanges }
}
