// Atomic database admission AFTER independent authenticated wallet verification.
// Distinct valid ceremonies/proofs and weak sessions for one account are inputs.
module identity_bootstrap {
  val Requests = Set(0, 1)
  type Request = {phase: str, strong: bool}
  var s: {identities: Set[int], requests: int -> Request, lock: int, violation: bool}
  val vars = s
  action initialize(strongSecond: bool): bool = s' = {identities: Set(),
    requests: Map(0 -> {phase: "verified", strong: false},
      1 -> {phase: "verified", strong: strongSecond}), lock: -1, violation: false}
  action init = initialize(false)
  action strongInit = initialize(true)
  action acquire(i: int): bool = all {
    s.requests.get(i).phase == "verified", s.lock == -1,
    s' = {...s, lock: i, requests: s.requests.set(i, {...s.requests.get(i), phase: "locked"})}
  }
  // Fresh READ COMMITTED snapshot after the FOR UPDATE lock wait. Count and
  // insert commit in one transaction; no other writer can cross this lock.
  action commit(i: int): bool = {
    val r = s.requests.get(i)
    val allowed = r.strong or s.identities.size() == 0
    all { s.lock == i, r.phase == "locked",
      s' = {...s, lock: -1,
        identities: if (allowed) s.identities.union(Set(i)) else s.identities,
        requests: s.requests.set(i, {...r, phase: if (allowed) "committed" else "denied"}),
        violation: s.violation or (allowed and not(r.strong) and s.identities.size() > 0)} }
  }
  // Immediate account FKs also conflict with the bootstrap account lock.
  action trustedStrongWriter = all { s.lock == -1, not(s.identities.contains(2)),
    s' = {...s, identities: s.identities.union(Set(2))} }
  // Completed finite requests leave the backend idle, not deadlocked.
  action quiescent = all { s.lock == -1, s.identities.contains(2),
    Requests.forall(i => Set("committed", "denied").contains(s.requests.get(i).phase)), s' = s }
  action step = {
    nondet i = Requests.oneOf()
    any {acquire(i), commit(i), trustedStrongWriter, quiescent}
  }
  val FirstOnly = not(s.violation)
  run concurrentWeakSecondDeniedTest = init.then(acquire(0)).then(commit(0))
    .then(acquire(1)).then(commit(1))
    .expect(FirstOnly and s.identities == Set(0) and s.requests.get(1).phase == "denied")
  run legitimateStrongSecondAllowedTest = strongInit.then(acquire(0)).then(commit(0))
    .then(acquire(1)).then(commit(1)).expect(FirstOnly and s.identities == Requests)
  run provisioningWriterObservedTest = init.then(trustedStrongWriter)
    .then(acquire(0)).then(commit(0))
    .expect(FirstOnly and s.identities == Set(2) and s.requests.get(0).phase == "denied")
}
