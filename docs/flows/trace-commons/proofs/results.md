# Selected safety results

**All 13 selected hypotheses now hold in the reviewed models.** There are 15 predicates across seven models because P8 and P11 each span two seams. Each model has a complete induction certificate with initialization, full-transition preservation, implication to the selected predicates, and a nonempty arbitrary-state initializer.

The native stale-history and stale-grant findings are repaired in this PR. Source regressions, structural checks and native tests are separate from the model proofs. This is not a machine-checked equivalence theorem for the application, cryptography, database/RLS implementation, filesystem or deployed configuration. The source [inventory](../coverage.md) retains explicit correspondence gaps.

## Verdicts

| ID | Selected hypothesis | Verdict | Model / boundary |
| --- | --- | --- | --- |
| P1 | Matching completed grant before automatic sharing | **Holds** | Consent authorization and current local grant-read identity; remote snapshots are informational, not instantaneous authority. |
| P2 | Approval controls submitted source and consent | **Holds** | Actual captured send configuration and pinned preview/source boundaries. |
| P3 | Temporary rate/capacity refusals preserve approval | **Holds** | Queue approval, retry state and retained metadata. |
| P4 | Removed enrollment prevents a new upload | **Holds** | Pre-send live enrollment/config/key authority; already-started external requests are not cancelled. |
| P5 | Stable retry result; different content is refused | **Holds** | Saved structured receipt/result selection, not serialized HTTP byte equality. |
| P6 | Accepting policy bundle stays fixed | **Holds** | Each modeled pipeline run. |
| P7 | Stale claims cannot commit phase decisions | **Holds** | Actual selected worker/reviewer claim, including replacement/expiry. |
| P8 | Missing required authority/privacy fails closed | **Holds: both seams** | Receipt and pipeline; external validation facts remain assumptions. |
| P9 | No duplicate internal credit; stable payout identity | **Holds** | Modeled operation identities/internal commits; provider idempotency and database atomicity remain explicit contracts. |
| P10 | Revocation/expiry blocks new export completion | **Holds** | Completion-time eligibility; historical deliveries are not recalled. |
| P11 | Tenant isolation at receipt/export seams | **Holds: both seams** | Receipt and lifecycle; not a complete middleware/RLS theorem. |
| P12 | Old-account replies cannot replace current history | **Holds** | Current account/client/request identity, including account ABA and arbitrary stale callback multiplicity. |
| P13 | Matching confirmation before tested skill installation | **Holds** | Confirmation lineage and independent publication reconciliation; installer/crypto remain bounded interfaces. |

The exact owner-selected wording remains in [the hypothesis list](../promise-candidates.md). No hypothesis was dropped or weakened.

## Repairs and regression evidence

The original generic callback applied every reply and error to the current model. A delayed history read could replace the new account's history; a delayed grant read could restore Automatic after Ask completed. The native repair:

- Captures retained account/client/request identity tokens before reads, and checks them before data, errors or read bookkeeping are published. Tokens cannot be reused while an old callback retains them.
- Fences status reads and awaited first-run status consumers, so an old status cannot restore the previous account identity and undermine history fencing.
- Invalidates grant reads at write start/completion. Overlapping writes retain their gate; an old account's completion cannot release the current account's gate.
- Refreshes grant snapshots on daemon status/resync notifications. Subscription callbacks also retain the client connection identity; ordinary account changes do not invalidate a still-current subscription.
- Establishes an initially unknown owner before dispatching Grant/Ask. A delayed initial status can no longer turn a confirmed grant into a refusal and make onboarding fall back to Ask. Confirmed legacy/watch-only nil scope remains supported.

The [native tests](../../../../macos/Tests/TraceCommonsAppTests/ScopedGrantHistoryReadsTests.swift) await the actual callback tasks, without drain sleeps. They cover current replies, stale successes/failures, same-account out-of-order reads, account/client ABA, sign-out, overlapping writes, notification refreshes, direct first-run consumers, connection events and initial owner discovery. Guard-bypass controls failed before the restored implementation passed. Final native commands/results and source hashes are recorded in [the evidence manifest](evidence/manifest.json).

[History](history_source_replay.py) and [consent](consent_source_replay.py) source-method regressions extract the current method bodies and ownership helpers and assert the repaired behavior with synthetic clients. They use no credentials, network or user data. Initial-owner preflight and complete native integration are covered by XCTest; the extracted-method shims are not an installed-app test.

