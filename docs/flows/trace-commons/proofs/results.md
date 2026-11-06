# Selected safety checks

The owner selected all 13 hypotheses. The final source-aligned models have **two counterexamples: P1's stale consent-choice display and P12's late account-history publication**. Both have controlled source-method replays. The other 11 promises remain **not checked to completion**. No production behavior was changed.

These results concern bounded models and reviewed source seams. They do not establish application-wide correctness, source equivalence, cryptography, complete database/RLS enforcement, filesystem correctness or deployed configuration. The refreshed inventory still has **925 correspondence gaps**.

## Verdicts

| ID | Hypothesis | Final status | Evidence and limit |
| --- | --- | --- | --- |
| P1 | Matching completed grant before Automatic sharing | **Breaks: UI publication facet** | TLC finds a ten-step stale grant-read path. Source replay shows Ask followed by a late Automatic display while the daemon stays withdrawn. No unauthorized upload is demonstrated. |
| P2 | Approval controls submitted source and consent | **Not checked** | Final TLC reruns reach the 100,000-state cap. A historical scope-change counterexample was a model/source mismatch; see below. |
| P3 | Temporary refusals preserve approval | **Not checked** | Final contribution rerun reaches the 100,000-state cap. |
| P4 | Sign-out prevents a new upload under removed enrollment | **Not checked** | Final contribution rerun reaches the 100,000-state cap. Already-started external requests are outside the promise. |
| P5 | Stable saved retry result and refusal of content conflicts | **Not checked** | Receipt exploration exceeds the process budget. Focused positive/negative controls validate the structured-result observer, not the universal property or HTTP serialization. |
| P6 | Accepting policy bundle stays fixed | **Not checked** | Pipeline exploration exceeds the process budget. |
| P7 | Stale claims cannot commit phase decisions | **Not checked** | Pipeline exploration exceeds the process budget. |
| P8 | Required authority/privacy fail closed | **Not checked** | Both receipt and pipeline seams require completed checks. External validation booleans remain explicit assumptions. |
| P9 | No duplicate internal credit; stable payout identity | **Not checked** | Settlement exploration remains incomplete; induction cannot construct the arbitrary-state initializer. External provider idempotency is assumed. |
| P10 | Revocation/expiry blocks new export completion | **Not checked** | Lifecycle exploration remains incomplete. Historical deliveries are not recalled. |
| P11 | Tenant isolation at receipt/export seams | **Not checked** | Both seam checks remain incomplete; this is not a complete RLS theorem. |
| P12 | Old-account replies cannot replace current history | **Breaks** | TLC finds a four-step history-read/account-change path; exact source methods reproduce replacement of new history by old history. |
| P13 | Matching confirmation before tested skill installation | **Not checked** | Learning exploration exceeds the budget; induction translation fails. Focused installation observer controls pass. |

There are 15 configured predicates because P8 and P11 each span two models. A multi-seam promise cannot hold unless every required seam is checked. No promise in this run has a completed universal `holds` verdict.

## Reproduced findings

**P1:** an old `automatic_grant` read can publish after Ask successfully withdraws the grant. The source replay begins with an already known, completed grant; a background refresh does not set the sharing card's local `working` guard. Ask completes, its fresh read displays Ask, then the old read displays Automatic. The daemon remains withdrawn. This is the source-equivalent stale-publication setup; the model's shortest internal-selector path is not a literal native UI click sequence.

- [TLC report](evidence/p1-tlc-report.md), [raw TLC log](evidence/p1-tlc.log), [execution metadata](evidence/p1-tlc-result.json).
- [Source replay](consent_source_replay.py) extracts unchanged `refreshProjects`, `setAskEachTime`, `firstRunCall`, `perform`, and `SharingChoice.current` methods. UI guard/source details are in [client notes](client-notes.md).
- TLC stopped on the violation after 1,107,179 distinct states (6,061,614 generated), not after exhaustive exploration. Its shortest path has ten transitions.

**P12:** start reading the old account's history, change accounts, publish new history, then allow the old reply to finish. `refreshHistory` uses generic `perform`, whose callback has no account generation check; the old rows replace the new rows. Account sign-out is not disabled by the history read's busy state.

- [TLC report](evidence/contribution-tlc-report.md), [raw TLC log](evidence/contribution-tlc.log), [execution metadata](evidence/contribution-tlc-result.json).
- [Source replay](history_source_replay.py) extracts unchanged `refreshHistory` and `perform` bodies, includes a no-account-change control, and reproduces the replacement.
- The primary TLC run found P12 after 1,762 distinct states (4,420 generated); its shortest path has four transitions. Subsequent checks of P2/P3/P4 hit the pinned checker's rerun cap.

Both Swift replays use synthetic clients and controlled scheduling. They execute actual extracted methods with dependency shims, without network, credentials or user data. They are not installed-app integration tests. The findings have not been fixed in this PR.

## Source fidelity and observer review

