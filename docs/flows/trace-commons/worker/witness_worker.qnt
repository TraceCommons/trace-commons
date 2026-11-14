// Background-worker projection of the witness-circuit-followups working tree on 453522e60.
// See README.md for bounds/omissions.
module witness_worker {
  pure val Ids = Set(0, 1, 2, 3)
  pure val QueueBound = 3 // production MAX_QUEUED=64; fourth identity exercises displacement
  type Key = {revision: int, manual: bool, approvedAt: int}
  type Entry = {state: str, held: bool, revision: int, key: Key, attempts: int, failures: int,
    retrying: bool, due: int, pinned: bool, pinKey: Key}
  type Job = {present: bool, key: Key, attempts: int, retrying: bool, due: int,
    order: int, started: bool, dispatched: int}
  type Circuit = {seen: int -> int, reopen: int, probe: int, delay: int, last: int}
  type State = {now: int, entries: int -> Entry, queued: int -> Job, running: int -> Job,
    serial: int, circuit: Circuit, hold: int, answering: bool, paused: bool, enrolled: bool}
  type Checks = {dispatch: bool, request: bool, pin: bool, upload: bool, approval: bool,
    isolated: bool, late: bool, capacity: bool, refreshed: bool}
  var s: State
  var checked: Checks
  pure val emptyKey: Key = {revision: 0, manual: false, approvedAt: -1}
  pure val emptyJob: Job = {present: false, key: emptyKey, attempts: 0, retrying: false,
    due: 0, order: 0, started: false, dispatched: 0}
  pure val emptyCircuit: Circuit = {seen: Ids.mapBy(_ => -1), reopen: -1, probe: -1, delay: 0, last: -1}
  pure val initialEntry: Entry = {state: "approved", held: false, revision: 0, key: emptyKey, attempts: 0, failures: 0,
    retrying: false, due: 0, pinned: false, pinKey: emptyKey}
  pure def waiting(t: State): Set[int] = Ids.filter(i => t.queued.get(i).present)
  pure def running(t: State): Set[int] = Ids.filter(i => t.running.get(i).present)
  pure def ready(e: Entry, now: int): bool = e.state == "approved" and (not(e.retrying) or e.due <= now)
  pure def current(t: State, i: int): bool = t.entries.get(i).state == "approved" and
    not(t.entries.get(i).pinned) and t.entries.get(i).key == t.running.get(i).key
  pure def snapshot(e: Entry, order: int): Job = {present: true, key: e.key, attempts: e.attempts,
    retrying: e.retrying, due: e.due, order: order, started: false, dispatched: 0}
  // A retry is an attempted entry still holding a deadline (`due` 0 is unset);
  // attempts never reset, so a reapproved or pinned trace sorts as fresh work.
  pure def isRetry(j: Job): bool = j.attempts > 0 and j.due > 0
  // Manual ties retain queue order. Automatic source age/UUID are represented by ID.
  pure def less(a: Job, ai: int, b: Job, bi: int): bool =
    if (a.key.manual != b.key.manual) a.key.manual
    else if (a.key.manual) false
    else if (isRetry(a) != isRetry(b)) not(isRetry(a))
    else if (isRetry(a) and a.due != b.due) a.due < b.due
    else ai < bi
  pure def preferred(t: State, a: int, b: int): bool = {
    val aj = t.queued.get(a)
    val bj = t.queued.get(b)
    if (t.circuit.reopen >= 0 and ((a == t.circuit.last) != (b == t.circuit.last))) a != t.circuit.last
    else less(aj, a, bj, b) or
      (not(less(bj, b, aj, a)) and aj.order < bj.order)
  }
  pure def eligible(t: State): Set[int] = waiting(t).filter(i => {
    val j = t.queued.get(i)
    not(j.retrying) or j.due <= t.now
  })
  pure def laneReady(t: State): bool = running(t).size() <
    (if (t.answering and t.circuit.reopen < 0) 2 else 1) and
    t.hold <= t.now and t.circuit.reopen <= t.now and t.circuit.probe < 0
  pure def best(t: State, i: int): bool = eligible(t).contains(i) and
    eligible(t).forall(j => not(preferred(t, j, i)))
  pure def nominal(failures: int): int = if (failures <= 1) 60 else if (failures == 2) 120
    else if (failures == 3) 300 else if (failures == 4) 900 else 3600
  // `started` is the failing request's dispatch time, `now` its end. A run resets when
  // the request was dispatched more than 300 s after the latest recorded failure ended.
  pure def failed(c: Circuit, i: int, started: int, now: int): Circuit = {
    val quiet = Ids.exists(j => c.seen.get(j) >= 0) and
      Ids.forall(j => c.seen.get(j) < 0 or started - c.seen.get(j) > 300)
    val recent = Ids.mapBy(j => if (j == i) now else if (quiet) -1 else c.seen.get(j))
    if (c.reopen >= 0) {
      val delay = if (c.delay * 2 > 300) 300 else c.delay * 2
      if (c.probe != i) c else {...c, reopen: now + delay, probe: -1, delay: delay, last: i}
    } else if (Ids.filter(j => recent.get(j) >= 0).size() >= 3)
      {seen: recent, reopen: now + 30, probe: -1, delay: 30, last: i}
    else {...c, seen: recent}
  }
  pure def cancelled(c: Circuit, i: int): Circuit = if (c.probe == i) {...c, probe: -1} else c
  pure def capacityDue(t: State, i: int, hint: int): int = t.now + hint + (i % (hint / 2 + 1))

