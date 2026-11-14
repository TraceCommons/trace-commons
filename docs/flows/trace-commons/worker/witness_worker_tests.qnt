module witness_worker_tests {
  import witness_worker.* from "./witness_worker"

  action review(i: int): bool = enqueue(i).then(dispatch(i)).then(begin(i))
  action failTrace(i: int, wait: int): bool = review(i).then(transient(i, wait))
  // Dispatch at the current time, run for `seconds`, then fail with an outage label.
  action slowFail(i: int, seconds: int, wait: int): bool = review(i).then(tick(seconds)).then(transient(i, wait))
  action redact(i: int, wait: int): bool = review(i).then(redactionFailed(i, wait))
  action trip = failTrace(0, 30).then(failTrace(1, 30)).then(failTrace(2, 30))
  action warm = review(3).then(success(3)).then(cancel(3)).then(approve(3, false))
  action twoRunning = warm.then(enqueue(0)).then(enqueue(1)).then(dispatch(0)).then(begin(0))
    .then(dispatch(1)).then(begin(1))
  action require(condition: bool): bool = all {assert(condition), s' = s, checked' = checked}

  run isolatedFailureKeepsHealthyWorkRunnableTest = init.then(enqueue(0)).then(enqueue(1))
    .then(dispatch(0)).then(begin(0)).then(transient(0, 30))
    .expect(safety and s.entries.get(0).state == "approved" and s.entries.get(0).due == 30 and
      s.hold < 0 and laneReady(s) and best(s, 1))

  run repeatedSameTraceNeverTripsCircuitTest = init.then(failTrace(0, 30)).then(tick(30))
    .then(failTrace(0, 60)).then(tick(60)).then(failTrace(0, 150))
    .expect(safety and s.circuit.reopen < 0 and Ids.filter(i => s.circuit.seen.get(i) >= 0).size() == 1)

  run threeDistinctFailuresPauseThenOneProbeRecoversTest = init.then(trip)
    .then(enqueue(3)).then(require(not(laneReady(s)) and s.circuit.reopen == 30))
    .then(tick(30)).then(dispatch(3)).then(begin(3))
    .then(require(not(laneReady(s)) and s.circuit.probe == 3))
    .then(success(3)).then(enqueue(0)).then(enqueue(1)).then(dispatch(0)).then(dispatch(1))
    .expect(safety and s.answering and running(s).size() == 2 and s.circuit.reopen < 0)

  // Chain rule: the gap runs from the latest failure's end to the next failing dispatch.
  // Each request here lasts 100 s, so end-to-end distances exceed 300 s in both tests.
  run quietGapOfJustOverFiveMinutesStartsNewRunTest = init.then(slowFail(0, 100, 30)).then(tick(301))
    .then(slowFail(1, 100, 30)).then(slowFail(2, 100, 30))
    .expect(safety and s.circuit.reopen < 0 and Ids.filter(i => s.circuit.seen.get(i) >= 0) == Set(1, 2))

  run quietGapOfExactlyFiveMinutesKeepsRunTest = init.then(slowFail(0, 100, 30)).then(tick(300))
    .then(slowFail(1, 100, 30)).then(slowFail(2, 100, 30)).expect(safety and s.circuit.reopen == 630)

  // A hung witness: every request times out after 570 s and the next is dispatched at once.
  run backToBackTimeoutsLongerThanTheGapOpenTheCircuitTest = init.then(slowFail(0, 570, 30))
    .then(slowFail(1, 570, 30)).then(require(s.circuit.reopen < 0)).then(slowFail(2, 570, 30))
    .expect(safety and s.circuit.reopen == 1740 and s.circuit.delay == 30 and not(laneReady(s)))

  run quietGapAfterLongTimeoutsStartsNewRunTest = init.then(slowFail(0, 570, 30)).then(slowFail(1, 570, 30))
    .then(tick(301)).then(slowFail(2, 59, 30))
    .then(require(s.circuit.reopen < 0 and Ids.filter(i => s.circuit.seen.get(i) >= 0) == Set(2)))
    .then(slowFail(3, 10, 30)).then(require(s.circuit.reopen < 0))
    .then(slowFail(0, 10, 60)).expect(safety and s.circuit.reopen == 1550)

  run successBetweenLongTimeoutsEndsTheRunTest = init.then(slowFail(0, 570, 30)).then(slowFail(1, 570, 30))
    .then(review(3)).then(success(3)).then(slowFail(2, 570, 30))
    .expect(safety and s.circuit.reopen < 0 and Ids.filter(i => s.circuit.seen.get(i) >= 0) == Set(2))

  // witness_redaction_failed retries per trace but never feeds the shared circuit.
  run redactionFailedOutageOfThreeDistinctTracesDoesNotOpenCircuitTest = init.then(redact(0, 30))
    .then(redact(1, 30)).then(redact(2, 30))
    .expect(safety and s.circuit == emptyCircuit and laneReady(s) and
      Ids.filter(i => s.entries.get(i).retrying and s.entries.get(i).failures == 1 and
        s.entries.get(i).attempts == 1 and s.entries.get(i).due == 30) == Set(0, 1, 2))

  run redactionFailureNeitherCountsNorBreaksAnOutageRunTest = init.then(failTrace(0, 30)).then(failTrace(1, 30))
    .then(redact(2, 30)).then(require(s.circuit.reopen < 0)).then(failTrace(3, 30))
    .expect(safety and s.circuit.reopen == 30 and s.circuit.seen.get(2) < 0)

  run redactionFailedProbeReleasesSlotWithoutExtendingPauseTest = init.then(trip).then(tick(30))
    .then(review(3)).then(redactionFailed(3, 30))
    .expect(safety and s.circuit.probe < 0 and s.circuit.reopen == 30 and s.circuit.delay == 30 and
      not(s.answering) and laneReady(s))

  run redactionFailureClosesSecondSlotTest = init.then(twoRunning).then(redactionFailed(0, 30))
    .expect(safety and not(s.answering) and running(s) == Set(1) and not(laneReady(s)))

  run differentProbeOutranksManualRetryOfLastFailureTest = init.then(trip).then(tick(30))
    .then(cancel(2)).then(approve(2, true)).then(enqueue(2)).then(enqueue(0)).then(dispatch(0))
    .expect(safety and s.circuit.probe == 0 and waiting(s).contains(2))

  run repeatedFailedProbesCapSharedPauseAtFiveMinutesTest = init.then(trip)
    .then(tick(30)).then(failTrace(3, 30)).then(require(s.circuit.delay == 60))
    .then(tick(60)).then(failTrace(0, 60)).then(require(s.circuit.delay == 120))
    .then(tick(120)).then(failTrace(1, 60)).then(require(s.circuit.delay == 240))
    .then(tick(240)).then(failTrace(2, 60)).then(require(s.circuit.delay == 300))
    .then(tick(300)).then(failTrace(3, 60)).expect(safety and s.circuit.delay == 300)

  run droppedProbeReleasesOnlyItsSlotTest = init.then(trip).then(tick(30))
    .then(enqueue(3)).then(dispatch(3)).then(drop(3))
    .expect(safety and s.circuit.probe < 0 and s.circuit.reopen == 30 and not(s.answering))

  run saturationKeepsApprovalAndDefersQueuedHintsWithoutAttemptsTest = init.then(enqueue(0))
    .then(enqueue(1)).then(dispatch(0)).then(begin(0)).then(busy(0, 45))
    .expect(safety and waiting(s).size() == 0 and s.hold == 45 and
      s.entries.get(0).attempts == 1 and s.entries.get(1).attempts == 0 and
      s.entries.get(0).state == "approved" and s.entries.get(1).state == "approved" and
      s.entries.get(1).due == 46)

  run lateSuccessBeforeSaturationExpiryDoesNotReopenCapacityTest = init.then(twoRunning)
    .then(busy(0, 120)).then(success(1))
    .expect(safety and not(s.answering) and not(laneReady(s)) and s.hold == 120)

  run lateSuccessAfterSaturationExpiryStillNeedsFreshProbeTest = init.then(twoRunning)
    .then(busy(0, 120)).then(tick(121)).then(success(1))
    .then(require(not(s.answering) and s.hold == 120))
    .then(review(3)).then(require(not(laneReady(s)))).then(success(3))
    .expect(safety and s.answering and s.hold < 0)

  run queuedManualReapprovalRefreshesIdentityAndPriorityTest = init.then(enqueue(0)).then(enqueue(1))
    .then(cancel(1)).then(approve(1, true)).then(enqueue(1)).then(dispatch(1)).then(begin(1))
    .expect(safety and s.running.get(1).key == s.entries.get(1).key and s.running.get(1).key.manual)

  run lateManualApprovalDisplacesWorstHintWithoutLosingApprovalTest = init.then(enqueue(0))
    .then(enqueue(1)).then(enqueue(2)).then(cancel(3)).then(approve(3, true)).then(enqueue(3))
    .expect(safety and waiting(s) == Set(0, 1, 3) and s.entries.get(2).state == "approved" and best(s, 3))

  run freshAutomaticWorkPrecedesQueuedRetriesTest = init.then(failTrace(0, 30)).then(tick(30))
    .then(enqueue(0)).then(enqueue(1)).expect(safety and best(s, 1))

  // Trace 1 failed once, then was cancelled and reapproved: its attempts remain but
  // its deadline is cleared, so it is fresh work and precedes trace 2 by source age.
  run reapprovedTraceWithEarlierAttemptsSortsAsFreshWorkTest = init.then(failTrace(1, 30))
    .then(cancel(1)).then(approve(1, false)).then(failTrace(0, 30)).then(tick(30))
    .then(enqueue(0)).then(enqueue(1)).then(enqueue(2))
    .expect(safety and s.entries.get(1).attempts == 1 and s.entries.get(1).due == 0 and best(s, 1))

  run olderRetryDeadlinePrecedesSourceAgeTest = init.then(failTrace(0, 60)).then(failTrace(1, 30))
    .then(tick(60)).then(enqueue(0)).then(enqueue(1)).expect(safety and best(s, 1))

  run restartPreservesEffectiveEntryDeadlineTest = init.then(failTrace(0, 60)).then(restart)
    .then(tick(59)).then(require(not(ready(s.entries.get(0), s.now))))
    .then(tick(1)).then(review(0)).expect(safety and s.running.get(0).started)

  run restartPreservesCertifiedPinTest = init.then(review(0)).then(success(0)).then(restart)
    .then(upload(0)).expect(safety and s.entries.get(0).state == "uploaded")

  run pauseAfterRequestAllowsPinButHoldsUploadTest = init.then(review(0)).then(pause(true))
    .then(success(0)).then(require(s.entries.get(0).pinned and s.paused))
    .then(pause(false)).then(upload(0)).expect(safety)

  run unenrollmentDiscardsInFlightResultTest = init.then(review(0)).then(enrollment(false)).then(success(0))
    .expect(safety and not(s.entries.get(0).pinned) and running(s).size() == 0)

  run manualReapprovalRejectsOldAutomaticCompletionTest = init.then(review(0)).then(cancel(0))
    .then(approve(0, true)).then(success(0)).expect(safety and not(s.entries.get(0).pinned))

  run changedTermsRejectOldCompletionTest = init.then(review(0)).then(termsChanged(0))
    .then(approve(0, false)).then(success(0)).expect(safety and not(s.entries.get(0).pinned))

  // Deliberate source behavior: JobKey is equality of terms, not an epoch.
  run identicalAutomaticReapprovalCanAcceptMatchingOldCompletionTest = init.then(review(0)).then(cancel(0))
    .then(approve(0, false)).then(success(0)).expect(safety and s.entries.get(0).pinned)

  // Known defect retained as a counterexample to an unconditional deadline claim.
  run freshWatchdogDeadlineIsNotEffectiveWithoutRetryLabelTest = init.then(review(0)).then(watchdog(0))
    .expect(safety and s.entries.get(0).due > s.now and ready(s.entries.get(0), s.now))

  run watchdogDeadlineIsEffectiveOnAnAlreadyRetryingTraceTest = init.then(failTrace(0, 30)).then(tick(30))
    .then(review(0)).then(watchdog(0)).expect(safety and not(ready(s.entries.get(0), s.now)))

  run trustRefusalRequiresNewApprovalTest = init.then(review(0)).then(permanent(0, true))
    .expect(safety and s.entries.get(0).state == "pending" and s.entries.get(0).held and
      not(canApprove(s, 0, false)) and not(s.entries.get(0).pinned))

  run manualApprovalCanReleaseTrustHoldTest = init.then(review(0)).then(permanent(0, true))
    .then(approve(0, true)).then(review(0)).then(success(0))
    .expect(safety and s.entries.get(0).pinned and s.entries.get(0).key.manual and not(s.entries.get(0).held))

  run successfulReviewClearsWitnessRetryRunTest = init.then(failTrace(0, 30)).then(tick(30))
    .then(review(0)).then(success(0)).expect(safety and s.entries.get(0).failures == 0 and
      not(s.entries.get(0).retrying) and s.entries.get(0).due == 0 and s.entries.get(0).attempts == 1)

  run permanentRefusalDoesNotBecomeTransientTest = init.then(review(0)).then(permanent(0, false))
    .expect(safety and s.entries.get(0).state == "refused" and not(s.entries.get(0).retrying))

  run retryScheduleMatchesProductionTest = init.expect(nominal(1) == 60 and nominal(2) == 120 and
    nominal(3) == 300 and nominal(4) == 900 and nominal(5) == 3600 and nominal(100) == 3600)

  // Faults below are excluded from step; each must trip the independent observers.
  action legacyGlobalFailure(i: int): bool = {
    val e = s.entries.get(i)
    commit({...s, entries: s.entries.set(i, {...e, retrying: true, due: s.now + 60}),
      running: s.running.set(i, emptyJob), queued: Ids.mapBy(_ => emptyJob), hold: s.now + 60}, "transient", i)
  }
  run negativeLegacyGlobalFailureIsDetectedTest = init.then(enqueue(0)).then(enqueue(1))
    .then(dispatch(0)).then(begin(0)).then(legacyGlobalFailure(0)).expect(not(safety) and not(checked.isolated))

  action legacyLateSuccess(i: int): bool = commit({...s,
    entries: s.entries.set(i, {...s.entries.get(i), pinned: true, pinKey: s.running.get(i).key}),
    running: s.running.set(i, emptyJob), answering: s.hold <= s.now}, "success", i)
  run negativeLateSuccessAfterExpiryIsDetectedTest = init.then(twoRunning).then(busy(0, 120))
    .then(tick(121)).then(legacyLateSuccess(1)).expect(not(safety) and not(checked.capacity))

  action staleEnqueue(i: int): bool = commit(s, "enqueue", i)
  run negativeObsoleteQueuedApprovalIsDetectedTest = init.then(enqueue(0)).then(cancel(0))
    .then(approve(0, true)).then(staleEnqueue(0)).expect(not(safety) and not(checked.refreshed))

  run negativePriorityBypassIsDetectedTest = init.then(enqueue(0)).then(cancel(1)).then(approve(1, true))
    .then(enqueue(1)).then(dispatchUnchecked(0)).expect(not(safety) and not(checked.dispatch))

  action automaticApprovalBypassingTrustHold(i: int): bool = commit({...s,
    entries: s.entries.set(i, {...s.entries.get(i), state: "approved", held: false, key: emptyKey})}, "approve", i)
  run negativeAutomaticTrustHoldBypassIsDetectedTest = init.then(review(0)).then(permanent(0, true))
    .then(automaticApprovalBypassingTrustHold(0)).expect(not(safety) and not(checked.approval))

  action forgedPin(i: int): bool = commit({...s,
    entries: s.entries.set(i, {...s.entries.get(i), pinned: true, pinKey: s.entries.get(i).key})}, "success", i)
  run negativeUncertifiedPinIsDetectedTest = init.then(forgedPin(0)).expect(not(safety) and not(checked.pin))
}
