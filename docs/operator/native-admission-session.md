# Prepare an admission-backed inference session

Native `prepare_admission_session {entry_id, backend, confirmed:true}` registers
an account-bound challenge for the selected session's **next** inference request.
A `ready_for_next_inference` response means registration succeeded. It does not
mean an inference ran, a receipt was verified, or a submission was admitted.
Existing inference calls cannot be retroactively bound.

Required configuration:

- A verified NEAR device enrollment with pinned witness settings and
  `admission_evidence=true`, plus an explicitly selected contribution purpose.
- Separate contributor body-export consent (`ironwire_attested_bodies=true`).
- An explicitly declared local IronWire proxy and its trustworthy control token.
- IronWire capability `supported=true`, `protocol="openai.chat"`, and
  `body_capture_ready=true`. Old proxies missing the capability are refused.
- A configured, funded backend with a compatible OpenAI Chat target and an
  existing agent route through that proxy. Translation to that target is allowed
  where IronWire supports it. Subscription-only or unsupported backend modes
  cannot be used. This operation changes no route, credentials, or capture flag.
- An enforcing Commons/issuer/receipt-service host allowlist and configured
  admission service. A saved `inference_receipt_endpoint` must explicitly name the
  provider's HTTPS receipt-service base URL. Native signup preserves
  `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` when configured; it never derives it
  from a backend name or funds. Existing enrollment configuration must be updated
  explicitly when this endpoint was absent at signup.

The daemon resolves the queue entry back to a declared native source and reads
its exact session identifier from metadata: Codex `session_meta.payload.id` or
Claude Code `sessionId`. An opaque queue ID, rollout filename, or imported
trajectory name is never used as the proxy session key. The running agent must
continue that exact session and preserve its supported session header through
the local route.

The daemon checks consent before any network request, reads proxy capability,
mints a scoped claim, and requests `/v1/admission/challenge`. It checks canonical
encoding, the enrolled account anchor, matching expiry, and a lifetime of at most
15 minutes before authenticated loopback registration. It checks the config,
settings and session metadata again before that write. Only the session ID,
backend ID and canonical binding go to the local proxy; no transcript body
leaves during setup. The remote service receives no session ID or transcript.

The contributor must then continue the selected session. Future routing failure
or an expired binding refuses the inference path rather than silently emitting
an unbound request. The normal witness/preview/submission flow independently
checks evidence, redaction and admission limits afterward. This setup does not
create inference funds or consume an upload entitlement.

The required proxy implementation is in the separate IronWire repository; it
must be built and configured before this capability can become available.

Preparation refuses before contacting the proxy or issuer when that saved endpoint
is absent (`admission_receipt_endpoint_required`) or fails trust validation
(`admission_receipt_endpoint_invalid`). These fixed codes contain no URL or
credentials. This prerequisite applies to preparation for the next bound inference;
it does not add a receipt requirement to ordinary admission-window history uploads.

## Admission policy and provider trust

Admission evidence uses `trace_commons_admission_evidence.v2`. Only Ed25519
receipts are accepted; ECDSA receipt addresses are not gateway attestation keys.
Ordinary redaction certificates and the `tcad1` request binding remain unchanged.
The ingest process requires all three settings below whenever admission is enabled:

- `TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS`: comma-separated canonical lowercase
  64-hex Ed25519 gateway public keys, without `0x`.
- `TRACE_COMMONS_ADMISSION_ACCEPTED_MODELS`: nonempty comma-separated exact model IDs.
- `TRACE_COMMONS_ADMISSION_MIN_REQUEST_BYTES`: positive integer floor on the exact
  final request body in bytes, including metadata. No default economic threshold.

The admission witness requires the corresponding settings with prefix
`TRACE_COMMONS_WITNESS_ADMISSION_`. A missing or invalid model/floor refuses startup
when provider signers enable that route. Its ordinary route remains independent.
The witness verifies the explicit receipt algorithm, signature, exact request and
response hashes, model membership and size floor before signing evidence. Ingest
independently checks the signed model and size against its configured policy.

