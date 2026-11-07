# Client safety proofs and source replays

P1, P2, P3, P4 and P12 now have complete induction certificates for the documented
client models. Each certificate checks initiation, preservation by the **entire
original step**, implication to the selected properties, and a nonvacuity
counterexample to `false` from the arbitrary initializer. Independent review
confirmed exact state/domain coverage and current source, harness and log hashes.
This is a theorem about the source projection, not universal application or
cryptographic correctness. Native XCTest regressions validate the application
repair; extracted source-method replays provide additional focused evidence.

| Property | Model | Current result |
| --- | --- | --- |
| P1 Automatic sharing starts only after my matching access grant has completed. | client_consent | Proved by full-step induction. |
| P2 What I approved controls what is submitted; changed source or consent requires a fresh decision. | client_contribution | Proved after correcting captured-config fidelity. |
| P3 A temporary rate limit or witness-capacity refusal keeps my approval for a later retry. | client_contribution | Proved. |
| P4 Signing out prevents a new upload from starting under the removed enrollment. | client_contribution | Proved at the modeled pre-send authority boundary. |
| P12 A late result from my previous account never replaces the current account's displayed history. | client_contribution | Proved after the native callback-ownership repair. |

## Models, domain coverage and checking

Run from this checkout with Java 21 and Quint 0.33.0 available:

```sh
python3 docs/flows/trace-commons/proofs/client_induction.py client_consent \
  --out /private/tmp/my-consent-induction --seconds 600 --port 8851
python3 docs/flows/trace-commons/proofs/client_induction.py client_contribution \
  --out /private/tmp/my-contribution-induction --seconds 600 --port 8851
```

Use fresh evidence directories and a server port not used by another checker.
The generated harnesses import the complete source-model `init` and `step`;
50 consent transitions and 76 contribution transitions were checked. They do
not replace the step with a local subset. No model `CONSTRAINT`, dropped
transition, extra permission guard or reachable-state sampling establishes the
result. `CHECK_DEADLOCK FALSE` excludes an unselected deadlock theorem and does
not constrain transitions.

Every field of the exact source state type is generated independently. Maps
have exactly the original complete identity sets: folders/entries 0 and 1,
history account keys 1 and 2. Primitive records and scalar values use typed
`apalache::generate(1)` followed by the documented `domainFor` and `strongFor`
filters. This covers the product of all permitted component values rather than
an enumeration of reachable traces. `domain-coverage.json` records every
scalar membership condition, constructor component and fixed map key. The
preservation target includes domain closure as well as the strengthening.
The strengthening is proved from original initialization and preserved by the
full original step; it is not an exploration restriction.

Consent strengthens the claim with independently captured completed-grant
read evidence, completed folder/global authorization provenance, and automatic
entry provenance. Contribution strengthens it with the actual captured upload
scopes, active-upload row identity and source/pin relationships. For example,
`offeredRev` remains 0 in this projection: discovery is the only assignment and
a source can change only after discovery. A fresh superseding allocation is
outside the source model, as documented in its baseline bounds. This derived
fact is proved under the full step, not assumed to block a transition.

The final checked model/harness hashes are:

| Artifact | SHA256 |
| --- | --- |
| client_consent.qnt | b4c00c7f25ddda740375cf589463facd5b2e251924b6908cee7ae9bcbb156b87 |
| client_consent_induction.qnt | fedbeec53947d1242fdbeb412cd3acfbc38b18aaf9f15ce62b35fba9cf77d9b2 |
| client_contribution.qnt | e612dbe9c2cdb247e0eaaa4ed2797f51d276ce1e3d744a8ca2c265fc24aae264 |
| client_contribution_induction.qnt | 111066737e09653c47f480901da555dc704c1543daa8a0f213dca7ba8a7d98fb |

Raw local certificates, commands, domain coverage and phase logs are under
`/private/tmp/tc-client-consent-induction-final` and
`/private/tmp/tc-client-contribution-induction-final`. Each phase records current
model/harness hashes, command, working directory, Java environment, elapsed
budget, log path and log hash. The first three phases end with `NoError` and
`[ok] No violation found`, exit 0; nonvacuity produces an actual counterexample.
The full-step checks took about 48 seconds for consent and 87 seconds for
contribution in the checker itself. Earlier resource-capped TLC and failed
frontend attempts are historical, not the final proof result.

## Faithful asynchronous read ownership

Native AppModel now captures retained reference-identity epochs and account/client
ownership for history and saved-grant reads. Pending callbacks retain retired
epoch objects, so later epochs cannot alias them through numeric wrap or ABA. Completion checks run before publication,
read-status bookkeeping or error effects. Beginning/finishing a grant mutation
invalidates grant reads; no new grant snapshot starts while a mutation is
active. Processed status/resync notifications request a fresh grant snapshot;
an unseen remote mutation is not treated as synchronously known to the UI.

The models preserve stale callbacks in a separate class instead of removing
them from the behavior. A latest request has an independent capture. Starting
a newer read moves the previous latest into the stale class. A stale bit means
at least one pending old callback; discard can leave it set or clear it,
overapproximating arbitrary stale multiplicity. A stale callback can never
become the latest request merely because another request uses the same account.
The contribution account can cycle 1 ↔ 2, so the old-account ABA sequence is
represented. Owner invalidation retains already displayed snapshots.

