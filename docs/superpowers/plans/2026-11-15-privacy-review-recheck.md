# Privacy Review Recheck Implementation Plan

> **For agentic workers:** Use inline execution or the assigned independent agent for the scoped diagnostic.

**Goal:** Produce fresh privacy evidence for the 32 held production passes without modifying them.

**Architecture:** A standalone protocol example streams bounded, integrity-checked JSONL; a host-only reader verifies/decrypts stored artifacts and pipes their exact bytes to it.

**Tech Stack:** Existing Rust protocol APIs, Python host reader, GCS and KMS existing permissions. No new dependencies.

**Spec:** ../specs/2026-11-15-privacy-review-recheck-design.md

## Global constraints

- Preserve original evidence, risk and review state. No automated approval.
- No raw trace or authentication material in diagnostic output.
- Preserve dirty primary checkout; isolated worktree only.
- RUSTFLAGS=-D warnings, CARGO_INCREMENTAL=0, SCCACHE_SERVER_PORT=4227.

## Review focus

- Wrong hash or tenant must never be scanned as the selected pass.
- Scan failure must remain distinct from no findings.
- A clean scan must not imply approval or a risk downgrade.
- Content changes outside events must count as changed.
- Invalid input errors must not echo supplied content.

## Completed diagnostic work

- [x] Add the standalone diagnostic and focused regression tests without changing ordinary privacy policy.
- [x] Validate residual scanning, input integrity and output confidentiality; build the native and Linux diagnostics.
- [x] Independently review the bounded on-host reader and diagnostic.
- [x] Verify and recheck the 32 original stored passes without changing them.
- [x] Run the explicitly authorized external-classifier pilot on the smallest pass.
- [x] Record results separately from saved-version reviews, credit and deployment.

## Results and authorization scope

All 32 selected passes completed raw-JSON residual-secret scans in 38 seconds,
with no secret-pattern findings or mutations. This is detector evidence, not a
complete privacy assessment or an approval.

The operator explicitly approved external classification of the smallest pass
at `https://cloud-api.near.ai`. That pilot completed in 21 seconds under the
ordinary prose-only policy. It added seven redaction annotations: five private
date, one account-number and one secret labels. These are classifier findings,
not proof that every match is sensitive. Content changed and risk stayed High.
Only hashes, labels and counts were retained; no corrected artifact, assessment
or credit was recorded.

The remaining 31 passes were not sent for fresh external classification by this
diagnostic. Their external processing requires approval covering those payloads.
Building the saved-version feature does not execute that batch or deploy it.

The durable implementation and its final validation are tracked in
[Reviewed Privacy Revisions](./2026-11-15-reviewed-privacy-revisions.md). The
standalone diagnostic remains read-only and never creates approval eligibility.
