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
| A.6 Credits | Non-transferable and redeemable by the holder only, against that contributor's own NEAR AI inference; may be recomputed or discarded before redemption | The clause now describes a credit that carries value but cannot be sent, sold, or priced. Confirm that framing is right, and that the text is clear that redemption is intended design and not yet available on any deployment. |
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

## Resolved: the withdrawal gap

Clause D.2 originally claimed withdrawal deleted the bytes. It did not: the
pilot bucket had object versioning on with no lifecycle configuration, so a
withdrawn object's superseded generation was archived indefinitely, carrying the
wrapped data key in the same record. The code comment in
`crates/trace-commons-server/src/trace_artifact_gcs.rs` assumed a lifecycle
policy that did not exist.

Fixed on 2026-08-21 by applying to `tc-pilot-artifacts-20260518`:

    lifecycle: Delete, daysSinceNoncurrentTime=1, isLive=false
    versioning_enabled: True        (migration drill + boot check need it)
    soft_delete_seconds: 604800     (7 days, unchanged)

`isLive: false` confines the rule to superseded generations. The only production
caller of the restore path is the object-store migration drill, which restores
its own probe object seconds after deleting it, so a one-day floor does not
break it.

The honest outer bound is therefore roughly ten days, not seven: up to a day
before the generation is eligible, GCS lifecycle lag, then the seven-day
soft-delete buffer. Clauses B.6 and D.2 now say so.

## Corrections made after adversarial review

The first draft asserted several things the code does not do. Each was verified
against source before the text was changed. Counsel is reading the corrected
version; this list exists so nobody re-derives it:

| Was claimed | Actually |
| --- | --- |
| Nothing is sent without per-trace preview | Armed auto-upload and approve-all send traces never previewed |
| Server re-scrub fails a submission that disagrees with the client | Server re-redacts, overwrites the redaction hash, warns, and accepts |
| Model PII filter runs at receipt and fails closed | Runs asynchronously via the backstop driver; `REQUIRE_PRIVACY_FILTER` only checks at boot that a backend is named |
| Invites are single-use | Invites carry a maximum redemption count; the pilot has 2000-use event invites |
| No retention schedule exists | Every submission is stamped with an expiry date; the pruning that would act on it is what is disabled |
| Aggregates suppress cells under two contributors | Suppression counts records, not contributors, and totals are never suppressed |
| Withdrawal deletes everything, checked | Derived summary, embeddings, gate/scoring rows and credit are revoked, not deleted; credit is not clawed back |
| Hash-only logging is enforced in code | True of what we log deliberately; the pilot was emitting trace text via `tokio_postgres` at `RUST_LOG=debug` until 2026-08-21 |

## Open engineering gaps a lawyer should know about

Both are disclosed in the document rather than hidden, and both are worth
closing:

1. **Permitted uses are not re-derived server-side.** Export and evaluation
   filtering gates on the envelope's `allowed_uses` list, which the contributor's
   client computes from the consent scopes. The server does not check that the
   list follows from the recorded scopes, so a modified client could present a
   wider list than its scopes justify. Clause A.5 therefore describes a
   contractual limit, not a technical control, and says so.

2. **Withdrawal revokes rather than deletes derived records.** The canonical
   summary text and vector embeddings survive withdrawal in a revoked state.
   Clause D.2 names them. Deleting them is the better answer and is not yet
   built.
