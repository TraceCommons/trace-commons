# Staging earned upload trust

## Approved behavior

The user approved this progression, based on distinct novel, accepted traces:

| Qualified traces | Uploads/day | Bytes/day |
| --- | --- | --- |
| 0 | 50 | 209715200 |
| 5 | 100 | 419430400 |
| 20 | 150 | 629145600 |
| 50 | 250 | 1048576000 |

There are no account-age or active-week prerequisites. Each eligible submission counts once; duplicate/re-upload contributions and withdrawn, revoked or quarantined traces do not earn additional trust. Existing eligible history counts. Contributor-chosen limits and today's counters remain intact.

## Integration

Staging deliberately admits provisioned accounts without the admission ledger. Keep that flow. Introduce an optional independent `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON` and matching `_VERSION`. It uses the existing tiered evaluator and generation/freshness-validated applied evaluations, but changes only advertised client caps. It does not turn on either admission ledger, require a witness, or change signup, consent, review, credit or payout.

The independent policy is mutually exclusive with either account-admission or legacy-admission configuration. Preserve the existing admission-backed trust behavior when the independent policy is absent. Store both parsed GrowthPolicy and BoundedPolicy so the worker's rule and the reader's validated metadata agree. The shared scheduler evaluates the independent policy through its existing `applied` mode, recorder and scoped token. Startup requires durable forced-RLS database controls, external-growth invalidation/read readiness and tiered writer/recorder readiness; missing controls fail closed. Enabling invalidation may invalidate old evaluations; record and evaluate afresh afterwards.

The endpoint resolves the authenticated account exactly as before and reads `account_admission_status` against the independent bounded policy for its validated effective allowance. This advisory read does not enforce admission or reserve/debit a processing budget. Missing/stale/wrong-generation/shadow evaluations return the base cap or null; read failures return an error. The contribution-status route continues to describe the existing admission path.

## Staging policy

Use multipliers 1, 2, 3 and 5 at 0, 5, 20 and 50 units. Set age/active-week/attested-unit requirements to zero, attested unit weight to one, and weekly cap high enough (10000) that it cannot impose weeks of waiting for these tiers. Retain the existing 84-day qualification window and two-day evaluation freshness bound. Scheduler cadence is five minutes. These duration choices are implementation defaults; the approved contribution thresholds and cap values above are the product requirements.

Add an optional `require_gate_pass` growth-policy flag, default false to preserve existing policies; staging sets true. Even attested traces must pass the allowlisted novelty/quality gate and be first in their dedup cluster for this policy. No attestation bonus. Gate and dedup version allowlists must be based on the live staging decisions, not copied from pilot defaults. No change to scoring or novelty thresholds.

## Delivery and verification

Check in a staging-only policy template, env rendering and operator instructions. Keep secrets out of source. A dedicated `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL` may supply the narrow nonowner trust connection for both shadow and applied operation; its endpoint and database must match the source pool, and it runs no migrations. The source-ingest/migrator role remains unchanged. Verify pure boundary cases 0/4/5/19/20/49/50, same-week fresh accounts, duplicates/reuploads, netted-out outcomes, and attested-without-gate exclusion. Verify independent startup rejects malformed/conflicting policies and missing controls, scheduler can evaluate with both admission ledgers off, and real PostgreSQL endpoint tests show the approved caps without changing admission/spend/signup. Re-run client lifecycle/counter regressions, licensing, formatting and relevant Clippy once at the end under fixed Rust flags. Prepare staging readiness and rollout artifacts before any deployment; do not claim live behavior from local tests.


## Evidence-driven source-contract repair

The live staging audit found eligible accepted submissions attributed through verified device upload aliases already covered by V135's authoritative `trace_account_owned_principals` view. Legacy trust functions still use direct principals and require monetary `accepted` credit events, while these valid submissions have newer utility credit events. No attribution or historical credit may be invented to bridge that gap.

Add V137 to use verified owned principals consistently and to record an `accepted_submission_status` source from actual accepted submission state. Only policies with `require_gate_pass=true` may count that additional accepted source; old credit-qualified `accepted_submission` behavior remains intact. Deduplicate both source kinds by submission UUID. Require the existing allowlisted passed gate and first-in-cluster tests. Preserve adverse/netting ownership after unlink/revocation using durable previously verified facts so contributors cannot shed withdrawals by unlinking. Current gate results and submission/alias changes must invalidate evaluations. This necessary migration supersedes the earlier no-migration assumption; it does not rewrite historical submissions or credit.


## Compatibility and ownership serialization

Self-reviewed and unknown-approver status outcomes cannot be overridden by an older credit-qualified source for the same UUID. Routine body expiry or purge must preserve already-verified qualifying history; it cannot create new acceptance facts or promote failed gates. Withdrawal, revocation, quarantine, inactive ownership, and current failed gates remain exclusions. Gate-required policy inputs must apply these exclusions even to legacy-only history without a stored status fact; default-false policies retain their original source semantics.

Alias and device invalidation must discover old/new owning accounts independently of existing facts, then take one globally sorted frontier-lock sequence. This serializes first-time recording/evaluation with concurrent ownership changes without creating a reverse lock order. V137 must also invalidate pre-migration applied evaluations on already-enabled databases after installing its changed input projection; advancing generation is control metadata, not a rewrite of contribution or credit history.
