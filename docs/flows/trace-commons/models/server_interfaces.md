# Other service interfaces: explicit coarse coverage

Baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. This model provides named,
observable service boundaries for the route families below. It does **not** claim
that their algorithms, queues, storage transactions or lifecycle invariants are
fully modeled. No promises/properties/TLC checks are encoded. Untracked primary
checkout files are excluded.

## Boundary contract and finite state

Two actors, tenants, request keys, access grants and token bundles are represented.
`family` ranges over the literal finite `Families` set. One ingest request is
observed at a time; witness requests use a separate two-ID pending set and two
work permits. `requestGeneration` and `currentGeneration` each range over
`{0,1}` to distinguish one request from an account-change/disconnect boundary;
there is no unbounded generation counter and no claim that this is an actual
server cancellation guard. The server request retains the actor/tenant it began
with, while the caller may leave.

The environment supplies `authenticated`, `familyCredential`, `roleAllowed`,
`policyAllowed`, `requestValid`, feature enablement and control availability.
These are route-specific validation **outcomes**, not an implementation or proof
of each route's authentication rules. `familyCredential` means the credential
kind accepted by that specific entry point, including an account session where
appropriate; it never means a utility worker credential authorizes a benchmark
route. Protected acceptance requires the applicable evidence to be true. Public
catalogs/rosters/estimates are explicitly separate families from authenticated
account/profile operations. `ReadOnly` prevents public catalogs from taking the
generic mutation action. Anonymous usage events intentionally can have a stored
effect. `public_source` has its own unconditional source-offer response, outside
feature/config/account/tenant gates.

`stage` distinguishes request received, authorized, effect pending, effect
committed, returned, refused, failed and response lost. `effects` stores at most
`Families × tenants × keys` opaque effect identities. This records that an
authorized handler did work; it does **not** assert generic exactly-once
semantics. Reusing an opaque key in this set cannot prove a real route is
idempotent. `coarseOutcome` records only the last coarse family outcome.
There is no invented universal job queue or worker lease across these families.

## Route-family mapping

`I` below is `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs`.
Internal module anchors beginning `trace_commons_ingest_internal/` are relative to
`crates/trace-commons-ingest/src/bin/`. The source anchors and literal route
inventory should be read together. Source
status/refusal details are collapsed to `route_specific_refusal` or
`service_or_store_unavailable`; their HTTP codes are not modeled.

