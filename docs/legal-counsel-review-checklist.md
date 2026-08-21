# Counsel review checklist — tracecommons.ai/legal

The document at `/legal/` (source: `src/components/LegalBody.astro` in
`trace-commons-community`) is a draft and carries a banner saying so. It was
written by transcribing system behaviour from code and live configuration, then
drafting clauses around it. Someone who is a lawyer needs to read it before the
banner comes off.

This list is what to look at hardest. It separates the two kinds of statement in
the document, because they fail in different ways: a wrong **fact** is an
engineering bug and I can fix it; a wrong **judgment** is a legal decision and I
should not have made it alone.

## Judgment calls a lawyer should confirm or overrule

| Clause | The call I made | Why it might be wrong |
| --- | --- | --- |
| A.2 Eligibility | 18 or older | Chosen to avoid COPPA and GDPR child-consent machinery entirely. Lower floors are defensible and may matter for a student-heavy hackathon audience. Nothing in the codebase enforces any age check at all — the clause is currently a promise made by prose alone. |
| A.5 Licence grant | Non-exclusive, scope-limited, revocable, no ownership transfer | Deliberately narrow. Confirm it is broad enough to cover what the corpus is actually for, including publishing derived datasets. |
| A.6 Credits | No entitlement, no value, may be recomputed or discarded | Drafted to avoid any securities or stored-value reading. Confirm the language survives a regulator reading it after credits become transferable. |
| A.8 Liability cap | US$100 aggregate | A conventional floor for a free pilot service. No basis beyond convention. |
| A.10 Amendments | Prospective only, governed by the `policy_version` in the envelope | Unusual and strong: the software genuinely records which version each submission was made under. Confirm that binding ourselves this way is wanted, because it is harder to walk back than a standard "we may change these terms" clause. |
| A.11 Governing law | California, Santa Clara County | Follows the entity's operating location. |
| B.4 Sub-processors | Named in the document, changes published by amendment | Confirm whether a DPA is required with either processor before contributors in the EU or UK are onboarded. |
| C Scope definitions | The five scopes as the enum defines them | Confirm the plain-language descriptions do not narrow or widen what the clause permits. |

## Facts, each traceable — flag any that read wrongly and I will correct the text

| Statement | Verified against |
| --- | --- |
| Single-use invites; device private key never leaves the machine | Enrolment path; `/v1/onboard` is not idempotent (see the Homebrew cask's deliberate zap exclusion in `docs/release-runbook.md`) |
| Local redaction before upload; server re-applies it | Contributor client redaction pipeline + server-side re-application |
| Privacy filter is mandatory and fails closed | `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1` on the pilot |
| Privacy filter and scorer both run at NEAR AI | `TRACE_PRIVACY_FILTER_BACKEND=near-ai`; `TRACE_COMMONS_NEAR_AI_BASE_URL=https://qwen3-6-27b.completions.near.ai/v1` |
| Medium residual-PII risk accepted on this deployment | `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS=true` (systemd drop-in) |
| Per-artifact data key wrapped by Google Cloud KMS | `TRACE_COMMONS_KEK_PROVIDER=gcp_cloud_kms`; `EncryptedTraceArtifact.wrapped_dek` |
| Hosting, storage, KMS in us-central1 | Bucket `tc-pilot-artifacts-20260518`, location US-CENTRAL1 |
| Hash-only audit and logging | Repo convention, enforced in code |
| No retention or pruning schedule configured | No `TRACE_COMMONS_RETENTION_*` variable set on the pilot |
| Deleted objects purged from soft-delete after 7 days | Bucket `softDeletePolicy.retentionDurationSeconds = 604800` |
| Aggregates: minimum cell of 2, suppression only, no noise | `COMMUNITY_MIN_CELL_COUNT_FLOOR = 2`; `TRACE_COMMONS_COMMUNITY_ANALYTICS_PUBLICATION_BASIS=suppression_only`; noise seed is the placeholder `v1:no_noise_yet`, refused on recompute and serve |
| Withdrawal deletes artifact, file record, object refs, status-derived paths, and errors propagate | `delete_withdrawn_trace_objects`, `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` |
| Snapshots refuse to serve when older than 15 minutes | `COMMUNITY_SNAPSHOT_MAX_AGE = 900` seconds |
| Credits compute but do not settle | `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled` |

## Blocking on an infrastructure change, not on counsel

**Clause D.2 is not fully true today.** The bucket has object versioning enabled
and **no lifecycle configuration**, so when `delete_withdrawn_trace_objects`
deletes an object, the previous generation is archived and retained
indefinitely. The code comments assume otherwise:

> Bucket-level GCS object versioning + lifecycle policies handle the
> `deleted_at` timestamp in production
> — `crates/trace-commons-server/src/trace_artifact_gcs.rs`

The lifecycle policy that comment relies on does not exist on
`tc-pilot-artifacts-20260518`. The archived generation carries the wrapped data
key inside the same stored record, so it remains decryptable by anyone with
bucket read and KMS decrypt.

Do not publish part D until this is fixed. The fix is a lifecycle rule deleting
noncurrent versions, after which the seven-day soft-delete window is the real
outer bound and the clause is accurate as written. This is an operator decision
about production data, so it is not made here.