The original counterexamples remain as [historical charts](historical/README.md), [P1 TLC output](evidence/p1-tlc-report.md) and [P12 TLC output](evidence/contribution-tlc-report.md). They describe the pre-fix model hashes, not the current source. No exhaustive current state graph is claimed.

## Why the previously incomplete checks now complete

The original attempts explored long execution prefixes and hit resource/exploration limits. Bare inductive predicates also lacked an arbitrary-state domain assignment. The new proof-only harnesses import the full source-model `init` and `step`, construct every state in a reviewed domain satisfying a strengthening, then check:

1. Original initialization establishes the domain and strengthening.
2. Every original transition from any such state preserves both the domain and strengthening.
3. The strengthening implies the selected predicates.
4. Checking `false` from the arbitrary initializer produces a state-zero counterexample, confirming it is nonempty.

The one-transition preservation query is an induction obligation over arbitrary valid states, not a depth-one reachability claim. Domain closure and generator coverage are essential; all original map keys, fields, identity sets and actions are covered. A separate reviewer checked these relationships, full-step aliases, selected conclusions, coherent source/harness hashes and terminal solver verdicts. See [Quint's induction documentation](https://quint.sh/docs/checking-properties).

No production behavior changed to resolve P5–P11/P13: proof-only strengthening eliminated unreachable arbitrary-state counterexamples, and all obligations subsequently passed. Such counterexamples were proof gaps, not newly discovered product defects.

Input models have no exploration constraints and retain their original identity bounds. Client changes model the actual freshness repairs. Stale callbacks remain possible; a nondeterministic stale-multiplicity class overapproximates any number of pending obsolete callbacks. Account changes may cycle without reactivating stale reads. No fairness/progress theorem is claimed.

## Encoding corrections and limits

P1's old UI observer demanded the daemon's *current* grant Boolean at callback time. A valid completed-grant snapshot can be followed by an unseen remote sweep before UI publication. That is not a new sharing authorization or a local Ask mutation. The corrected observer checks captured completed-grant evidence plus current local request/account/mutation identity. Actual arming/upload authority checks remain. Remote sweep actions are retained; asynchronous snapshot display is not instantaneous remote-state knowledge.

The historical P2 scope-change counterexample compared approval with the mutable saved preference, although the daemon uses a captured configuration. The model records `uploadScopes`/`sentScopes` and compares approval with the actual submitted scope. Consent changes remain enabled. [Extracted Rust helpers](scopes_source_replay.py) and [fault controls](scopes_observer_control.py) validate snapshot retention and detection of an unapproved sent scope.

P5 additionally observes the actual selected saved structured result; same-body replay cannot silently become an arbitrary refusal and a changed body cannot silently succeed. Positive/negative receipt controls and confirmation/publication skill controls remain. Client fault controls also detect lost retry approval, bypassed enrollment checks, unconfirmed Automatic results and stale-account publication.

Finite source-domain bounds and assumptions remain documented in [client notes](client-notes.md), [server induction notes](server-induction-notes.md), [ingest notes](server-ingest-notes.md), [credit/export notes](credits-exports-notes.md) and [skill notes](skill-install-notes.md). There is no universal cross-model composition or executable-source refinement proof. Already displayed snapshots need not disappear instantly when accounts change; obsolete replies cannot publish again.

## Reproduce and inspect

Tools are pinned to Quint 0.33.0, Apalache 0.62.1 and Java 21. The earlier TLC 1.7.4/flow-promises artifacts retain their tool hashes. Complete certificates and deterministic compressed raw logs live under [evidence/induction](evidence/induction/). Each certificate records exact commands, current model/harness hashes, raw/compressed log hashes, exit status and per-phase outcomes.

```sh
# Java 21 must be on PATH; use a fresh output directory and exclusive endpoint.
python3 docs/flows/trace-commons/scripts/run_induction.py \
  --spec docs/flows/trace-commons/proofs/induction-selection.json \
  --out /private/tmp/tc-induction-NEW --endpoint localhost:8860 --seconds 600
python3 -m unittest discover -s docs/flows/trace-commons/scripts -p 'test_*checks.py'
python3 docs/flows/trace-commons/proofs/history_source_replay.py
python3 docs/flows/trace-commons/proofs/consent_source_replay.py
```

The runner refuses stale output directories and unselected checks, fingerprints inputs, rejects tool errors even when a process exits zero, and requires all four coherent obligations. Semantic harness/source review remains separate from log classification. The runner was also validated end to end on the lifecycle certificate. Decompress a retained log with `gzip -dc FILE.log.gz`.
