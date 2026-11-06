# Client proof encoding and source replay

The owner selected all P1–P13 on 2026-11-06. This note covers P1–P4 and P12.
It records source-model fidelity and local checks, not exhaustive TLC results.
Relevant source seams were reviewed from the original `8218cc8` baseline to
`a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`.

## Observers and bounds

All proof booleans start true and latch false when the represented event
violates its predicate. No proof field occurs in a transition guard. The
additional authorization provenance, saved-read freshness and published-owner
fields do not enable or disable original consent transitions. Thus, erasing
these passive fields recovers the consent behavior projection. History is an
explicit source-fidelity expansion of the original contribution projection,
not merely passive instrumentation: the former single `historyRead` silently
lost an outstanding old-account callback when the account changed.

The source/consent/input revisions are symbolic, source/input widening is
bounded, there are two trace rows and one upload/preview. Enrollment remains a
live-config-and-key Boolean, not arbitrary enrollment identity generations.
Consent grant identities use the existing canonical freshness abstraction;
new reads receive a freshness bit invalidated by grant replacement/withdrawal,
so old replies do not become current again when the two canonical names cycle.

History has two request slots, one per represented account (1 then 2), and a
captured target row per slot. Old and new requests may coexist and complete in
either order. Multiple outstanding requests from the same account coalesce in
this projection; arbitrary same-account concurrency and account cycling are
outside the bound. Empty history snapshots can publish. Callback publication
is separate from server-side `previous_account` classification: a legitimate
previous-account row in a fresh snapshot is not itself a P12 violation.

No constraints, fairness assumptions, new permission guards or production
behavior changes were added. These full records have large cross products;
a resource-limited search or sampled run cannot be called exhaustive proof.

## Predicate ownership and source anchors

- P1: `client_consent.P1` checks completed matching grant arming, captured saved-
  grant response freshness when Automatic is displayed, and completed authority
  provenance when the watcher automatically approves. Successful direct folder
  or global Automatic policy writes count as explicit local authorization;
  they are covered, not excluded. `completedAuthorization` and
  `overrideAuthorization` record those successful writes. The source mapping
  is `policy.rs` automatic-grant arming / `ipc.rs` project and global policy
  setters, plus `SharingChoiceCard.swift` and AppModel's `automatic_grant` read.
  Legacy withdrawal retains previously armed folders by design, so the
  predicate observes authority at start rather than requiring a live standing
  grant forever. An old saved-grant response may still publish Automatic;
  that is retained as a possible UI-facet counterexample, not assumed to be
  proof that bytes were uploaded without authority. First-run fallback and
  successful grant/Ask sequencing remain separately modeled.
- P2: `client_contribution.P2` observes checking → sending. It compares source
  versus offered revision, approved scopes versus the captured send-context
  scopes, approved inputs versus modeled pipeline inputs, and pinned
  revision/artifact when a pin exists. Later changes
  after sending do not retroactively fail the event. An ordinary unpinned
  path is allowed. `uploader.rs:617` (`load_approved`) re-hashes source and
  checks the input fingerprint; `approved_envelope` validates stored bytes
  before the approved contributor-verdict overlay. `mod.rs:1510–1555` checks
  scopes under the claim lock against the pass's captured config. The refreshed model captures that context in `uploadScopes` and records
  `sentScopes` when sending starts. Saved scope changes remain enabled after
  claim; they do not mutate the already captured context. Full pass iteration
  and config-load/lock ordering remain abstract.
- P3: the two deferral actions compare their actual constructed next entry
  against the previous entry: approved/retryable, same pin, artifact, approval
  scopes/inputs, origin and attempt budget. Temporary deferral is an event
  promise; later consent changes may legitimately revoke approval. HTTP 429
  can happen after sending, so rate-limit deferral now covers checking (local
  pacing) and sending (remote response). Witness capacity remains pre-send.
- P4: checking → sending observes live enrollment, matching
  `uploader.rs:466` (`enrollment_is_live`, readable config and device key) and
  `send_gates:562–616`. The actual daemon also holds new claims and races the
  upload against unenroll interruption (`mod.rs:1147–1181`, `1510–1530`). The
  model Boolean is the live pre-send authority seam; it does not prove every
  cancellation race or equality of two distinct enrollments. Requests already
  started may complete after logout, as the selected wording permits.
- P12: each successful history callback observes captured request account
  equals current account. The historical monitor does not demand that an
  already displayed snapshot vanish immediately when account scope changes.
  Current `AppModel.swift:2031–2038` routes `refreshHistory` through generic
  `perform:4431–4462`, which captures a client and unconditionally publishes
  success on MainActor. `status.didSet:356–381` resets contribution readiness
  and account notices, with no history reset/cancellation. The unrelated
  contribution-account callback at `298–318` explicitly checks client identity,
  scope and cancellation; generic history callbacks do not.

The new asynchronous witness worker is represented by coarse witness outcomes.
Its shared `send_gates` and `load_approved` checks were reviewed; worker admission
identity, scheduling, async certified-pin persistence and recovery are outside
this model. The original line annotations are baseline anchors; named current
source symbols above identify the refreshed seams.