P1's UI facet observes a captured **completed matching include-existing grant
snapshot**, plus current local request/mutation ownership. Requiring the daemon
grant still exist at callback time would overstate the selected promise: a
successful read may linearize before an unseen daemon terms sweep. Such a
transient informational snapshot does not newly authorize sharing. The remote
sweep remains enabled; no fictitious synchronous notification was added. The
original local Ask race is still a required negative control.

Actual automatic approval, uploader candidate selection and uploader claim
also observe completed authorization provenance. Direct folder/global
Automatic writes are successful explicit authorization, not excluded cases.
Current policy/terms/claim guards remain in the source projection. Legacy grant
withdrawal may retain previously armed folders by design; the proof does not
replace this behavior with a permanently required standing grant.

## Historical counterexamples and source correspondence

The pre-repair P12 path was:

```text
open history → refresh old account → change account → publish old snapshot
```

The pre-repair P1 path was:

```text
completed grant → capture Automatic read → withdraw grant → publish old read
```

Both were replayed using the actual old source-method bodies. P1 demonstrated
stale UI selection while the daemon remained withdrawn; it did not demonstrate
unauthorized bytes being uploaded. The shortest explanatory model path used
internal selectors before the grant reply; the reachable native setup used a
previously known completed grant plus a background refresh. Native local
`working` controls do not block those background reads. The original TLC
reports remain historical regression evidence in the proof artifacts.

The first P2 counterexample was a **model mismatch**:

```text
discover → choose Automatic → automatic approval → claim → save new scopes → send
```

The old observer compared approval to the mutable saved preference, although
the payload uses the config captured by the pass. Source `drain_approved` loads
owned `cfg`, constructs `SubmitContext::new(&cfg, ...)`, and compares approval
with that snapshot under the queue claim lock. `effective_config` clones it;
`effective_cfg()` returns that owned context. Saving new scopes does not rewrite
an in-flight payload. A new pass captures the new scopes and rejects an old
approval. `uploadScopes` and `sentScopes` now expose the actual used snapshot;
no guard was added to suppress scope changes or make the old path disappear.
The historical seven-step counterexample must not be attributed to the final
model hash.

## Replays and focused negative controls

```sh
python3 docs/flows/trace-commons/proofs/history_source_replay.py
python3 docs/flows/trace-commons/proofs/consent_source_replay.py
python3 docs/flows/trace-commons/proofs/scopes_source_replay.py
python3 docs/flows/trace-commons/proofs/client_negative_controls.py
```

The first two default to **asserting the fixed live source**. They extract the
current methods/ownership helpers unchanged and await returned tasks rather
than relying on a drain sleep. Current replies publish; stale successes and
stale failures cannot replace data or change current read/error bookkeeping.
The consent replay keeps Ask selected after an old Automatic reply, with the
synthetic daemon still withdrawn. The Rust helper replay confirms captured
upload config and the next-pass rejection negative control. No credentials,
network, native app launch or real application data is involved.

These are method-boundary harnesses with explicit dependency shims, not full
SwiftUI/daemon integration. History records/rollups become synthetic labels,
unrelated refreshes are no-ops, account scope hook setup is projected to its
read invalidation, and fake clients provide deterministic synchronization.
Scripts print source/method hashes. Native XCTest regressions remain the
primary repair tests.

Before beginning a grant/Ask mutation, the final source now settles an unknown
account owner through `settleAutomaticGrantOwner()`. A known scope proceeds
immediately; otherwise only a current successfully published status establishes
an owner (successful nil remains valid for legacy/watch-only). Failed or stale
status does not begin a mutation. This prevents initial nil → known discovery
from retiring an already confirmed write. The P1 projection begins at this
established-owner mutation boundary; the initial-owner lookup is an external
validated precondition, with dedicated native XCTest regressions. The consent
source probe extracts the actual preflight helper and starts with a known scope;
its unused unknown-owner shim traps rather than pretending to validate that
path. Both probes were rerun against final AppModel SHA256
`7f120214910f356a6397634fd97bd87ade2bcb2a53b722a83ddc0cfb9f461d05`.

The focused model controls passed 12 assertions, including the original Ask
ordering, an Automatic result without completed grant evidence, stale epoch
publication, unapproved send scope, lost approval on witness capacity, bypassed
logout authority, and a stale-account callback fault. Mutants/fault fixtures
exist only in temporary copies; they deliberately show the selected monitor
turns false. The account ABA control ensures discarding an old callback cannot
consume or resurrect the latest request. These controls test proof sensitivity;
they are not new reachable product-bug claims.

## Source-model limits

Two trace/folder identities, one uploader/preview, bounded source/input revisions
and retry relations remain the documented projection. Numeric grant IDs are
canonical freshness names, not actual wrapping implementation counters. Callback
classes overapproximate stale multiplicity; the model does not prove every
scheduler, UI framework or wire-decoding behavior. Enrollment is a live config-
and-key authority Boolean; full unenroll interruption and arbitrary enrollment
identity equality remain source integration concerns. Source/input/artifact
checks occur at their modeled send boundary, so later changes do not
retroactively invalidate an already started request.

The asynchronous witness worker remains an external validated witness outcome.
Shared `send_gates`/`load_approved` were audited, but admission job identities,
certificate cryptography, worker persistence/recovery and nondeterministic
classifier correctness are not proved by this client model. There is no
unconditional progress/fairness theorem or universal cross-model source
refinement claim.