The first contribution observer compared approval with the mutable saved scope preference. That produced a seven-step P2 counterexample after a scope change during an upload. The actual daemon captures `cfg` before building `SubmitContext`, checks approval against that snapshot, and sends using `effective_config`/`effective_cfg`. The final model records `uploadScopes` and `sentScopes` and compares approval with the scope actually submitted. Scope changes remain enabled; no send guard was added to suppress the trace. The historical mismatch is not a product bug and does not establish P2 holds.

[Extracted Rust helpers](scopes_source_replay.py) reproduce snapshot retention and the next pass's mismatch with the old approval. [Observer controls](scopes_observer_control.py) check detection of an injected unapproved sent scope.

Review also strengthened P5: a completed replay must actually select the saved structured receipt/result; a same-body retry cannot silently become an arbitrary refusal, and a differing body cannot silently succeed. Five [positive/negative controls](server_receipt_p5_tests.qnt) pass. Serialized HTTP payload equivalence remains outside this model. Five [P13 controls](client_learning_p13_tests.qnt) check confirmed publication and detect missing confirmation or unreconciled daemon success.

Observers only record transition facts. They do not authorize transitions or shrink action domains. Source-fidelity corrections and the projection argument are documented in [client notes](client-notes.md), [ingest notes](server-ingest-notes.md), [settlement/export notes](credits-exports-notes.md), and [skill notes](skill-install-notes.md). This is a reviewed correspondence argument, not a machine-checked refinement proof.

## Bounds, tools and incomplete runs

The model seed is `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`; inventory and selected source seams were reviewed against main `a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`. Main advanced to `828ad59f5bc539fafd4915267a45152941000639` during the run; comparison found the selected source files unchanged (see the manifest). Broader source discovery remains pinned to `a2e0aa566`. Model notes state the finite domains: consent has two folders/entries; contribution has two entries/accounts and one upload slot; receipt has four attempts, two tenants and three keys per tenant; pipeline has two runs/workers/reviewers; settlement has two runs and bounded instrument/outbox identities; lifecycle has three objects, two tenants/actors and two exports; learning bounds two owners/revisions with symbolic fresh plan identities. These abstractions exclude arbitrary identity reuse, unbounded concurrency and elapsed real time.

Tools: Quint 0.33.0, Apalache 0.62.1, Java 21.0.13, TLC 1.7.4, Pele 1.9.0, Node 26.13.0 and Python 3.12.8. The flow-promises/statechart workflow is pinned at `6f35f6ae6b3581c499bdfcae080932b4ffa7e281`. TLC jar SHA256: `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.

All selected checks are safety invariants. No fairness/progress assumptions or liveness guarantees are claimed. `CHECK_DEADLOCK FALSE` excludes the unselected deadlock theorem; it removes no transition. Committed configs contain no state or action constraints. The pinned checker caps reruns after a first counterexample at 100,000 states and reports the remaining properties as not checked. The outer runner separately enforces a 120-second process budget. Neither limit produces a passing verdict.

Supplemental symbolic checks requested depths eight or ten and used 55–120-second budgets. They timed out before completing those bounds. Inductive attempts passed initiation where available, but Apalache rejected consecution because the property alone does not assign a finite arbitrary-state record (`s is used before it is assigned`). Connection drops, Java startup/cache errors and renderer failures were tool errors, not logical verdicts. Successful counterexample runs were repeated with working tool paths/caches and unchanged model hashes.

[The evidence manifest](evidence/manifest.json) records final model/config hashes, commands, run outcomes and retained artifact hashes. Raw timing-limited results are preserved even though they do not prove safety. Checked charts in the [review surface](../mocks/index.html) merge states by the existing view labels and show only counterexample paths; they are not complete application state graphs.

## Reproduce

From the repository root, with Quint 0.33.0, Swift 6, Rust, Python and Java 21 on PATH:

```sh
python3 docs/flows/trace-commons/proofs/history_source_replay.py
python3 docs/flows/trace-commons/proofs/consent_source_replay.py
python3 docs/flows/trace-commons/proofs/scopes_source_replay.py
python3 docs/flows/trace-commons/proofs/scopes_observer_control.py
quint test docs/flows/trace-commons/proofs/server_receipt_p5_tests.qnt --backend typescript --max-samples 5
quint test docs/flows/trace-commons/proofs/client_learning_p13_tests.qnt --backend typescript --max-samples 5
python3 -m unittest discover -s docs/flows/trace-commons/scripts -p 'test_proof_checks.py'
python3 docs/flows/trace-commons/scripts/run_proofs.py \
  --skill-root /path/to/pinned/bkase/plugins \
  --out /private/tmp/tc-proof-NEW --seconds 120 --jobs 1
```

Use a fresh output directory and a writable `XDG_CACHE_HOME` and npm cache. `TLA2TOOLS` may point to the verified TLC jar; `JAVA` must point to Java 21. The runner refuses unselected checks, constraints and stale output folders; changed artifacts invalidate results. Structural reproduction commands are in the [README](../README.md). All 14 model typechecks, 155 frame/control checks and 1,792 random trace projections passed; these structural checks do not replace the incomplete safety checks above.