## Local validation and P12 replay

Both changed models pass `quint typecheck` with Quint 0.33.0. The existing
`verify.py.validate_frames` check matches all 4 consent and 16 contribution
frames/controls. No TLC search was launched by this work unit.

A deterministic model trace was tested in a temporary copy by adding:

```quint
run history_owner_replay = init
  .then(YouOpenHistory)
  .then(YouRefreshHistory)
  .then(TheCurrentAccountChanges)
  .then(TheHistoryReadPublishesAnEmptySnapshot)
  .then(all { assert(not(P12)), s' = s })
```

`quint test /private/tmp/client_contribution_replay.qnt --main
client_contribution --match history_owner_replay --max-samples 1 --backend
typescript` returned `1 passing`: the assertion confirms the model violation,
not that P12 holds. With only the old-account request pending, the publishing
callback necessarily belongs to that old account.

Run the checked-in source-method replay from any directory on macOS with Swift 6:

```sh
python3 docs/flows/trace-commons/proofs/history_source_replay.py
```

It extracts the exact current `refreshHistory` and `perform` bodies and compiles
them unchanged against synthetic clients. The negative control completes an
old request without changing accounts; its displayed owner matches. The race
then blocks the old `listHistory`, changes the synthetic current account/client,
publishes the new snapshot, and releases the old result. Observed output:

```text
control: no account change; history matches current account
current account=new; new history published=["new"]
late old result replaced current history=["old"]
```

The actual source-method boundary therefore reproduces the stale publication.
It uses no credentials, network requests or application/project data. It is not
an installed-app or full SwiftUI integration test. Dependency shims reduce
history records/rollups to synthetic account labels, `publishIfChanged` to an
assignment, read bookkeeping to a no-op, and unrelated error-copy dependencies
to stubs. The synthetic account change represents `status.accountScope`; the
current source observer has no relevant history/cancellation effect to omit.
The script prints source and extracted-method SHA256 hashes to identify exactly
which method bodies it replayed. The reviewed AppModel SHA256 was
`e58e157dcfaa6c09d93ea4b7466ea95c4e739bdd6c0cde0ddc3f4e901ffd8532`;
refreshHistory was
`bb6f4c1b34a47feb157dc02870a988f469411f4bcd2f6b87d664192e39d33a98`;
perform was
`249bb8553f984163d6a440b367633dbaad76b611d6ad232375460083158d1ae3`.
No production fix is included in this proof PR.

## P1 stale saved-grant UI replay

The formal P1 counterexample reported by the runner follows grant saved →
saved-grant read captured → grant withdrawn → old read displayed. Its shortest
model path uses internal selectors before the grant reply reaches the native
UI; native controls are disabled while the grant request is working. That
specific shortest selector sequence is not itself a literal native click
reproduction. The source-equivalent race starts after a completed grant is
already known and displayed, with another background refresh outstanding.

`python3 docs/flows/trace-commons/proofs/consent_source_replay.py` extracts the
exact current `refreshProjects`, `setAskEachTime`, `firstRunCall`, `perform` and
`SharingChoice.current` methods into a synthetic Swift harness. It begins with
that previously completed/displayed Automatic grant, blocks a captured old
saved-grant read, completes Ask using the actual extracted method, permits its
fresh read to display Ask, then releases the old read. The source-method replay
passed, printing:

```text
Ask completed: daemon grant=false; displayed choice=Ask
late old read: daemon grant=false; displayed choice=Automatic
```

This confirms the stale UI-selection facet of the counterexample. It does
**not** show automatic uploads restarting: the synthetic daemon remains
withdrawn throughout the stale publication. The same distinction applies to
interpreting P1's formal result. The user-facing choice and toolbar directly
read `SharingChoice.current(model.automaticGrant)`; the old `perform` callback
can replace that property without a freshness check. `setAskEachTime` schedules
`refreshProjects` after successful withdrawal but does not invalidate older
reads. The source replay requires no credentials, IPC or application data.

As with the history replay, unrelated dependencies are shims: project lists and
arming/status refreshes have no effect, `DaemonWork.run` uses detached synthetic
work, and a lock-backed fake daemon supplies captured grant snapshots and
withdrawal. No complete native UI, actual daemon locks, or upload path is
executed. The extracted `SharingChoice.current` remains unchanged, so the
observed stale choice follows the actual rendering predicate.

## Supplemental SMT attempts

Four `quint verify MODEL --main MODULE --inductive-invariant Pn --verbosity 3`
checks were attempted with Java 21, one per P1–P4. P1 could not construct the
arbitrary-state preservation query: `s is used before it is assigned`, because
the property alone does not define a finite domain for the entire state record.
P2–P4 suffered a shared-endpoint connection drop. These are tool/invariant-domain
failures, not reachable model counterexamples or successful induction proofs.

