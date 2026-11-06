// Leased processing after synchronous Admission. No product promises selected.
module server_pipeline {
  type Run = {tenant: int, bundle: int, phase: str, state: str, admission: str, submissionStatus: str,
    attempt: int, token: int, worker: int, live: bool, renewals: int, due: bool,
    privacy: str, reviewAssessment: str, reviewAfterPass: bool, scoreAssessment: str,
    reviewOwner: int, reviewToken: int, reviewLive: bool, operable: bool,
    revision: bool, award: bool, selection: bool, effectsTerminal: bool, paid: bool,
    finishGranted: bool, heldBefore: bool, error: str}
  type HumanWork = {runId: int, token: int, present: bool}
  type Work = {runId: int, token: int, phase: str, present: bool}
  type Artifact = {runId: int, token: int, kind: str, state: str, object: bool, due: bool}
  val Runs = Set(0, 1)
  val Workers = Set(0, 1)
  val Reviewers = Set(0, 1)
  val Phases = Set("review", "score", "settle")
  pure val emptyRun: Run = {tenant: 0, bundle: 0, phase: "none", state: "absent", admission: "none", submissionStatus: "none",
    attempt: 0, token: 0, worker: 0, live: false, renewals: 0, due: true,
    privacy: "none", reviewAssessment: "none", reviewAfterPass: false, scoreAssessment: "none",
    reviewOwner: 0, reviewToken: 0, reviewLive: false, operable: true,
    revision: false, award: false, selection: false, effectsTerminal: false, paid: false,
    finishGranted: false, heldBefore: false, error: "none"}
  pure val emptyWork: Work = {runId: 0, token: 0, phase: "none", present: false}
  pure val emptyArtifact: Artifact = {runId: 0, token: 0, kind: "none", state: "absent", object: false, due: false}
  var s: {runs: int -> Run, workers: int -> Work, humanRequests: int -> HumanWork, artifacts: int -> Artifact,
    runnable: int -> bool, dependency: bool, privacyControl: bool, frame: str, event: str}
  val vars = s
  pure def controls(frame: str): Set[str] = Set("ReviewerClaimsTheHold", "ReviewerApprovesTheHold", "ReviewerRejectsTheHold",
    "OperatorSuspendsOrResumesThePolicy", "OperatorReleasesThePaidRun", "OperatorFailsThePaidRun")
  pure def policyCell(r: Run): int = r.tenant * 6 + r.bundle * 3 + (if (r.phase == "review") 0 else if (r.phase == "score") 1 else 2)
  pure def finishable(r: Run): bool = r.phase == "settle" and r.selection and r.effectsTerminal
  def current(w: Work): bool = {
    val r = s.runs.get(w.runId)
    w.present and r.state == "leased" and r.live and r.token == w.token and r.phase == w.phase
  }
  def reviewable(r: Run): bool = {
    val waitingReview = r.phase == "review" and r.privacy != "none" and (r.admission == "quarantine" or r.privacy == "escalated") and r.reviewAssessment == "none"
    val waitingScore = r.phase == "score" and r.state == "awaiting_review" and r.scoreAssessment == "none"
    Set("pending", "retry", "awaiting_review").contains(r.state) and (waitingReview or waitingScore)
  }
  action init = s' = {runs: Runs.mapBy(_ => emptyRun), workers: Workers.mapBy(_ => emptyWork), humanRequests: Reviewers.mapBy(_ => {runId: 0, token: 0, present: false}),
    artifacts: 0.to(7).mapBy(_ => emptyArtifact), runnable: 0.to(11).mapBy(_ => true),
    dependency: true, privacyControl: true, frame: "pipeline_idle", event: "none"}
  action TheReceiptCommitsItsAdmission = {
    nondet id = Runs.oneOf()
    nondet t = Set(0, 1).oneOf()
    nondet bundle = Set(0, 1).oneOf()
    nondet admission = Set("admit", "quarantine", "reject").oneOf()
    all { s.runs.get(id).state == "absent",
      s' = {...s, runs: s.runs.set(id, {...emptyRun, tenant: t, bundle: bundle, admission: admission, submissionStatus: if (admission == "admit") "received" else if (admission == "quarantine") "quarantined" else "rejected",
        state: if (admission == "reject") "complete" else "pending", phase: if (admission == "reject") "none" else "review"}),
        frame: "pipeline_pending", event: "receipt_and_admission_atomic"} }
  }
  action TheWorkerClaimsOrReclaimsTheRun = {
    nondet id = Runs.oneOf()
    nondet worker = Workers.oneOf()
    val r = s.runs.get(id)
    nondet freshToken = 1.to(16).filter(token => token != r.token
      and not(Workers.exists(w => s.workers.get(w).present and s.workers.get(w).runId == id and s.workers.get(w).token == token))
      and not(0.to(7).exists(a => Set("staged", "committed").contains(s.artifacts.get(a).state) and s.artifacts.get(a).runId == id and s.artifacts.get(a).token == token))).oneOf()
    all { not(s.workers.get(worker).present), r.phase != "none", r.attempt < 2,
      ((Set("pending", "retry").contains(r.state) and r.due) or (r.state == "leased" and not(r.live))),
      s' = {...s, runs: s.runs.set(id, {...r, state: "leased", worker: worker, token: freshToken,
        live: true, renewals: 0, attempt: r.attempt + 1, error: "none"}),
        workers: s.workers.set(worker, {present: true, runId: id, token: freshToken, phase: r.phase}),
        frame: "pipeline_leased", event: "charged_claim"} }
  }
  action TheWorkerLeaseExpires = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { r.state == "leased", r.live,
      s' = {...s, runs: s.runs.set(id, {...r, live: false}), frame: "pipeline_leased", event: "lease_expired"} }
  }
  action TheWorkerRenewsWithinItsCap = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.renewals < 2,
      s' = {...s, runs: s.runs.set(w.runId, {...r, renewals: r.renewals + 1}), event: "bounded_renewal"} }
  }
  action TheStaleWorkerReturns = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val sameToken = r.state == "leased" and r.token == w.token
    all { w.present, not(current(w)), s' = {...s,
      runs: if (sameToken) s.runs.set(w.runId, {...r, state: "retry", live: false, attempt: r.attempt - 1, due: false, error: "lease_expired"}) else s.runs,
      workers: s.workers.set(worker, emptyWork), event: if (sameToken) "token_only_expiry_recorded_uncharged" else "stale_token_changes_nothing"} }
  }
  action TheWorkerStagesAnAttemptArtifact = {
    nondet worker = Workers.oneOf()
    nondet artifact = 0.to(7).oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val kind = if (r.phase == "review" and r.privacy == "none") "privacy_pass" else if (r.phase == "review") "approved" else "score_commands"
    all { current(w), r.phase != "settle", kind != "privacy_pass" or s.privacyControl, Set("absent", "deleted").contains(s.artifacts.get(artifact).state),
      not(0.to(7).exists(id => { val a = s.artifacts.get(id)
        Set("staged", "committed").contains(a.state) and a.runId == w.runId and a.token == w.token and a.kind == kind })),
      kind != "approved" or ((r.admission == "admit" or r.reviewAssessment == "approve") and
        (r.privacy != "escalated" or (r.reviewAfterPass and r.reviewAssessment == "approve"))),
      s' = {...s, artifacts: s.artifacts.set(artifact, {runId: w.runId, token: w.token, kind: kind, state: "staged", object: false, due: false}),
        event: "attempt_key_staged"} }
  }
  action TheAttemptArtifactIsPublished = {
    nondet artifact = 0.to(7).oneOf()
    val a = s.artifacts.get(artifact)
    all { a.state == "staged", not(a.object), not(a.due),
      s' = {...s, artifacts: s.artifacts.set(artifact, {...a, object: true}), event: "attempt_object_written"} }
  }
  action ThePrivacyPassCommits = {
    nondet worker = Workers.oneOf()
    nondet artifact = 0.to(7).oneOf()
    nondet privacy = Set("cleared", "escalated").oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val a = s.artifacts.get(artifact)
    all { current(w), r.phase == "review", r.privacy == "none", s.privacyControl, r.operable,
      a.state == "staged", a.object, not(a.due), a.runId == w.runId, a.token == w.token, a.kind == "privacy_pass",
      s' = {...s, runs: s.runs.set(w.runId, {...r, privacy: privacy, reviewAfterPass: false, submissionStatus: if (privacy == "escalated" and r.submissionStatus == "received") "quarantined" else r.submissionStatus}),
        artifacts: s.artifacts.set(artifact, {...a, state: "committed"}), event: "privacy_pass_recorded"} }
  }
  action TheWorkerParksForHumanReview = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.phase == "review", r.privacy != "none",
      (r.admission == "quarantine" and r.reviewAssessment == "none") or (r.privacy == "escalated" and not(r.reviewAfterPass)),
      s' = {...s, runs: s.runs.set(w.runId, {...r, state: "awaiting_review", live: false, attempt: r.attempt - 1, error: "assessment_required"}),
        workers: s.workers.set(worker, emptyWork), frame: "pipeline_review_hold", event: "review_parked_uncharged"} }
  }
  action ReviewerClaimsTheHold = {
    nondet id = Runs.oneOf()
    nondet reviewer = Reviewers.oneOf()
    val r = s.runs.get(id)
    nondet freshToken = 1.to(4).filter(token => token != r.reviewToken
      and not(Reviewers.exists(v => s.humanRequests.get(v).present and s.humanRequests.get(v).runId == id and s.humanRequests.get(v).token == token))).oneOf()
    all { controls(s.frame).contains("ReviewerClaimsTheHold"), reviewable(r), r.operable, true,
      not(r.reviewLive) or r.reviewOwner == reviewer,
      s' = {...s, runs: s.runs.set(id, {...r, reviewOwner: reviewer, reviewToken: freshToken, reviewLive: true}),
        humanRequests: s.humanRequests.set(reviewer, {runId: id, token: freshToken, present: true}),
        frame: "pipeline_review_hold", event: "human_claim_separate_from_worker"} }
  }
  action TheHumanReviewLeaseExpires = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { r.reviewLive, s' = {...s, runs: s.runs.set(id, {...r, reviewLive: false}), event: "human_claim_expired"} }
  }
  action ReviewerApprovesTheHold = {
    nondet reviewer = Reviewers.oneOf()
    val request = s.humanRequests.get(reviewer)
    val id = request.runId
    val r = s.runs.get(id)
    // This event represents the current matching human claim and all required reasons resolved.
    all { controls(s.frame).contains("ReviewerApprovesTheHold"), reviewable(r), r.operable, r.reviewLive, request.present, r.reviewOwner == reviewer, r.reviewToken == request.token,
      s' = {...s, runs: s.runs.set(id, {...r, reviewLive: false,
        reviewAssessment: if (r.phase == "review") "approve" else r.reviewAssessment,
        reviewAfterPass: if (r.phase == "review") r.privacy != "none" else r.reviewAfterPass,
        scoreAssessment: if (r.phase == "score") "approve" else r.scoreAssessment,
        state: if (r.state == "awaiting_review") "pending" else r.state, due: if (r.state == "awaiting_review") true else r.due}),
        humanRequests: s.humanRequests.set(reviewer, {...request, present: false}),
        frame: "pipeline_pending", event: "phase_bound_assessment_approved"} }
  }
  action ReviewerRejectsTheHold = {
    nondet reviewer = Reviewers.oneOf()
    val request = s.humanRequests.get(reviewer)
    val id = request.runId
    val r = s.runs.get(id)
    all { controls(s.frame).contains("ReviewerRejectsTheHold"), reviewable(r), r.operable, r.reviewLive, request.present, r.reviewOwner == reviewer, r.reviewToken == request.token,
      s' = {...s, runs: s.runs.set(id, {...r, reviewLive: false,
        reviewAssessment: if (r.phase == "review") "reject" else r.reviewAssessment,
        reviewAfterPass: if (r.phase == "review") r.privacy != "none" else r.reviewAfterPass,
        scoreAssessment: if (r.phase == "score") "reject" else r.scoreAssessment,
        state: if (r.state == "awaiting_review") "pending" else r.state, due: if (r.state == "awaiting_review") true else r.due}),
        humanRequests: s.humanRequests.set(reviewer, {...request, present: false}),
        frame: "pipeline_pending", event: "phase_bound_assessment_rejected"} }
  }
  action TheStaleHumanAssessmentIsRefused = {
    nondet reviewer = Reviewers.oneOf()
    val request = s.humanRequests.get(reviewer)
    val r = s.runs.get(request.runId)
    all { request.present, not(r.reviewLive) or r.reviewOwner != reviewer or r.reviewToken != request.token or not(reviewable(r)) or not(r.operable),
      s' = {...s, humanRequests: s.humanRequests.set(reviewer, {...request, present: false}), event: "stale_or_inoperable_human_claim_refused"} }
  }
  action TheReviewApprovesAndCommits = {
    nondet worker = Workers.oneOf()
    nondet artifact = 0.to(7).oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val a = s.artifacts.get(artifact)
    all { current(w), r.phase == "review", r.operable, s.runnable.get(policyCell(r)), r.privacy != "none",
      (r.admission == "admit" or r.reviewAssessment == "approve"),
      (r.privacy != "escalated" or (r.reviewAfterPass and r.reviewAssessment == "approve")),
      a.runId == w.runId, a.token == w.token, a.kind == "approved", a.state == "staged", a.object, not(a.due),
      s' = {...s, runs: s.runs.set(w.runId, {...r, phase: "score", state: "pending", attempt: 0, live: false, due: true, revision: true, submissionStatus: "accepted"}),
        artifacts: s.artifacts.set(artifact, {...a, state: "committed"}), workers: s.workers.set(worker, emptyWork),
        frame: "pipeline_pending", event: "approved_revision_committed"} }
  }
  action TheReviewRejectsAndCommits = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.phase == "review", r.operable, s.runnable.get(policyCell(r)), r.privacy != "none",
      r.reviewAssessment == "reject", (r.admission == "quarantine" or (r.privacy == "escalated" and r.reviewAfterPass)),
      s' = {...s, runs: s.runs.set(w.runId, {...r, phase: "none", state: "complete", live: false, submissionStatus: "rejected"}),
        workers: s.workers.set(worker, emptyWork), frame: "pipeline_terminal", event: "review_rejected"} }
  }
  action TheScorePolicyHoldsTheTrace = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.phase == "score", r.operable, s.runnable.get(policyCell(r)), r.scoreAssessment == "none",
      not(0.to(7).exists(id => { val a = s.artifacts.get(id)
        a.runId == w.runId and a.token == w.token and a.state == "staged" and a.object })),
      s' = {...s, runs: s.runs.set(w.runId, {...r, state: "awaiting_review", live: false, attempt: r.attempt - 1, error: "score_hold"}),
        artifacts: s.artifacts.keys().mapBy(id => { val a = s.artifacts.get(id)
          if (a.runId == w.runId and a.token == w.token and a.state == "staged") {...a, state: "deleted"} else a }),
        workers: s.workers.set(worker, emptyWork), frame: "pipeline_score_hold", event: "no_award_or_settlement_seeded"} }
  }
  action TheScoreCommits = {
    nondet worker = Workers.oneOf()
    nondet award = Set(false, true).oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.phase == "score", r.operable, s.runnable.get(policyCell(r)),
      not(0.to(7).exists(id => { val a = s.artifacts.get(id)
        a.runId == w.runId and a.token == w.token and a.state == "staged" and (not(a.object) or a.due) })),
      s' = {...s, runs: s.runs.set(w.runId, {...r, phase: "settle", state: "pending", attempt: 0, live: false, due: true,
        award: award and r.scoreAssessment != "reject"}),
        artifacts: s.artifacts.keys().mapBy(id => { val a = s.artifacts.get(id)
          if (a.runId == w.runId and a.token == w.token and a.state == "staged" and a.object) {...a, state: "committed"} else a }), workers: s.workers.set(worker, emptyWork),
        frame: "pipeline_pending", event: if (r.scoreAssessment == "reject") "server_zero_score_after_rejection" else "score_and_index_commands_persisted"} }
  }
  action TheSettleSelectionAndTerminalEffectsAreRecorded = {
    nondet worker = Workers.oneOf()
    nondet paid = Set(false, true).oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), r.phase == "settle", s.runnable.get(policyCell(r)), not(r.effectsTerminal),
      s' = {...s, runs: s.runs.set(w.runId, {...r, selection: true, effectsTerminal: true, paid: paid and r.award and r.operable}),
        frame: "pipeline_settle", event: "coarse_settlement_progress_boundary"} }
  }
  action TheSettleCommitsFromProgress = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), finishable(r), s.runnable.get(policyCell(r)),
      s' = {...s, runs: s.runs.set(w.runId, {...r, state: "complete", phase: "none", live: false}),
        workers: s.workers.set(worker, emptyWork), frame: "pipeline_terminal", event: "settle_complete"} }
  }
  action TheDependencyOrPolicySuspensionReleasesTheAttempt = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), not(s.dependency) or not(s.runnable.get(policyCell(r))) or (r.phase == "review" and not(s.privacyControl)),
      s' = {...s, runs: s.runs.set(w.runId, {...r, state: "retry", live: false, attempt: r.attempt - 1, due: false, error: "uncharged_suspension"}),
        workers: s.workers.set(worker, emptyWork), frame: "pipeline_retry", event: "phase_age_backoff_without_terminal_budget"} }
  }
  action TheTransientOutageOrScoreLockBusyReleasesTheAttempt = {
    nondet worker = Workers.oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    all { current(w), s' = {...s, runs: s.runs.set(w.runId, {...r, state: "retry", live: false, attempt: r.attempt - 1, due: false, error: "transient_or_score_lock_busy"}),
      workers: s.workers.set(worker, emptyWork), frame: "pipeline_retry", event: "uncharged_operational_retry"} }
  }
  action TheOperationalFailureChargesTheAttempt = {
    nondet worker = Workers.oneOf()
    nondet label = Set("operational", "artifact_integrity", "privacy_classification_failed").oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val exhausted = r.attempt >= 2
    val grant = exhausted and finishable(r) and not(r.finishGranted)
    val rehold = exhausted and finishable(r) and r.finishGranted and r.heldBefore and r.paid
    all { current(w), label != "privacy_classification_failed" or (r.phase == "review" and r.privacy == "none"), s' = {...s, runs: s.runs.set(w.runId, {...r,
      state: if (grant) "retry" else if (rehold) "awaiting_operator" else if (exhausted) "failed" else "retry",
      live: false, due: grant, attempt: if (grant) 1 else r.attempt, finishGranted: r.finishGranted or grant,
      submissionStatus: if (exhausted and not(grant) and not(rehold) and label != "privacy_classification_failed" and r.submissionStatus == "received") "rejected" else r.submissionStatus,
      error: if (exhausted and not(grant) and label != "privacy_classification_failed") "attempts_exhausted" else label}),
      workers: s.workers.set(worker, emptyWork), frame: if (rehold) "pipeline_operator_hold" else if (exhausted and not(grant)) "pipeline_terminal" else "pipeline_retry",
      event: if (grant) "one_extra_commit_attempt" else "charged_failure"} }
  }
  action TheDeterministicRefusalEndsOrHoldsTheRun = {
    nondet worker = Workers.oneOf()
    nondet gateRefusal = Set(false, true).oneOf()
    val w = s.workers.get(worker)
    val r = s.runs.get(w.runId)
    val hold = finishable(r) and r.paid and (gateRefusal or r.heldBefore)
    all { current(w), (gateRefusal and r.phase == "settle") or (not(gateRefusal) and ((not(r.operable) and r.phase != "settle") or (r.phase == "score" and r.scoreAssessment == "approve"))),
      s' = {...s, runs: s.runs.set(w.runId, {...r, state: if (hold) "awaiting_operator" else "failed", live: false,
        heldBefore: r.heldBefore or hold, submissionStatus: if (not(hold) and r.submissionStatus == "received") "rejected" else r.submissionStatus, error: if (gateRefusal) "gate_decision_conflict_or_incomplete" else if (not(r.operable)) "submission_inoperable" else "score_hold_repeated"}),
        workers: s.workers.set(worker, emptyWork), frame: if (hold) "pipeline_operator_hold" else "pipeline_terminal", event: "deterministic_refusal"} }
  }
  action TheExpiredExhaustedLeaseIsSwept = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    val grant = finishable(r) and not(r.finishGranted)
    val rehold = finishable(r) and r.finishGranted and r.heldBefore and r.paid
    all { r.state == "leased", not(r.live), r.attempt >= 2,
      s' = {...s, runs: s.runs.set(id, {...r, state: if (grant) "retry" else if (rehold) "awaiting_operator" else "failed",
        attempt: if (grant) 1 else r.attempt, due: grant, finishGranted: r.finishGranted or grant,
        submissionStatus: if (not(grant) and not(rehold) and r.submissionStatus == "received") "rejected" else r.submissionStatus, error: "expired_exhausted_lease"}), frame: if (rehold) "pipeline_operator_hold" else if (grant) "pipeline_retry" else "pipeline_terminal", event: "claim_sweep"} }
  }
  action TheRetryBecomesDue = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { r.state == "retry", not(r.due), s' = {...s, runs: s.runs.set(id, {...r, due: true}), event: "abstract_backoff_elapsed"} }
  }
  action TheSubmissionBecomesInoperable = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { r.state != "absent", r.operable, s' = {...s, runs: s.runs.set(id, {...r, operable: false}), event: "withdrawal_revocation_expiry_or_purge"} }
  }
  action TheInoperableHumanHoldIsReleased = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { r.state == "awaiting_review", not(r.operable),
      s' = {...s, runs: s.runs.set(id, {...r, state: "pending", due: true, reviewLive: false}), frame: "pipeline_pending", event: "worker_can_end_inoperable_hold"} }
  }
  action TheAttemptArtifactBecomesDue = {
    nondet artifact = 0.to(7).oneOf()
    val a = s.artifacts.get(artifact)
    all { a.state == "staged", not(a.due), not(s.runs.get(a.runId).live and s.runs.get(a.runId).token == a.token),
      s' = {...s, artifacts: s.artifacts.set(artifact, {...a, due: true}), event: "lease_cap_plus_commit_margin_elapsed"} }
  }
  action TheOrphanArtifactIsSwept = {
    nondet artifact = 0.to(7).oneOf()
    val a = s.artifacts.get(artifact)
    all { a.state == "staged", a.due, s' = {...s, artifacts: s.artifacts.set(artifact, {...a, state: "deleted", object: false}), event: "orphan_attempt_object_removed"} }
  }
  action OperatorSuspendsOrResumesThePolicy = {
    nondet cell = 0.to(11).oneOf()
    all { controls(s.frame).contains("OperatorSuspendsOrResumesThePolicy"), s' = {...s, runnable: s.runnable.set(cell, not(s.runnable.get(cell))), event: "bound_phase_policy_intervention"} }
  }
  action OperatorReleasesThePaidRun = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { controls(s.frame).contains("OperatorReleasesThePaidRun"), r.state == "awaiting_operator",
      s' = {...s, runs: s.runs.set(id, {...r, state: "retry", attempt: 1, due: true, finishGranted: true}), frame: "pipeline_retry", event: "operator_extra_commit_attempt"} }
  }
  action OperatorFailsThePaidRun = {
    nondet id = Runs.oneOf()
    val r = s.runs.get(id)
    all { controls(s.frame).contains("OperatorFailsThePaidRun"), r.state == "awaiting_operator",
      s' = {...s, runs: s.runs.set(id, {...r, state: "failed"}), frame: "pipeline_terminal", event: "operator_failed_paid_run_index_cleanup_owned_elsewhere"} }
  }
  action TheRuntimeDependencyOrPrivacyControlChanges = {
    nondet dependency = Set(false, true).oneOf()
    nondet privacyControl = Set(false, true).oneOf()
    s' = {...s, dependency: dependency, privacyControl: privacyControl}
  }
  action step = any { TheReceiptCommitsItsAdmission, TheWorkerClaimsOrReclaimsTheRun, TheWorkerLeaseExpires,
    TheWorkerRenewsWithinItsCap, TheStaleWorkerReturns, TheWorkerStagesAnAttemptArtifact, TheAttemptArtifactIsPublished,
    ThePrivacyPassCommits, TheWorkerParksForHumanReview, ReviewerClaimsTheHold, TheHumanReviewLeaseExpires,
    ReviewerApprovesTheHold, ReviewerRejectsTheHold, TheStaleHumanAssessmentIsRefused, TheReviewApprovesAndCommits, TheReviewRejectsAndCommits,
    TheScorePolicyHoldsTheTrace, TheScoreCommits, TheSettleSelectionAndTerminalEffectsAreRecorded,
    TheSettleCommitsFromProgress, TheDependencyOrPolicySuspensionReleasesTheAttempt,
    TheTransientOutageOrScoreLockBusyReleasesTheAttempt, TheOperationalFailureChargesTheAttempt,
    TheDeterministicRefusalEndsOrHoldsTheRun, TheExpiredExhaustedLeaseIsSwept, TheRetryBecomesDue,
    TheSubmissionBecomesInoperable, TheInoperableHumanHoldIsReleased, TheAttemptArtifactBecomesDue,
    TheOrphanArtifactIsSwept, OperatorSuspendsOrResumesThePolicy, OperatorReleasesThePaidRun,
    OperatorFailsThePaidRun, TheRuntimeDependencyOrPrivacyControlChanges }
}