| Family / concrete source anchor | Observable state and actions | Explicitly coarse/omitted |
| --- | --- | --- |
| `legacy_gate`; I:9685 (`/v1/workers/gate/evaluate`), handler I:60479 | Authorized gate invocation through `TheServiceStartsItsCoarseWork`, read/effect/failure result in `coarseOutcome`. | Legacy scoring/redaction, file/DB mirrors, duplicate decisions and gate internals. Versioned processing is owned by `server_pipeline`. |
| `replay_export`; I:9249, :9275, :9283 through :9298 (`datasets/replay`, manifests, replay worker, export-job claim/run/retry); I:45092 | Opaque selected/exported result or refusal/effect committed before response loss. | Replay selection, exported artifacts, grants and queued job lease/retry algorithms. Detailed versioned snapshot lifecycle is in `server_lifecycle`. |
| `benchmark`; I:9254 through :9274, :9600 vicinity (conversion, evaluation, registry publication/outbox submit/confirm/mark-status); I:55284 | Separate `benchmark` outcome/effect slot; external/store failure before or after effect. | Conversion/evaluation quality, benchmark registry outbox details, lifecycle authorization and recovery. |
| `ranker`; I:9299 through :9315, :9585 through :9665, :9715 through :9745; I:55877, :56246 | Named ranker reads and opaque accepted worker/admin effects. | Features, training pairs, preference labels, calibration, promotions, backtests, credit policy, worker leases and recovery. |
| `active_learning`; I:9197, handler I:56549 | Reviewer queue read with protected boundary evidence; no mutation inferred from this read-only family. | Ranking priority, query privacy/lease filters, review decision transitions (owned by `server_pipeline` for versioned). |
| `utility`; I:9689, :9690, handlers I:28712, :28835 | Utility-credential evidence, finite/valid request evidence and coarse credit/attestation effect. | Delayed-credit formulas, exact cap arithmetic, external-ref dedup, issuer checks and signature primitives. |
| `vector`; I:9667, :9680, handler I:61000 | Vector-specific credential/control/config evidence and opaque index effect/failure. | Legacy vector batching, embedder/search contracts and ANN internals; versioned physical write/removal is in `server_lifecycle`. |
| `access_grant`; I:9317 through :9330, :9341; handler I:14896 | `YouCreateAnAccessGrant`, `YouRevokeAnAccessGrant`; grant key and tenant retained independently. | Principal/scopes/uses policy interpreter, actual ABAC and every consumer revalidation. Grant state is not silently substituted for route-specific `policyAllowed`. |
| `community_profile`; I:8660, :18267, :18327 | Authenticated attribution-scope evidence; profile private/public/withdrawn; snapshot dirtiness and withdrawal invalidation distinct. | Handle/bio validation, uniqueness conflicts, audit tails, eviction receipts and coalesced scheduler internals. |
| `community_read`; I:8664 through :8679, :18091, :18171, :18229, :18244 | Public roster/run/analytics families remain public. `TheCommunityRosterReadRechecksAnalytics` withholds analytics unless current controls and old snapshot content permit it. Stale/pending invalidation refuses the specialized read; newly enabled analytics can need recomputation. | Aggregate cell suppression/noise quality, detailed freshness timestamps and public-run artifact selection. Generic community reads are opaque successful/failing boundary observations, not a bypass proof of specialized gates. |
| `reward_catalog`; I:8867 through :8873; `trace_commons_ingest_internal/rewards.rs:185`, :200, :218 | Anonymous offer/catalog/mission publication reads with valid/config evidence. | Budget/offer availability, suppression of closed/suspended detail and exact rate-slot arithmetic. |
| `reward_account`; `trace_commons_ingest_internal/rewards.rs:43`, :236, :269, :284 | Protected reservation/status/history result or opaque authorized reservation effect. | Budget allocation, lock serialization, reservation idempotency, participant DB login and reward calculation. |
| `activity_catalog` versus `activity_account`; I:8869, :8745; `trace_commons_ingest_internal/activity_missions.rs:49`, :65 | Public catalogue is separate from AccountCtx-authenticated progress/status. Both are read-only in this model. | Date windows, mission activity evaluation, account progress and exact rate limits. No anonymous account progress or mutation is inferred. |
| `token_bundle`, `token_research`; I:8874 through :8917; `trace_commons_ingest_internal/token_bundles.rs:105`, :162, :251, :298, :437, :465, :519, :746 | Manifest staging, attachment ready, accepted underlying submission, durable receipt, post-submit rescrub/store failure and revoke/expiry milestones. `TheTokenBundleFinalizeAcceptsTheSubmission` does not issue a durable bundle receipt. Protected research reads use separate family. | Attachment bytes/digests, certificate verification, query authorization details, rescrub algorithm, processing loop, exact 24-hour staging/30-day retention and per-body memory/admission budgets. Attachments-ready is an already-authorized route-result boundary. |
| `analytics`; I:9195, :9397, :9401; `public_usage` I:8949 through :8966 | Protected analytics reports versus explicitly anonymous region/events and their validated allowlisted coarse effect. | Aggregation math, privacy mechanism, geolocation inference, weekly totals and schema validation. |
| `drill`; I:9367 through :9445, :9540, :9651 | Authorized drill returns opaque/hash-only evidence or failure in its named family. | Every drill's backend behavior, rollout-smoke promotion/readiness assertions, key rotation and repair transactions. A successful coarse response proves none of those claims. |
| `process_evaluation`; I:9747 through :9753 | Own protected worker boundary and opaque effect. | Evaluation quality, run queue and retry scheduling. |
| `credit_admin`; I:9480 through :9584, :9694 through :9713 | Legacy credit cycle/settlement/hold/attestation/outbox controls remain an explicit coarse family. | Legacy settlement batches and manual outbox status reconciliation. Detailed pipeline NEAR and separate account NEAR AI payout behavior is in `server_settlement`. |
| `account_trust`; I:9465 through :9489 | Protected fact record/evaluation/explain/drill boundary. | Trust facts, growth/risk policy, starter advances and administrative review internals. |
| `inference_account`; `trace_commons_ingest_internal/inference_connection.rs`; account router I:8687 onward | Authenticated inference connection/funding/account result or effect; separate family from witness. | Provider funding/credentials, org policy, connector request bodies and model inference quality. Client orchestration belongs to client service models. |
| `audit`; I:9755 | Protected audit read result. | Audit chain validation/repair, log hashing and tenant RLS internals. |
| `public_source`; I:8921, :13661 | `ThePublicSourceOfferReturnsWithoutTenantAuthority` returns source license/URL/build metadata without auth, tenant or feature gates. | Build metadata correctness and external source host availability. |
| `public_stats`, `public_estimate`, `collateral`; I:8924 through :8974, :9092, :87935 | Anonymous register stats, estimate tables/first-run copy and attestation collateral/readiness boundaries. | Refresh jobs, table calculations, nonce/quote verification and cache freshness. Feature/control absence can still refuse the applicable endpoint; source offer is distinct. |

