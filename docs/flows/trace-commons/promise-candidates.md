# Owner-selected safety hypotheses

The owner selected all 13 candidates with “Check all 13 candidates.” These
remain verification hypotheses, not established guarantees. Their exact wording
is preserved below. See [results](proofs/results.md) and the recorded
[selection](promise-selection.json).

| ID | Product-language promise | What it would rule out / scope |
| --- | --- | --- |
| P1 | Automatic sharing starts only after my matching access grant has completed. | Uploading under a pending, refused or stale grant; changing Ask before successful completion. Detailed first-run/consent bounds apply. |
| P2 | What I approved controls what is submitted; changed source or consent requires a fresh decision. | Sending a new source/configuration revision under old approval, or rebuilding a missing pinned preview. The approved contributor-verdict overlay is permitted; literal serialized preview-byte equality is not the promise. |
| P3 | A temporary rate limit or witness-capacity refusal keeps my approval for a later retry. | Losing the approved queue entry or treating an unjudged temporary refusal as a final rejection. It does not promise that a permanently unavailable service recovers. |
| P4 | Signing out prevents a new upload from starting under the removed enrollment. | Reusing a cached upload claim after the pre-send enrollment check fails. It does not claim to cancel bytes already sent or an external request already started. |
| P5 | Retrying the same submission returns its saved result; reusing its key for different content is refused. | Duplicate durable receipt/run creation for an unchanged replay and silent key/content conflicts. Receipt commit and response delivery remain separate. |
| P6 | A trace stays on the policy bundle that accepted it, even when the operator activates another bundle. | Moving an existing run's Review, Score or Settle to the newly active bundle. |
| P7 | A worker or reviewer whose claim is stale cannot commit a new phase decision. | Committing with an expired/replaced token. Recording a refusal/retry for stale work remains allowed. |
| P8 | Missing required authority or privacy controls cannot silently produce an accepted submission or favorable phase decision. | Weaker-backend fallback or favorable decisions from absent evidence. External validation booleans remain assumptions; this does not prove cryptography or middleware correctness. |
| P9 | A retry cannot create a second internal credit for the same operation, and payout retries retain their operation identity. | Duplicate internal ledger events and changing the external payout identity after an uncertain response. External provider idempotency is an explicit contract, not an exactly-once network-delivery theorem. |
| P10 | Taking a trace back, revoking it or reaching expiry prevents a new export from completing with that trace. | Completing a previously prepared export after its source becomes ineligible. Previously delivered exports are not recalled, and physical index removal can finish later. |
| P11 | Another tenant's request cannot change my receipt or complete an export of my trace. | Cross-tenant effects in the modeled receipt and export seams. Public source/catalog routes remain intentionally public. It does not imply complete database/RLS implementation verification. |
| P12 | A late result from my previous account never replaces the current account's displayed history. | Publishing a captured old-account IPC snapshot after account change. Current source may violate this candidate; a model failure must be replayed before it is reported as a product defect. |
| P13 | A tested skill is installed only after I confirm the matching installation plan. | Installing an unconfirmed/stale candidate or displaying daemon success as validated local installation without reconciliation. Filesystem/installer algorithms remain bounded interfaces. |

Progress promises need a separate choice about the user and environment.
An example is: **“If I keep retrying, the required services keep recovering,
and a worker continues to receive time, an eligible approved trace eventually
gets a final processing result.”** This is not proposed as an unconditional
guarantee: human-review decisions, policy suspension, retries and permanent
refusals need explicit scope and fairness before it can be encoded.

The owner may select all P1–P13, a subset by ID, or revise their wording.
Selecting them authorizes checking, not changing production behavior or
weakening a promise when it fails. Each selected result must later be reported
as holds, breaks, or not checked under the documented finite bounds.