Fallback runs used `--invariant Pn --max-steps 8`, isolated server endpoints
8834–8837, `JDK_JAVA_OPTIONS=-Xmx1g`, and a 90-second limit per process. All four
timed out without a complete verdict. P1 reached state 8's invariant query;
the larger contribution model was still unrolling step 4. These attempts do
not establish bounded safety even through the requested depth. Commands,
working directories, elapsed times, model hashes and exact logs are preserved
under `/private/tmp/tc-quint-client-supplemental` in `jobs.json`,
`bounded-jobs.json`, and each `P*/result.json` / `verify.log`.

The models remained unchanged during these attempts. Their SHA256 hashes were:

- client_consent.qnt:
  `1f00f4582914a737e9f178403d32c557509ab62a9b7d8f6af76b7e4f20b3a045`
- client_contribution.qnt:
  `78c90dc380275ccc0e78ba5e1d6afe4fb0fe513e7e8ce5e6fe35a4b2210b2a6d`

## P2 captured-config correction and replay

The pre-correction model produced a seven-step P2 path: discover trace → open
policy → choose Automatic → automatic approval → claim → change saved scopes →
send. The old observer compared approved scopes with the mutable saved setting
at the final send edge even though it had no variable for the scope actually
used by that upload. This was a model/source correspondence mismatch, not a
confirmed application defect. The historical trace is retained in the earlier
checker artifact and must not be attributed to the final model hash.

Source ordering: `drain_approved` loads an owned `cfg` (`mod.rs:1407`), constructs
`SubmitContext::new(&store, &cfg, ...)` (`:1462`), then compares the approval to
that same snapshot under the queue claim lock (`:1540`). `SubmitContext::new`
uses `effective_config(cfg, opts)` (`submit.rs:1041`); that helper clones the
provided config (`:3184`) and `effective_cfg()` returns that owned clone
(`:1467`). Later saved-config changes do not mutate the in-flight payload's
scope list. The source explicitly treats the row as already in flight after
claim. A new pass captures new scopes and rejects an approval for old scopes.

`uploadScopes` now captures the pass context at claim and `sentScopes` records
what the final send uses. P2 compares approval with those actual used scopes.
This adds observation of the concrete payload context; it adds no transition
guard, removes no consent change, and does not silently substitute new scopes
under an old approval. Source/input/artifact checks remain in place.

`python3 docs/flows/trace-commons/proofs/scopes_source_replay.py` replays the
extracted Rust `effective_config` and `effective_cfg` helpers, verifies that the
current source handoff/claim expressions remain present, and exercises the
owned-config boundary with synthetic data. It passed, printing:

```text
saved config changed after claim: ["newly-saved-scope"]
captured send context retains approved scopes: ["approved-old-scope"]
next pass: old approval mismatches new captured config
```

This is a focused source-helper replay and source-order audit, not full daemon,
claim issuer, envelope upload or network integration. ConfigStore persistence
is represented by an owned clone; the actual claim comparison expression is
reused. No production code or dependencies were changed.

## Native control reachability of asynchronous UI races

`refreshAfterQueueChange` invokes `refreshProjects` (`AppModel.swift:1823–1836`),
and `refreshAll` invokes both history and projects (`:1993–1998`). These
background reads do not set SharingChoiceCard's local `working` state. Its
toolbar is disabled only while `working` (`SharingChoiceCard.swift:151`); the
radio group also requires known grant/copy (`:173`). A previously known
completed grant therefore permits Ask while another background read is pending.
The source replay begins in that state.

History reads similarly do not set a global account-action busy flag. The
native sign-out control is disabled only by `model.signingOut`
(`AccountSection.swift:170–172`); `signOut` (`AppModel.swift:4877–4898`) neither
cancels generic perform tasks nor clears history. Account scope changes also
have no history cancellation hook. The inspected control guards therefore do
not exclude the represented read/account-change interleaving. Full native UI
integration remains outside these isolated source-method replays.

The final P2 observer also has explicit positive/fault-injection controls:

```sh
python3 docs/flows/trace-commons/proofs/scopes_observer_control.py
```

The script copies the final model unchanged into a temporary test module. Its
fixture prepares a claimed row, then invokes the actual
`TheUploadPassesItsPreconditions` action and P2 definition. A matching captured
scope sends with P2 true. Injecting a new, unapproved scope into the captured
send context sends that injected scope but latches P2 false. Both assertions
passed (`2 passing`). The injection tests observer sensitivity; it is not a
claim that the substituted context is reachable in the application or ordinary
model. No artificial guard is added to suppress the faulty send in this test.

The final contribution model SHA256 is
`8aa6f3a4bcddfffcb3c1333b6e9d42c0f8ab88ff1af974dde448cf5716c04a00`.
It passed the scoped 128-sample × 100-step structural rerun with all 16
frame/control comparisons. Evidence is
`/private/tmp/tc-quint-client-contribution-final-structural/verification.json`.
The final scoped TLC rerun reported the same four-step P12 violation and no
P2 violation before its 100,000-state exploration stop. P2, P3 and P4 remain
**not checked**, not holds. Report and exact command/hash metadata are at
`/private/tmp/tc-quint-proof-checks/tlc-client-scopes-refresh-2/client_contribution/`.