  // Observers accumulate facts about transitions, not guards restricting the model.
  pure def observe(a: State, b: State, old: Checks, event: str, i: int): Checks = {
    val matching = current(a, i) and a.enrolled
    val firstPin = not(a.entries.get(i).pinned) and b.entries.get(i).pinned
    val late = a.circuit.reopen >= 0 and a.circuit.probe != i
    {dispatch: old.dispatch and (event != "dispatch" or (laneReady(a) and best(a, i))),
     request: old.request and (event != "begin" or
       (matching and ready(a.entries.get(i), a.now) and not(a.paused))),
     pin: old.pin and (not(firstPin) or (event == "success" and matching and a.running.get(i).started)),
     upload: old.upload and (event != "upload" or (a.enrolled and not(a.paused) and
       a.entries.get(i).pinned and a.entries.get(i).pinKey == a.entries.get(i).key)),
     approval: old.approval and (not(Set("transient", "redaction", "busy").contains(event) and matching) or
       (b.entries.get(i).state == "approved" and b.entries.get(i).key == a.entries.get(i).key)) and
       (event != "enqueue" or a.entries == b.entries) and
       (not(event == "approve" and a.entries.get(i).held) or b.entries.get(i).key.manual),
     isolated: old.isolated and (not(Set("transient", "redaction").contains(event) and matching and
       a.circuit.reopen < 0 and b.circuit.reopen < 0) or (a.hold == b.hold and a.queued == b.queued)),
     late: old.late and (not(Set("success", "transient", "redaction").contains(event) and matching and late) or
       a.circuit == b.circuit),
     capacity: old.capacity and (not(event == "success" and a.hold >= 0) or not(b.answering)),
     refreshed: old.refreshed and (not(event == "enqueue" and waiting(a).contains(i)) or
       b.queued.get(i).key == a.entries.get(i).key)}
  }
  action commit(after: State, event: str, i: int): bool = all {
    s' = after, checked' = observe(s, after, checked, event, i)
  }
  action init = all {
    s' = {now: 0, entries: Ids.mapBy(_ => initialEntry), queued: Ids.mapBy(_ => emptyJob),
      running: Ids.mapBy(_ => emptyJob), serial: 0, circuit: emptyCircuit, hold: -1,
      answering: false, paused: false, enrolled: true},
    checked' = {dispatch: true, request: true, pin: true, upload: true, approval: true,
      isolated: true, late: true, capacity: true, refreshed: true}
  }
  // Producer gates; queued duplicate refresh and full-queue displacement.
  action enqueue(i: int): bool = {
    val e = s.entries.get(i)
    val incoming = snapshot(e, s.serial)
    all {s.enrolled, not(s.paused), ready(e, s.now), not(e.pinned), not(running(s).contains(i)),
      if (waiting(s).contains(i)) commit({...s, queued: s.queued.set(i,
        snapshot(e, s.queued.get(i).order))}, "enqueue", i)
      else if (waiting(s).size() < QueueBound) commit({...s, queued: s.queued.set(i, incoming), serial: s.serial + 1}, "enqueue", i)
      else {
        nondet victim = waiting(s).oneOf()
        val v = s.queued.get(victim)
        all {waiting(s).forall(j => not(less(v, victim, s.queued.get(j), j)) and
          (less(s.queued.get(j), j, v, victim) or v.order >= s.queued.get(j).order)),
          less(incoming, i, v, victim),
          commit({...s, queued: s.queued.set(victim, emptyJob).set(i, incoming), serial: s.serial + 1}, "enqueue", i)}
      }
    }
  }
  // Unsafe helper exists for negative tests; step calls only guarded dispatch.
  action dispatchUnchecked(i: int): bool = commit({...s, hold: -1,
    circuit: if (s.circuit.reopen >= 0) {...s.circuit, probe: i} else s.circuit,
    running: s.running.set(i, {...s.queued.get(i), dispatched: s.now}), queued: s.queued.set(i, emptyJob)}, "dispatch", i)
  action dispatch(i: int): bool = all {laneReady(s), best(s, i), dispatchUnchecked(i)}
  action begin(i: int): bool = all {running(s).contains(i), not(s.running.get(i).started),
    s.enrolled, not(s.paused), current(s, i), ready(s.entries.get(i), s.now),
    commit({...s, running: s.running.set(i, {...s.running.get(i), started: true})}, "begin", i)}
  action drop(i: int): bool = all {running(s).contains(i),
    commit({...s, running: s.running.set(i, emptyJob), circuit: cancelled(s.circuit, i)}, "drop", i)}
  // Pin persistence failure, cancellation and stale approvals finish as Dropped.
  action success(i: int): bool = {
    val closes = s.circuit.reopen < 0 or s.circuit.probe == i
    all {s.running.get(i).started,
      if (current(s, i) and s.enrolled) commit({...s,
        entries: s.entries.set(i, {...s.entries.get(i), pinned: true, pinKey: s.running.get(i).key,
          failures: 0, retrying: false, due: if (s.entries.get(i).retrying) 0 else s.entries.get(i).due}),
        running: s.running.set(i, emptyJob), circuit: if (closes) emptyCircuit else s.circuit,
        answering: closes and s.hold < 0}, "success", i)
      else drop(i)
    }
  }
  // Every transient label retries per trace. Only outage labels feed the circuit;
  // witness_redaction_failed ends the job as Done for the lane (probe released).
  action transientEnd(i: int, wait: int, outage: bool): bool = {
    val e = s.entries.get(i)
    val n = if (e.failures >= 5) 5 else e.failures + 1
    all {s.running.get(i).started, wait >= nominal(n) / 2, wait <= nominal(n),
      if (current(s, i) and s.enrolled) commit({...s,
        entries: s.entries.set(i, {...e, failures: n, attempts: e.attempts + 1, retrying: true, due: s.now + wait}),
        running: s.running.set(i, emptyJob), answering: false,
        circuit: if (outage) failed(s.circuit, i, s.running.get(i).dispatched, s.now) else cancelled(s.circuit, i)},
        if (outage) "transient" else "redaction", i)
      else drop(i)
    }
  }
  action transient(i: int, wait: int): bool = transientEnd(i, wait, true)
  action redactionFailed(i: int, wait: int): bool = transientEnd(i, wait, false)
  action busy(i: int, hint: int): bool = {
    val deadline = s.now + hint
    val e = s.entries.get(i)
    val entries = Ids.mapBy(j => {
      val row = s.entries.get(j)
      if (j == i) {...e, attempts: e.attempts + 1, retrying: true, due: capacityDue(s, i, hint)}
      else if (waiting(s).contains(j) and row.state == "approved") {...row, retrying: true, due: capacityDue(s, j, hint)}
      else row
    })
    all {s.running.get(i).started, hint >= 1, hint <= 3600,
      if (current(s, i) and s.enrolled) commit({...s, entries: entries,
        queued: Ids.mapBy(_ => emptyJob), running: s.running.set(i, emptyJob), answering: false,
        hold: if (s.hold > deadline) s.hold else deadline, circuit: cancelled(s.circuit, i)}, "busy", i)
      else drop(i)
    }
  }
  action permanent(i: int, trustHold: bool): bool = all {s.running.get(i).started,
    if (current(s, i) and s.enrolled) commit({...s,
      entries: s.entries.set(i, {...s.entries.get(i), state: if (trustHold) "pending" else "refused",
        held: trustHold, retrying: false}),
      running: s.running.set(i, emptyJob), answering: false, circuit: cancelled(s.circuit, i)}, "permanent", i)
    else drop(i)
  }
  // Exact existing bug: defer_without_attempt writes no retry reason.
  action watchdog(i: int): bool = all {s.running.get(i).started,
    commit({...s, entries: if (current(s, i)) s.entries.set(i, {...s.entries.get(i), due: s.now + 600}) else s.entries,
      running: s.running.set(i, emptyJob), circuit: cancelled(s.circuit, i)}, "watchdog", i)}
  action cancel(i: int): bool = all {s.entries.get(i).state == "approved",
    commit({...s, entries: s.entries.set(i, {...s.entries.get(i), state: "pending",
      retrying: false, due: 0, held: false, pinned: false, key: emptyKey})}, "cancel", i)}
  pure def canApprove(t: State, i: int, manual: bool): bool = t.enrolled and not(t.paused) and
    t.entries.get(i).state == "pending" and (manual or not(t.entries.get(i).held))
  action approve(i: int, manual: bool): bool = all {canApprove(s, i, manual),
    commit({...s, entries: s.entries.set(i, {...s.entries.get(i), state: "approved",
      key: {revision: s.entries.get(i).revision, manual: manual, approvedAt: if (manual) s.now else -1},
      retrying: false, due: 0, failures: 0, held: false})}, "approve", i)}
  // Projection of a queued approval being retracted for changed consent/input terms.
  action termsChanged(i: int): bool = all {s.entries.get(i).state == "approved",
    commit({...s, entries: s.entries.set(i, {...s.entries.get(i), state: "pending",
      revision: 1 - s.entries.get(i).revision, key: emptyKey, pinned: false, held: false,
      retrying: false, due: 0})}, "terms", i)}
  action upload(i: int): bool = all {s.enrolled, not(s.paused), s.entries.get(i).state == "approved",
    s.entries.get(i).pinned, s.entries.get(i).pinKey == s.entries.get(i).key,
    commit({...s, entries: s.entries.set(i, {...s.entries.get(i), state: "uploaded"})}, "upload", i)}
  action tick(seconds: int): bool = all {seconds > 0, commit({...s, now: s.now + seconds}, "tick", 0)}
  action pause(value: bool): bool = commit({...s, paused: value}, "pause", 0)
  action enrollment(value: bool): bool = commit({...s, enrolled: value}, "enrollment", 0)
  action restart = commit({...s, queued: Ids.mapBy(_ => emptyJob), running: Ids.mapBy(_ => emptyJob),
    circuit: emptyCircuit, hold: -1, answering: false}, "restart", 0)

  action step = {
    nondet i = Ids.oneOf()
    nondet manual = Set(false, true).oneOf()
    nondet seconds = Set(1, 29, 30, 59, 60, 120, 240, 300, 301, 3600).oneOf()
    nondet hint = Set(1, 45, 120, 300, 3600).oneOf()
    val n = if (s.entries.get(i).failures >= 5) 5 else s.entries.get(i).failures + 1
    nondet wait = (nominal(n) / 2).to(nominal(n)).oneOf()
    any {enqueue(i), dispatch(i), begin(i), drop(i), success(i), transient(i, wait),
      redactionFailed(i, wait), busy(i, hint),
      permanent(i, manual), watchdog(i), cancel(i), approve(i, manual), termsChanged(i), upload(i), tick(seconds),
      pause(manual), enrollment(manual), restart}
  }
  val safety = all {waiting(s).size() <= QueueBound, running(s).size() <= 2,
    waiting(s).intersect(running(s)).size() == 0,
    s.circuit.probe < 0 or (s.circuit.reopen >= 0 and running(s).contains(s.circuit.probe)),
    Set(0, 30, 60, 120, 240, 300).contains(s.circuit.delay),
    Ids.forall(i => not(s.entries.get(i).pinned) or s.entries.get(i).pinKey == s.entries.get(i).key),
    checked.dispatch, checked.request, checked.pin, checked.upload, checked.approval,
    checked.isolated, checked.late, checked.capacity, checked.refreshed}
}