## Separate witness HTTP surface

Source: `crates/trace-commons-server/src/witness_service/http.rs:94` registers
`POST /v1/witness`, `/v1/witness/token-bundle`, `/v1/witness/admission`, then
`GET /v1/attestation` after the work-load middleware. These entry points are
unauthenticated. `surface.rs:243`, `:264`, `:284`, `:340`, `:387` expose the
narrow service operations; `inference.rs:707` and `:742` validate inference
attestation; `token_bundle.rs:51` owns bundle witness work.

`TheWorldRequestsWitnessEvidence` chooses two bounded request identities and
one route-kind. Non-attestation work consumes one of two work permits; the
attestation quote route bypasses that semaphore, matching router placement.
`TheWitnessReturnsValidatedEvidence` records a quote only after nonce-valid
evidence for attestation, or a certificate after applicable contribution/
inference validation. `TheWitnessRejectsEvidenceOrTimesOut` is the coarse
classifier/signer/evidence/unavailable failure boundary. Saturated work is refused
by `TheWitnessLoadBoundRefusesAnotherRequest`.

Nonce shape, evidence validity and applicable policy results are abstract
booleans. This does not prove cryptography, enclave identity, redaction quality,
non-correlation, network isolation, inference correspondence, signer pin rotation,
certificate lifetime, or request/body size limits. The two observed request IDs
bound all pending requests, while actual attestation concurrency is outside the
work permit bound. No tenant/account identity is introduced into witness work.

## Composition events and limitations

The generic shared boundary is `TheWorldSendsAServiceRequest` with fixed
`family`, actor, tenant and key, followed by one route-specific accepted/refused
boundary. A `TheServiceCommitsACoarseDomainEffect` event is an opaque effect, not
a durable contribution receipt or payment. `TheResponseIsLostAfterTheEffect`
keeps the effect while losing delivery. Token-bundle receipt has its own
`TheTokenBundleDurableReceiptCommits` event. Profile and grant events retain
separate projected state; community invalidation is a later public-snapshot
boundary. Witness success supplies validated boundary evidence to contribution
and admission models, not account authorization.

Detailed replay/benchmark/ranker/utility/vector/grant/reward logic remains a
visible modeling gap beyond these boundaries. Source-specific retries,
conflicts, leases, arithmetic, multi-row atomicity, concurrent requests and
configured runtime enablement must be expanded before selecting a promise
about those internals. Source-specific protected evidence is intentionally
external, so no authorization guarantee can be proved from this module alone.
All server frames are explanation panels, not new application UI.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