Operators must establish and refresh the gateway keys' attestation provenance
through their deployment trust process. A configured key pin is **not** live TDX
quote verification. The contributor report/key association check alone is also
not a complete quote/collateral verifier. Do not describe a syntactically valid
self-reported gateway key as attested. The signup witness pin is obtained from
the configured HTTPS ingest origin: that origin is a trust-on-first-use authority,
not an independent witness attestation source.

Model membership and request byte length are eligibility controls, not proof of
price paid, token consumption, output quality or an unpaddable economic floor.
Choose thresholds using measured provider behavior; no conversion, sponsorship
or per-call cost is inferred by this implementation. A free window is per verified
account anchor, not per human: another controlled NEAR account can receive another
window. A dust-funded implicit account may suffice if it meets the chain/provider
requirements; no meaningful minimum spend or one-person barrier is assumed.
Account creation/control costs are external, variable and not quantified
here. Size the separate global processing cap assuming multiple accounts per actor.

Native account-session bearer tokens authorize account operations; they are not
upload claims. Uploads use a registered device's signed workload JWT, then issuer
scope-ceiling intersection and a signed upload claim with nonempty consent scopes
and allowed uses. NEAR identity exempts the invite grant, not those scope checks.

The client's no-downgrade rule applies to a selected bound request and its retries.
An invalid evidence request rejected before SQL reservation has **no ledger row**;
a caller can submit that identifier again without evidence and request an ordinary
window slot. The server still charges a bounded window reservation before processing.
Once reserved, changed evidence conflicts with the recorded request identity.
Neither statement means admission bounds witness work before ingest reservation.

## Migration, ingest and retention roles

Run migrations as a dedicated schema owner. Run ingest as a different
`NOSUPERUSER NOBYPASSRLS` role that owns no admission tables and belongs to neither
`trace_admission_guard` nor `trace_onboarding_retention_guard`. Both function-owner
roles are `NOLOGIN NOBYPASSRLS`. V59/V60 temporarily grant their membership for
ownership transfer, grant the migrator explicit `EXECUTE WITH GRANT OPTION`, then
revoke membership. The retained grant option permits runtime provisioning without
restoring function-owner membership. The schema owner remains an administrative
trust principal with DDL powers; revocation cannot remove those inherent powers.

As the migrator, substitute the deployment's runtime role for `commons_runtime`:

```sql
GRANT USAGE ON SCHEMA public TO commons_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_admission_challenges,
  trace_admission_accounts, trace_admission_submissions TO commons_runtime;
GRANT EXECUTE ON FUNCTION
  trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),
  trace_transition_admission(TEXT,UUID,UUID,TEXT)
  TO commons_runtime;
GRANT EXECUTE ON FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN)
  TO commons_runtime;
```

Keep the normal tenant-scoped application grants required by provisioning and
submission storage. Never grant direct access to admission receipt/global-budget
tables, function ownership, or either guard membership to the runtime role.
Admission startup checks the restricted runtime role and required ledger grants.
Deployments that ran the earlier unrevoked V59 must explicitly revoke the old
membership after preserving the execute grant options; changing an already-recorded
migration file does not rerun it. V60 also repairs the V59 function grants/revoke for
those deployments before installing cleanup.

The existing authenticated retention worker runs bounded onboarding cleanup when
native provisioning or admission is configured, respecting its `dry_run` setting.
Each call considers at most 1000 expired records: globally scoped pre-account
ceremonies and then expired challenges in the authenticated worker's tenant.
Schedule the worker for each active admission tenant to cover its challenges.
Expired consumed challenges may be removed; account counters, submission identities,
terminal receipts, the global receipt dedup set and global budget are retained.
Cleanup cannot restore a window or replay an old receipt. Missing migration or
function-execute grants cause a fixed-label worker failure, never silent success.
