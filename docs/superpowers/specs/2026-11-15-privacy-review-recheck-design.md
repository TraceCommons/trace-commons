# Privacy review recheck

The operator requested a fresh recheck of 32 production pipeline runs held for high privacy risk. Preserve original pass artifacts and review decisions. Produce fresh evidence; never equate a scan with approval.

## Approach

Use a standalone protocol diagnostic, invoked on verified stored pass bytes on the host. This avoids a service deploy and retains the once-per-run pass invariant. Saving corrected versions belongs to the separate durable-version feature linked below; resetting pass columns would destroy evidence and is excluded.

Inputs are bounded JSONL records containing a run reference, exact expected SHA-256, and envelope JSON. Credentials stay on the host. Output is hashes, fixed labels, counts and hashed residual locations and count labels, never trace contents or secrets. Each input is processed independently, sequentially.

First run a detection-only residual scan on the exact original JSON value, including legacy or unknown fields, with the existing per-leaf scanner and depth/node bounds. Do not round-trip through the current typed serializer for this scan. Full classifier mode still refuses unsupported typed fields. This supplies fresh evidence about whether the previous unavailable scan can now complete; it does not lower High or authorize approval. Full mode uses the current configured classifier and existing protocol scrub/risk rules on a clone. The diagnostic defaults to prose-only; all-events requires an explicit option. Reuse one adapter across the batch. Pilot full classification on the smallest trace before a larger batch.

Any error, incomplete coverage, High result or changed content remains for review. New redactions cannot justify approving the old stored artifact. Every result explicitly declines automatic approval. No DB writes, withdrawals, credit changes, service restart or environment changes are part of the diagnostic.

Validation covers malformed/hash-mismatched input, survivors, clean prior High, sidechannel changes, missing classifier and output confidentiality. Run targeted protocol regression tests and review the diagnostic before production execution.

## Relationship to saved versions

This document describes the inspection-only diagnostic. The saved-version API,
manual-review coverage profile and exact candidate-bound assessments are defined
in [Reviewed privacy revisions](./2026-11-15-reviewed-privacy-revisions-design.md).
The diagnostic's classifier mode uses ordinary prose-pass limits; it does not
create complete manual-review evidence or a stored candidate. Only the smallest
pass's external classifier pilot was authorized in the preceding investigation.
This build does not execute an external batch or perform a production rollout.
