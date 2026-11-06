# Credit retries and export completion observations

The owner selected all 13 candidates. This slice implements P9 in
`server_settlement` and P10 plus the export portion of P11 in
`server_lifecycle`; `server_receipt` owns the receipt portion of P11. The exact
selected candidate wording is:

- P9: A retry cannot create a second internal credit for the same operation,
  and payout retries retain their operation identity.
- P10: Taking a trace back, revoking it or reaching expiry prevents a new
  export from completing with that trace.
- P11: Another tenant's request cannot change my receipt or complete an export
  of my trace.

## Scope and source correspondence

The model's source baseline is `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
The four selected source files below have no diff from that baseline to
`a2e0aa56608f8c403deff679f8b51e76ac2f5ca7` (current main at review). This is a
check of these seams, not a revalidation of every production module.

| Source anchor | Selected correspondence |
| --- | --- |
| `crates/trace-commons-server/src/versioned_pipeline.rs:15848`, `:15946`, `:16000` | Internal ledger commit checks current authority, then commits ledger/batch/completed-leg together. The modeled commit event is this atomic transaction boundary. |
| `crates/trace-commons-server/src/versioned_pipeline_credit.rs:449`, `:456`, `:465`, `:472` | Deterministic event, ledger source, batch and pipeline NEAR operation keys. Immutable finite model IDs abstract these source-derived identities. |
| `crates/trace-commons-server/src/versioned_pipeline_credit.rs:256` | Repeated NEAR submission on the same key is an explicit provider contract. The model does not prove the provider implements it. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:16758`–`:17112` | Pipeline NEAR stores submit identity and contract across local uncertainty and confirmation. |
| `crates/trace-commons-server/src/payout_pass.rs:1749`, `:1796`, `:1859`, `:1948` | Account rail persists a pending line before send, recovers matching receipts, and excludes receipts already claimed by another line. |
| `crates/trace-commons-server/src/versioned_pipeline_product.rs:740`, `:952`, `:1830` | Snapshot creation/completion/load are tenant/requester scoped. Completion locks submissions for share, queries current export eligibility in a new SQL statement and commits manifest, items and snapshot together. |

P9's duplicate-credit observation counts actual modeled internal commit events,
not merely membership in the already idempotent `ledger` set. Counts saturate at
two: zero means absent, one means committed, and two means a duplicate violation.
Completed-leg transitions and source atomicity are part of the abstraction;
this is not an independent proof of PostgreSQL uniqueness or transaction
isolation. Payout observations compare each operation's first dispatch key with
subsequent send/receipt-query keys and its immutable operation ID. Pipeline NEAR
also observes the stored contract selected at first dispatch. Numerical reuse
across instrument, pipeline NEAR and account-rail namespaces does not alias
operations. Hash collisions, receipt collisions, repeated awards with different
operation IDs, successor-account migration and external exactly-once delivery
are excluded. Existing provider effect sets assume provider idempotency; P9
checks internal commits and identity retention rather than deriving uniqueness
from those sets.

P10 saves the existing completion-time live/permitted-use revalidation result.
P11 saves completion-time requester/tenant authorization and checks every stored
snapshot's items against its immutable tenant. A historical delivered export is
not recalled when a trace is withdrawn later; physical cleanup may also lag.
The model linearizes a successful completion at its existing revalidation and
commit transition. Source database-clock eligibility and submission locks are
the correspondence boundary, not a theorem about elapsed wall-clock time while
a transaction is running. Revocation arriving after delivery cannot retroactively
make delivery unauthorized. Request authorization is auth-derived boundary
input; this does not prove every RLS policy, authenticated principal derivation,
ABAC interpreter or external consumer behavior.

## Bounds and observation noninterference

Settlement has two runs, three lease tokens per run and four instrument legs
(two per run: internal Trace Credit and one external instrument). Pipeline NEAR
has one bounded outbox identity per run. Account payout has two accounts and
one line for each of two streams. Each of the three rails has one independent
in-flight slot. `creditCommits` ranges over `0..2`; first/last key maps range
over the existing identity domain plus the unobserved `-1` sentinel; first
contract ranges over the existing two contracts plus `-1`.

Lifecycle has three immutable objects, two actors, two tenants and two export
IDs. Objects 0 and 1 belong to tenant 0; object 2 belongs to tenant 1. Request
actor and tenant may change independently, including mismatch. One index call
and one invalidation claim may be observed in flight. Saved export observations
are a subset of export IDs and two Boolean maps over those IDs. These are the
existing bounds; adding observers imposes no new state/action constraint.

All observations are fields of the existing `s` record, so direct execution of
named actions, frame extraction and `{ ...s }` preservation use the same state
shape convention. Initial observations are deterministic. Each action retains
all original guards and original-field assignments; new assignments only
compute observations from the current transition, prior observations and its
chosen existing IDs. No guard reads an observation. Erasing the added fields
therefore maps every observed transition to an original transition, and every
original transition can be lifted with its deterministic observation update.
This is a syntactic projection argument, not a machine-checked refinement proof.

## Local validation

Both models pass `quint typecheck` with Quint 0.33.0. Both pass sampled invariant
runs using the TypeScript backend, seed `20261106`, 16 samples and 50 maximum
steps: `--invariant P9` for settlement and `--invariant 'P10 and P11'` for
lifecycle. Sampling can miss counterexamples and does not establish exhaustive
safety. Model/frame control comparisons are checked independently by the flow
validator. TLC execution and result status belong to the integrated proof
evidence; this note does not claim a TLC pass.
