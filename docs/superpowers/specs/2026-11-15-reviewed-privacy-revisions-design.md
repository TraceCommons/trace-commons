# Reviewed privacy revisions

## Intent

Save corrected versions of privacy-held pipeline submissions, review the exact saved content, and resume existing corpus indexing and eligible contributor credit. The operator approved building this after a 32-run residual scan and a classifier pilot found seven additional redactions.

## Chosen approach

Add immutable recheck candidates linked to the existing run and submission. Keep original source and privacy-pass artifacts intact. A candidate chain starts from the original recorded pass and later rechecks start from the latest candidate, so earlier redactions are retained. An existing reviewer claim authorizes preparation; an explicit candidate ID and hash bind the eventual assessment. This reuses existing Review/Score/Settle and credit source keys. Resetting pass columns loses evidence; new submissions duplicate ownership/credit boundaries; both are excluded.

## Protocol coverage

A new review-only classifier API covers all events plus remaining content-bearing side fields under AllEvents. Existing ordinary scrub behavior remains its own policy. Work on a clone and publish changes atomically only after classifier calls succeed. Preserve human correction text under S5; classify a copy and mark unresolved if a finding would rewrite it. Inspect dynamic keys without rewriting them; findings remain unresolved. Evidence records fixed policy, basis, complete coverage, correction status, residual scan completion and useful classifier result. Never reset a prior High rating. Manual approval may resolve a prior High only with complete AllEvents evidence, no unresolved correction or key, and a completed clean residual scan.

## Bounded manual processing

The ordinary classifier's 4,000-node, 400,000-byte aggregate and 32,000-byte field limits cannot cover the target queue: an on-host shape-only census found all 32 held traces exceed at least one ordinary limit. Maxima were 63,111 structured nodes, 2,738,697 structured string bytes, a 47,069-byte structured field, depth 10, and a 2,325,232-byte event text field; no content was exported by this census.

Use a separate fixed manual-review profile: 32 MiB aggregate classifier-input bytes, 32 MiB field cap, 4,194,304 nodes, depth 24. Share one budget across event prose, structured content, side prose and correction inspection. Do not reset it per event. Existing source size/residual scan limits, adapter windows/transport input caps, one process-wide recheck permit and the classifier/live-claim deadline remain enforced. Ordinary upload budgets do not change. A field or total exceeding the manual profile remains held. Local deterministic protection covers the expanded range before external calls. Locally detected credentials in original keys or immutable corrections stop those values from reaching the classifier; other correction text may be classified on a copy and is never overwritten.

## Durable candidates

V159 adds immutable tenant-scoped pipeline_review_rechecks rows, source/output refs and hashes, parent/ordinal, evidence, creator and timestamps, and optional candidate binding on Review assessments. FORCE RLS and runtime least-privilege grants apply. Per-run locks allocate ordinals and compare expected source to the current latest ready candidate. Same recheck_id returns the same committed result; conflicting reuse refuses.

Stage encrypted candidate objects under a dedicated review-recheck artifact kind using the real reviewer claim, not a fabricated worker lease. Existing staged-artifact cleanup and submission withdrawal apply. Finalize object ref/candidate/staging atomically after revalidating claim, liveness and source. Expired claims, changed sources, withdrawals or publication failures cannot leave an approvable candidate.

## API and decisions

POST /v1/review/pipeline/runs/{run_id}/recheck accepts lease_token, recheck_id and expected_source_hash only. Require reviewer/admin, tenant and current consent/ABAC before classification. Reauthenticate the same request after external I/O to observe expired credentials, revoked access grants and current role, then reapply current consent/ABAC before final commit or content readback. Classifier work holds no DB transaction. Restrict creation to parked unassessed Review runs. Metadata bodies use a 1 MiB cap and reject unknown fields. Responses contain hashes, fixed enums and counts, never plaintext.

Existing assessment gains optional candidate_id and candidate_content_hash as a pair. Once a candidate exists, both approval and rejection must name the current latest candidate; legacy unbound decisions refuse. Score holds remain separate and reject privacy candidate parameters. Assessment evidence binds the candidate identity/hash. Approval verifies complete safe evidence and all hold reasons.

Worker Review loads only the candidate bound to its assessment, verifies integrity/operability, and records that candidate as the derived input. Its approved revision then follows ordinary scoring/indexing/settlement. Retries retain run/submission IDs and existing idempotency keys. No direct credit edits or automatic approval. Reviewer content reads, if exposed, require live own claim, current ABAC and a successful content-read audit before returning exact candidate bytes.

## Verification

Pin source immutability, tenant/role/ABAC denial, expired/stolen claims, idempotent retry/conflicting IDs, concurrent source changes, candidate hash mismatch, refusal of unbound decisions, Score separation, failed publication/sweep, withdrawal at each boundary, exact corrected corpus bytes, and one credit event under retries/restart. No new dependencies or license boundary changes.

## Execution boundary

This build does not deploy a server, apply a production migration, prepare a
production candidate, record an assessment, or issue credit. The preceding
investigation completed host-only residual-secret scans for 32 selected passes.
External classification was explicitly approved and performed only for the
smallest pass; that diagnostic did not save its changed content. Approval for
that pilot does not cover external processing of the remaining 31 passes.
Final branch validation and review are tracked in the implementation plan.
