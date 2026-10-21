# The Insights judgement service -- design

Date: 2026-10-20. Owner: Zaki. Status: draft, nothing built.

This is the service spec that delivery step 3 of
[`2026-10-20-insights-private-judgements-addendum.md`](2026-10-20-insights-private-judgements-addendum.md)
requires before any service code. It covers step 4 (the account-bound limiter)
too, because the limiter is the service's only state. Read the addendum first;
this document does not repeat its owner decisions, its digest contents or its
consent principles. It follows the shape of
[`2026-09-02-redaction-witness-service-design.md`](2026-09-02-redaction-witness-service-design.md)
and [`2026-09-02-witness-client-design.md`](2026-09-02-witness-client-design.md),
and reuses their code wherever it can.

## Owner decisions (2026-10-20, answering section 12)

1. **Interim production model: none.** Production waits for the TEE-hosted
   decision model on NEAR AI. Until that model is in the catalog, returns a
   valid per-request `provider_tee` receipt and passes section 5, the
   production allowlist is empty and the production service refuses to
   start. Slices 1-6 (protocol through the development deploy, with the mock
   and Jev development adapters) proceed now. Slice 9, the production deploy,
   waits for the model.
2. **Production pins are compiled into the app.** A service upgrade pauses
   judgements until a daemon release, which is accepted. There is no signed
   measurement list.
3. **Gateway attestation is verified at each service boot,** pinning the
   quote-bound receipt keys. Every request's receipt is still verified.
4. **The operator-wide monthly budget is $100,** in the measured compose. It
   sits above the $5 per account per month cap.
5. **Pooled-tenant accounts stay excluded in v1** (the section 3 default).
6. **Phala named-volume survival across an in-place upgrade:** this is a
   fact to verify before slice 6, not a choice. If the volume does not
   survive, section 6's ledger placement changes, and the owner is told
   before deploying.

## What already exists, verified against the tree on 2026-10-20

- **The witness is the template.** `crates/trace-commons-server/src/bin/trace-commons-witness.rs`
  is a thin binary over `crates/trace-commons-server/src/witness_service/`. Boot
  is fail-closed: every dependency resolves before the listener binds.
  `witness_service/enclave.rs` derives the signing key with
  `GetKey(path = "vms", algorithm = "secp256k1")` on the `/v0/GetKey` surface
  (`GET_KEY_PATH`), and serves a nonce-bound quote whose 64-byte report data is
  `"tcwitns1" | address(20) | nonce(32) | zeros(4)` on
  `GET /v1/attestation?nonce=`.
- **dstack keys are app-id-bound, not measurement-bound.** The KMS derives a key
  from the app id and the `GetKey` path; measurements gate key *release*, not
  *derivation* (`deploy/witness/README.md`, "Upgrades"; `enclave.rs` module
  docs). Different `path` values give different keys; the `algorithm` argument
  does not separate anything. The addendum's "key sealed to the enclave
  measurement" is imprecise, and this spec corrects it in section 6.
- **The client verification core is permissive and reusable.**
  `crates/trace-commons-contributor/src/witness/verify.rs` has `VerifiedWitness`
  (private fields, one constructor, `verify_witness`) and `check_quote`, which
  rebuilds the expected report data and checks the measurement against a
  `WitnessTrust` (`mrtd` + `mrconfigid` sets, defined in `witness/mod.rs`). It
  hardcodes the `tcwitns1` domain tag. `witness/transport.rs` has
  `WitnessNonce`, the attestation fetch and the collateral fetch.
- **Witness pins on the client are configuration**, not compiled in:
  `crates/trace-commons-contributor/src/config.rs` carries an optional
  `witness: WitnessSettings` with a recorded origin. Nothing today makes a
  release daemon refuse a pin it was configured with.
- **Pacing.** `crates/trace-commons-protocol/src/witness_pacing.rs`: only a
  `503` with the exact capacity label is retryable; policy and timeout
  refusals are distinct and are not.
- **Receipts.** `crates/trace-commons-attestation/src/receipt.rs` (behind the
  `receipt` feature, which `crates/trace-commons-server/Cargo.toml` enables)
  has `verify_receipt(payload, request_body, response_body, expected_model)`.
  A three-part receipt binds the model; a two-part receipt binds none.
  `ReceiptSignatureKind::ProviderTee` is signed by a per-model key from the
  gateway report's `model_attestations`; `Gateway` is signed by one key shared
  across all models. `attested_keys_for_receipt`, `model_entry_quotes` and
  `quote_bound_keys` resolve and quote-check those keys.
- **Which key signs depends on the API, not the model**
  (`docs/operator/attested-inference.md`, "Which key signs depends on the
  protocol"): Chat Completions yields `provider_tee`, the Responses API yields
  `gateway`. "A gateway receipt attests the bytes, not the model."
- **Two attestation hosts** (`docs/operator/near-attestation-drill.md`, "Two
  hosts, two questions"): only the gateway `https://cloud-api.near.ai/v1`
  publishes per-model ed25519 keys, and only with `model=` and
  `signing_algo=ed25519`.
- **Per-model receipt pins already exist** on the witness:
  `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` (`model=key[,model=key...]`, parsed by
  `parse_model_key_pins` in `witness_service/inference.rs`).
- **The image workflow** `.github/workflows/witness-image.yml` is manual, builds
  amd64 on a public runner, stamps `SOURCE_DATE_EPOCH`, runs three fail-closed
  boot checks, pushes to GHCR and prints the digest to pin. The image is not
  reproducible, so the measurement pins one artifact.
- **`phala deploy` ignores `app-compose.json`** and builds its own manifest
  (`deploy/witness/build-app-compose.sh` header). The only authoritative
  measurement is read from the deployed instance.
- **Upload claims are EdDSA JWTs** minted by
  `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`
  (`UploadClaimClaims`: `iss`, `aud`, `sub`, `principal_ref`, `tenant_id`,
  `exp`, ...). `principal_ref` is device-scoped
  (`account_trust::device_principal_ref`). Since the NEAR account rework a NEAR
  account's tenant id is random and per account
  (`crates/trace-commons-server/src/near_account_identity.rs`); pooled tenants
  (Devfolio) hold many accounts.
- **The witness does not authenticate accounts.** Its admission route carries
  an `account_anchor_sha256` the server issued and checks later
  (`crates/trace-commons-protocol/src/admission.rs`). The judgement service
  must authenticate the caller itself, because the limiter keys on the account.
- **NEAR AI catalog, re-checked 2026-10-20** (`GET https://cloud-api.near.ai/v1/model/list`,
  59 models): `typesafe/jev-1.13` is `verifiable: false`,
  `attestationSupported: false`, `providerType: external`, `base_url`
  `https://openrouter.ai/api/v1`. Thirteen models are both `verifiable` and
  `attestationSupported`. The flags are not sufficient on their own:
  `moonshotai/kimi-k2.6` reports both flags `true` with `providerType: chutes`
  and an OpenRouter `base_url`. Prices are `inputCostPerToken` /
  `outputCostPerToken` as `{amount, scale: 9, currency: USD}`. The cheapest
  `vllm` model that lists `structured_outputs` is `Qwen/Qwen3.8-27B`
  (input 40, output 1350 at scale 9: $0.04 and $1.35 per million tokens). It
  is a reasoning model.
- **`TaskClassifier` does not exist in code yet.** It is reserved by
  `2026-10-20-insights-model-answers-design.md`, section 5.

## 1. Purpose and non-goals

**Purpose.** Run three pinned judgements over one session's judgement digest
on a NEAR AI model in a TEE, from inside our own attested CVM, and return
labels the daemon can show came from that path:

- `task` -- `{bug_fix, refactor, tests, docs, other}`;
- `event_kind` per marked event -- `{rejection, redirect, iteration, unclear}`;
- `finished` -- `{yes, no, unclear}`.

The service exists so that every contributor's labels come from one pinned
`(prompt_version, model_id)` paid for by the operator, under a per-account cap.

**Non-goals.**

- Storing digests, labels, or any per-session data. The service keeps only the
  limiter ledger (section 6).
- Phase 2, comparisons across users. That needs a sealed per-account store, a
  minimum cohort and query auditing. It gets its own spec; nothing here
  prepares for it beyond using the same keyed account digest.
- Witnessing, redaction, or anything on the contribution path. The service
  shares the witness's patterns and none of its routes or keys.
- Judging Codex denials. The transcripts do not record them (addendum, "Why").

## 2. Deployment

**Where.** A dstack CVM on Phala, CPU-only TDX, deployed the way the witness is
(`deploy/witness/README.md`). The model runs at NEAR AI, so no GPU TEE. One
instance: the limiter ledger is local state, so the service does not replicate
the way the witness does. Judgements are deferrable, so one instance's downtime
pauses judgements and breaks nothing else.

**Code layout.** All new service code is AGPL with the two-line header:

| Path | Contents |
|---|---|
| `crates/trace-commons-server/src/bin/trace-commons-judgement.rs` | Thin binary: env, dstack socket, listener. Copies `trace-commons-witness.rs`'s shape. |
| `crates/trace-commons-server/src/judgement_service/` | Router, handlers, enclave seam, model adapters, prompts, limiter. |
| `crates/trace-commons-protocol/src/insights_judgement.rs` | Wire types, refusal labels, `JUDGEMENT_DIGEST_MAX_BYTES = 8192`, the signing-bytes encoder. Permissive. |
| `deploy/judgement/` | `Dockerfile`, `docker-compose.yml`, `docker-compose.development.yml`, `README.md`. |
| `.github/workflows/judgement-image.yml` | Image build, copied from `witness-image.yml`. |

**Production and development are two builds, two apps, two measurements.**

- A server-crate feature `judgement-development-models` compiles in the
  development-only adapters (Jev, and any model whose catalog entry fails the
  production check). The production image is built without it.
- A unit test asserts that, with the feature off, the adapter registry contains
  no model outside the production allowlist (section 5). The image workflow
  takes a `profile: production | development` input, builds two differently
  named images (`ghcr.io/tracecommons/trace-commons-judgement` and
  `...-judgement-development`), and for `production` checks that the image
  refuses to boot when configured with `typesafe/jev-1.13`. (The `--version`
  line from `trace-commons-build-info` carries no feature list, so it cannot
  be the check.)
- Each profile is deployed as **its own dstack app**, so its own app id. Keys
  derive from the app id, so a development deployment can never derive the
  production limiter or signing keys, even from an identical image.
- An environment variable alone is not the separation. Compose environment
  values are measured (MRCONFIGID), so a flag would change the measurement,
  but one image could then serve either mode, and the production allowlist
  would rest on one variable. A compiled-out adapter cannot be switched on.
- Development serves synthetic or developer-owned data only. The release daemon
  never reaches it (section 4).

**Measurement pin.** `mrtd` + `mrconfigid`, never RTMR3 or RTMR0, for the
reasons in `witness/mod.rs` (`WitnessTrust` docs). The pinned value is read from
the running instance (`phala cvms get --json` `compose_hash`, cross-checked
against a signed response), never from a locally hashed manifest.

**Image checks before push** (the workflow, mirroring the witness's three):
the binary resolves its libraries (`--version`); boot refuses with no model
configured; a production image refuses a development model; boot refuses with no dstack guest agent; boot refuses with no
limiter volume.

**Keys, all via `GetKey` on `/v0/GetKey`**, each on its own path:

| Path | Use |
|---|---|
| `judgement-signing` | secp256k1 key that signs every response (section 3). |
| `judgement-limiter` | 32-byte root; HKDF-SHA256 gives `account-digest` (HMAC key) and `ledger-seal` (AEAD key). |

Neither path is `vms`, so neither equals the witness's key even under one app
id. The NEAR AI API key is the one secret in `allowed_envs`, injected encrypted
at deploy, as on the witness.

**Persistent volume.** One named volume at `/data`, holding only the sealed
ledger. The root filesystem stays read-only with a small `tmpfs`, as on the
witness. The ledger is encrypted by the service itself (section 6), so whether
dstack also encrypts the volume does not affect confidentiality.

## 3. The wire

Two routes, unauthenticated at the transport (dstack-gateway terminates TLS),
and nothing else: no health or metrics route that reports state.

### `GET /v1/attestation?nonce=<64 hex>`

Same shape as the witness: `{"quote_hex", "signing_address"}`. The report data
is `"tcjudge1" | address(20) | nonce(32) | zeros(4)`. A new 8-byte tag, because
the tag is domain separation: a witness quote must never read as a judgement
quote, or the reverse. Not bounded by the request limiter, so load on the
judgement route cannot make the service unpinnable.

### `POST /v1/judgement`

Headers:

- `Authorization: Bearer <judgement claim>` -- the account credential (below).
- `x-trace-judgement-nonce: <64 hex>` -- the nonce the daemon verified the
  quote with. It goes into the signed response, binding the answer to the
  exchange that was verified.

Body (`application/json`, total cap 16 KiB):

```json
{
  "wire_version": 1,
  "digest": "<the judgement digest, UTF-8 JSON as built by contributor-insights>",
  "judgements": ["task", "event_kind", "finished"]
}
```

- `digest` is at most `JUDGEMENT_DIGEST_MAX_BYTES` (8192) bytes. The daemon
  drops an oversized digest whole and never sends it. The service refuses one
  that arrives anyway.
- The service hashes the digest bytes (`digest_sha256`) and logs only that.
- `event_kind` is judged for at most 4 marked events per digest, in digest
  order. Further marks come back `unjudged`, so one session costs at most six
  model calls.

**The account credential** is a new claim kind from the upload-claim issuer:
EdDSA, `aud = "trace-commons-insights-judgement"`, `exp` at most 15 minutes,
and an `account_ref` claim. `account_ref` is the account's tenant id for a
per-account (NEAR-bound) tenant. Accounts in pooled tenants are refused with
`judgement_account_ineligible` in v1, because a per-tenant key there would
share one cap across many people, and a per-device key would multiply one
account's cap by its devices. The distinct audience means an upload claim is
never accepted here and a judgement claim is never accepted by ingest. The
service verifies the claim offline against the issuer's Ed25519 public key(s)
in its measured compose environment (`TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS`).
It makes no call to ingest.

**Response, `200`:**

```json
{
  "wire_version": 1,
  "digest_sha256": "<hex>",
  "results": [
    {"judgement": "task", "label": "bug_fix",
     "confidence": {"bucket": "high", "basis": "probability"},
     "judge_version": {"prompt_version": "task-v1+sha256:<12 hex>", "model_id": "<id>"},
     "receipt": {"signature_kind": "provider_tee", "model": "<id>",
                 "signer": "<ed25519 hex>", "verified": true}},
    {"judgement": "event_kind", "event_index": 0, "label": "redirect", "...": "..."},
    {"judgement": "finished", "label": "unclear", "...": "..."}
  ],
  "spend": {"account_month_remaining_bucket": "over_half"},
  "signed_at": 1792540800,
  "signature": "<65-byte EIP-191 hex over the signing bytes>"
}
```

- `confidence.bucket` is `{high, medium, low}`. `basis` is `probability` when
  the model returns choice probabilities (a decision model, or logprobs), and
  `self_reported` when a chat model reports its own bucket. The daemon shows
  the basis in the answer's (i).
- `receipt` omits the request and response hashes and the provider `chat_id`.
  That follows the witness V2 certificate's reasoning
  (`docs/operator/attested-inference.md`, "Certificate profile rollout"): the
  daemon cannot use them, and they correlate with provider-side records.
- `spend` is a coarse bucket (`over_half | under_half | under_tenth`), never a
  dollar figure, so a response reveals little about how the account uses the
  service.
- **Signing bytes** use a length-prefixed encoder in `insights_judgement.rs`,
  not `Serialize`, for the reason the witness certificate gives
  (`serde_json/preserve_order` once shifted every untyped-JSON digest). The
  domain is `trace_commons.insights_judgement.v1\n`. The signed fields are the
  nonce, `digest_sha256`, every result field, and `signed_at`.

**Refusals** are `{"error": "<label>"}` with a closed label set in
`insights_judgement.rs`, mirroring `AdmissionRefusal` in `admission.rs`. The
client matches the label against the set and never echoes the body.

| Status | Label | Retry |
|---|---|---|
| 503 | `judgement_saturated` | Capacity. Retry after `Retry-After` (30 s), as `witness_pacing`. |
| 503 | `judgement_account_budget_exhausted` | Capacity. Body carries `reset_at` (first instant of next UTC month); do not retry before it. |
| 503 | `judgement_operator_budget_exhausted` | Capacity. Same `reset_at` rule. |
| 400 | `judgement_request_malformed` | Never, for this digest. |
| 413 | `judgement_digest_too_large` | Never, for this digest. |
| 400 | `judgement_wire_version_unsupported` | After a daemon update. |
| 401 | `judgement_credential_refused` | After minting a fresh claim, once. |
| 403 | `judgement_account_ineligible` | Not this month. |
| 502 | `judgement_model_unattested` | Not today. The receipt or catalog check failed (section 5). |
| 504 | `judgement_timeout` | Not today. |

`is_judgement_capacity(status, error)` is true only for `503` and the three
capacity labels, and a test pins the exact strings, as
`witness_pacing::tests` does. A model or timeout failure is not capacity: a
daemon that retried it would spend the account's cap on a broken path.

## 4. Client verification

**Order is the design, as for the witness:** fetch the quote with a fresh
nonce, verify it, and only then send the digest. The digest is scrubbed
counts and excerpts, not a raw transcript, but it is still the first Insights
data to leave the machine (addendum, principle 3).

**Reuse.**

- `check_quote` in `crates/trace-commons-contributor/src/witness/verify.rs`
  takes the domain tag as a parameter instead of reading
  `WITNESS_QUOTE_DOMAIN`. The witness passes `tcwitns1`; the judgement client
  passes `tcjudge1`. Its tests keep running against the witness tag, and new
  tests cover the second tag, including that a witness quote is refused as a
  judgement quote.
- The report-data rebuild and the response-signature recovery move into
  `crates/trace-commons-attestation`, so the ingest drill (section 10) can
  verify the service without depending on the contributor crate.
- A new `crates/trace-commons-contributor/src/judgement/` mirrors `witness/`:
  `VerifiedJudgementService` (private fields, one constructor that verifies),
  and a `send_digest(&VerifiedJudgementService, ...)` in a module that cannot
  construct one. "Send first, check after" does not compile.
- After the response arrives, the daemon recovers the signer from `signature`
  and requires it to equal the address bound in the quote it verified for this
  nonce. It requires the signed nonce to equal its own. Labels from an unsigned
  or mis-signed response are discarded.

**A release daemon accepts only production measurements.**

- The production pin set (`mrtd`, `mrconfigid`, signing address) is a
  compiled-in constant in `judgement/`. There is no configuration key for it in
  a release build.
- A development pin can be configured only when the crate is built with a
  `judgement-development` feature, which release builds do not enable. A test
  over the release feature set asserts that the config parser has no
  judgement pin key.
- The judgement service URL is also compiled in, following the
  `cloud-api.near.ai` precedent in `daemon/nearai_onboarding.rs`.
- Consequence: a new production measurement reaches contributors only in a
  daemon release. An in-place image upgrade therefore pauses judgements for
  release daemons until a release adds the new measurement. That is
  fail-closed and acceptable for an optional feature (open question 2).

## 5. The NEAR AI call

**One adapter trait**, held as `Arc<dyn JudgementModel>` so a different model
substitutes at the seam:

```rust
trait JudgementModel: Send + Sync {
    fn model_id(&self) -> &str;
    /// One typed choice over `choices`, from `prompt` and the digest.
    /// Returns the exact request bytes sent and response bytes received,
    /// so the caller verifies the receipt over them.
    async fn decide(&self, call: DecisionCall<'_>) -> Result<RawDecision, ModelError>;
}
```

Implementations:

- `ChatStructuredAdapter` -- the interim production path. `POST
  /v1/chat/completions` on `https://cloud-api.near.ai/v1`, `stream: false`,
  `temperature: 0`, `response_format` a JSON schema whose `label` is an enum of
  the choices and whose `confidence` is `{high, medium, low}`, `max_tokens: 64`,
  and reasoning off where the model supports it. Chat Completions, never the
  Responses API, because only Chat Completions yields a `provider_tee` receipt
  that binds the model.
- `DecisionModelAdapter` -- for the decision model expected this week. Its API
  is unknown. The trait asks only for "choices in, one choice out, with
  probabilities if available". If its API is not Chat Completions, it must
  still yield a model-binding `provider_tee` receipt, or it fails the
  production check below.
- `MockJudgementModel` -- deterministic, hash-derived from the digest and
  prompt, like the gate-enclave `Mock*` types. Test and development builds
  only; never on the production allowlist.
- `JevAdapter` -- behind `judgement-development-models` only.

**The production check, fail-closed at boot and on every call.** A model is
callable in a production build only if all of these hold:

1. Its id is in the compiled-in production allowlist. The initial list is one
   model (open question 1). Widening it is a code change and therefore a new
   measurement.
2. Its live catalog entry (`GET /v1/model/list`, refreshed hourly) has
   `verifiable: true` **and** `attestationSupported: true`. The flags are
   necessary, not sufficient: `moonshotai/kimi-k2.6` passes both while listing
   an OpenRouter `base_url`. If a refresh flips a flag, calls to that model are
   refused with `judgement_model_unattested` until it flips back. If the
   catalog is unreachable at boot, boot refuses. If refreshes fail, the last
   good entry is used for up to 24 hours, then calls are refused.
3. Its per-model signer pin is configured: `TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS`,
   in the witness's `model=key[,model=key...]` spelling, parsed by the same
   parser (`parse_model_key_pins`, moved to a shared spot if needed). At boot
   the service fetches the **gateway** report
   (`GET https://cloud-api.near.ai/v1/attestation/report?model=..&signing_algo=ed25519&nonce=..`),
   DCAP-verifies each `model_entry_quotes` quote, and requires every pinned
   key to be among `quote_bound_keys`. A pin that the live report does not
   attest refuses the boot.
4. **Per request:** the service fetches the receipt for the completion's id
   (`GET {gateway}/signature/{id}?model=..&signing_algo=ed25519`), runs
   `verify_receipt` over the exact request and response bytes with
   `expected_model = model_id` (it checks the signature itself, dispatching on
   `signing_algo`), and requires `ReceiptAlgo::Ed25519` (an ECDSA signer is not
   quote-bound), `ReceiptSignatureKind::ProviderTee`,
   a three-part receipt (`verdict.model == Some(model_id)`), and a signer in
   that model's pin set. A two-part or `gateway` receipt is refused: it binds
   no model. Any failure discards the label and returns
   `judgement_model_unattested`. The call is still metered, because NEAR AI
   still billed it.

**What a verified receipt proves.** NEAR AI's enclave for `model_id`, holding a
key that a DCAP-verified quote attests, signed the SHA-256 of these exact
request and response bytes, and the receipt names `model_id`. So the digest
went into an attested enclave serving that model, and the label came out of
it.

**What it does not prove.**

- That NEAR AI's enclave image is one we have reviewed. We check that the key
  is quote-bound; pinning NEAR's own measurements is the attestation drill's
  job (`near-attestation-drill.md`), not this service's.
- That the receipt is fresh or unique. Nothing dedupes receipts. It does not
  matter here, because the service makes the call itself and verifies the
  receipt for the response it received.
- Anything about a gateway receipt's model. A gateway key is shared across
  every model; such receipts are refused.
- What the gateway itself saw. The gateway terminates our TLS before the model
  enclave. The TEE-to-TEE hop is an accepted boundary only while each hop is
  attested (addendum, "The judgement service"); the gateway's own attestation
  is not checked per request (open question 3).

If the decision model ships without per-request receipts, it fails check 4 and
cannot be used in production. The addendum's "the (i) must say so" applies
only to a development build.

## 6. Account-bound limiter

**Keyed account digest.** `account_key = HMAC-SHA256(account-digest key,
"insights-judgement/v1" || account_ref)`. The HMAC key derives from
`GetKey(path = "judgement-limiter")`. It is bound to the **app id** and
released only to measurements the app's KMS admits. It is not bound to one
measurement, and that is the property we want: an in-place image upgrade keeps
every account's spend. A redeploy under a new app id (or a KMS change) yields a
new key, an unreadable old ledger, and a deliberate fresh start (section 9,
"Rotate"). The raw `account_ref` lives only in request-scoped memory.

**Metering spend, not calls.**

- Units are integer nano-USD, matching the catalog's `scale: 9`.
- Prices are taken from the catalog entry at the last good refresh.
- Before each model call the service **reserves** an upper bound:
  `ceil(request_bytes / 3) * input_price + max_tokens * output_price`. The
  reservation applies to the account and to the operator total. If it would
  pass either cap, the call is refused with the matching capacity label, and
  no NEAR AI call is made.
- After the call it **settles** to `usage.prompt_tokens * input_price +
  usage.completion_tokens * output_price` from the response body. Reasoning
  tokens count as completion tokens. If `usage` is absent, the reservation
  stands as the charge.
- A crash between reserve and settle leaves the reservation charged. Every
  failure errs toward over-counting.
- Rough cost on the interim model: a 6-call session at about 3,000 input and
  64 output tokens per call costs about $0.0012, so the $5 cap is about 4,000
  sessions a month. `max_tokens` is the guard against a reasoning model
  spending far past that.

**Caps** live in the measured compose environment, so a contributor can read
them from the published manifest:

- `TRACE_COMMONS_JUDGEMENT_ACCOUNT_MONTHLY_CAP_USD=5`;
- `TRACE_COMMONS_JUDGEMENT_OPERATOR_MONTHLY_BUDGET_USD=<open question 4>`.

The month is the UTC calendar month. A cap change is a new measurement.

**Where the state lives.** This is the service's only state.

- In memory: a map from `account_key` to `(month, spent_nanousd)`, plus the
  operator `(month, spent_nanousd)`.
- On disk: `/data/limiter.v1.sealed`, AEAD-encrypted under the `ledger-seal`
  key, with the file name, the format version and the app's signing address as
  associated data. Written by temp file, fsync and atomic rename after every
  reserve and every settle, before the response leaves.
- At a month rollover, the previous month's entries are dropped, so the file
  never holds more than one month. At about 50 bytes an entry it stays small.
- **Restart and upgrade:** the volume and the app id persist, so the ledger is
  read back and spend carries over.
- **Boot rules:** if the file exists but does not decrypt, boot refuses
  (`judgement_limiter_unreadable`). If the file is absent and
  `/data/limiter.initialized` exists, boot refuses. If both are absent, the
  service creates both and logs `limiter_initialized`.
- **What an operator can still do:** delete the whole volume, or roll the file
  back to an older copy, and so reset spend. Both only spend the operator's
  own money; neither reveals anything, because the file is sealed. The
  operator is the payer, so this is not a defence against the operator. The
  per-account cap defends the operator's budget against contributors.
- **What the operator cannot do:** read which accounts used the service or how
  much each spent. The ledger holds keyed digests under a key only the enclave
  derives.

**Logging is hash-only.** A log line may carry a random request id,
`digest_sha256`, the model id, the refusal label or `ok`, and a latency bucket.
It never carries the account, the account key or a hash of either (that would
link one account's calls across time). It never carries labels, confidence,
prompts, the digest, the claim, or NEAR AI response text.

## 7. Prompts

- One prompt per judgement, versioned independently: `task-v1`,
  `event-kind-v1`, `finished-v1`. Each is a text file under
  `crates/trace-commons-server/src/judgement_service/prompts/`, compiled in
  with `include_str!`. Prompts are in the image, so they are in the
  measurement.
- **`prompt_version`** is `<name>-v<N>+sha256:<first 12 hex>`, where the hash
  covers the prompt bytes, the choice list and the output schema. A unit test
  pins the expected hash for each `<name>-v<N>`. Editing a prompt, its choices
  or its schema without bumping `N` fails that test. Bumping `N` changes the
  version, and the test then pins the new hash.
- **Judge version** is `(prompt_version, model_id)`, per result. Either part
  changing is a new judge version, and the model-answers basis switch then
  stops comparing old and new labels (addendum, "What comes back").
- **Output schema** for the chat adapter: `{"label": <enum of choices>,
  "confidence": "high" | "medium" | "low"}`, with no other properties. A
  response that does not parse, or names a value outside the enum, becomes
  `unclear` (or `other` for `task`) with `confidence.bucket: low`. It is still
  receipt-checked and still metered.
- **Decision adapter:** the choices are the enum. Bucket thresholds on the top
  choice's probability are `>= 0.8` high and `>= 0.6` medium, otherwise low.
  These are pinned in the same hashed material, so a threshold change is a
  version bump.
- The digest goes in the user turn, delimited, with an instruction that its
  contents are data. Prompt injection from a contributor's own excerpts can
  only mislabel that contributor's own session.

## 8. Failure modes and fail-closed rules

| Condition | Behaviour |
|---|---|
| No dstack agent, key derivation fails, or quote does not carry own report data | Boot refuses (witness rule). |
| No model configured, model not on the production allowlist, catalog check or signer pin fails at boot | Boot refuses. |
| Issuer public key absent or malformed | Boot refuses. |
| Limiter volume absent, or ledger unreadable or missing with the marker present | Boot refuses. |
| Catalog flag flips, or refresh stale over 24 h | That model's calls refused, `judgement_model_unattested`. |
| Receipt absent, unverifiable, `gateway`, two-part, wrong model, or unpinned signer | Label discarded, call metered, `judgement_model_unattested`. |
| `usage` absent | Reservation stands as the charge. |
| Ledger write fails | The response is not sent; `judgement_saturated`. A service that cannot record spend does not spend. |
| Per-account or operator cap reached | Capacity refusal with `reset_at`; no NEAR AI call. |
| Daemon: quote fails, measurement not in the compiled-in set, or nonce mismatch | Nothing sent. The session keeps its heuristic basis. |
| Daemon: response signature or nonce wrong | Labels discarded. |
| Daemon: any refusal | Session stays on the heuristic basis. The answer's (i) counts it among sessions without a judgement. A missing judgement is never treated as `no`/`other`. |

## 9. Operator runbook outline

The runbook is `docs/operator/insights-judgement-service.md`. The slice that
adds it also adds its row to the index in `docs/operator/README.md`, as the
repo rules require. `deploy/judgement/README.md` holds the recorded deployment
values (app ids, CVM ids, KMS, measurements), as the witness README does.

- **Deploy.** Build with `judgement-image.yml` (`profile`). Pin the digest in
  `deploy/judgement/docker-compose.yml`. Deploy with `phala deploy` and an
  explicit `--kms-id`. Read `compose_hash` from `phala cvms get --json`, and
  poll a signed response until it carries the new measurement (the witness's
  stale-container trap). Record app id, measurement and signing address.
- **Pin.** Add the production measurement and signing address to the daemon's
  compiled-in set in a release. Development pins go only into development
  config.
- **Upgrade.** In place, same app id. Spend carries over. Release daemons
  refuse the new measurement until a release adds it. Ship that release, then
  drop the old measurement in a later release.
- **Rotate.**
  - NEAR AI model key: add the new key to `TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS`
    beside the old (a new measurement), then remove the old.
  - Issuer key: list both keys in `TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS`
    during the overlap.
  - App id or KMS: a new signing address, a new limiter key, and a fresh
    ledger. Delete `/data` deliberately and record why.
- **Drill.** Run `POST /v1/admin/judgement-service-drill` daily and after every
  deploy (section 10).
- **Read a refusal.** A table from label to cause, like the "Reading a
  client-side refusal" table in `attested-inference.md`.
- **Roll back.** Set the account cap to 0 (a new measurement), which turns
  every call into a capacity refusal; or stop the CVM. Daemons fall back to
  the heuristic basis either way.

## 10. Testing

- **Unit tests** (server crate, `judgement_service/`):
  - claim verification: wrong audience, expired, pooled tenant, unknown key;
  - the limiter: reserve, settle, absent usage, month rollover, both caps,
    crash between reserve and settle;
  - ledger seal round trip and refusal on a wrong key;
  - the boot rules, each refusing;
  - the production allowlist test with the development feature off;
  - prompt-hash pins;
  - the signing-bytes encoder against a fixed vector;
  - a log-capture test that no line carries the account, a label or digest
    text.
- **Receipt path**, against the captured live fixtures the attestation crate
  already uses for `provider_tee`, plus refusal tests for `gateway`,
  two-part, wrong model and unpinned signer.
- **Protocol:** the exact refusal strings, and `is_judgement_capacity`, pinned
  like `witness_pacing::tests`.
- **Client:**
  - `check_quote` with both domain tags, and cross-tag refusal;
  - a structural test that `VerifiedJudgementService` has one construction
    site, like `there_is_no_way_to_build_a_verified_witness_but_verification`;
  - a release-feature test that no judgement pin key parses;
  - a signature-mismatch discard.
- **Deterministic local mock.** `MockJudgementModel` with a fake receipt
  signer whose key is pinned in the test config, so the whole request path,
  receipt check included, runs in `cargo test` without NEAR AI. It compiles
  only under `cfg(test)` or the development feature.
- **Measured-image smoke.** The image workflow's pre-push checks (section 2).
  After a development deploy, a script fetches a quote, verifies it against the
  development pin, sends a synthetic digest, and checks the signature and the
  judge version. It never uses contributor data.
- **Drill.** `POST /v1/admin/judgement-service-drill` on ingest, following the
  four steps in `docs/operator/drills.md` ("How to add a new drill"): table
  row, `smoke-test.md` curl, `scripts/operator/smoke-gate.sh`, and the
  `rollout_smoke_evidence` required-check list. It:
  - fetches a quote with a fresh nonce and verifies it against
    ingest-configured production pins, using the attestation-crate
    verification from section 4;
  - mints a drill claim for a dedicated drill account;
  - sends a fixed synthetic digest and verifies the signature;
  - requires every result to carry a verified `provider_tee` receipt and the
    expected judge versions.

  Evidence is hash-only: `purpose_hash`, the measurement, the judge versions
  and `digest_sha256`. Missing configuration reports
  `missing_control:judgement_*` and never passes. Like the NEAR AI drill it
  costs a few paid completions per run, charged to the drill account under the
  same cap. It is required for promotion only when a judgement service URL is
  configured, and reported not-applicable otherwise.

## 11. Delivery slices, in order

Each slice is one PR with its tests. None merges without review.

1. **Protocol.** `insights_judgement.rs`: wire types, the closed refusal set,
   `is_judgement_capacity`, the digest cap, and the signing-bytes encoder.
2. **Shared verification.** Parameterise `check_quote`'s domain tag. Move the
   report-data rebuild and the response-signer recovery into
   `trace-commons-attestation`. The witness tests stay green unchanged.
3. **Service core (library only).** The adapter trait, `MockJudgementModel`,
   prompts and version pins, the receipt-checking call wrapper, the catalog
   check, the limiter and the sealed ledger, and the claim verifier. No
   binary, no CVM.
4. **Issuer.** The `trace-commons-insights-judgement` audience, the
   `account_ref` claim and the pooled-tenant refusal.
5. **Binary, enclave seam and image.** `trace-commons-judgement`, the
   `GetKey` paths, the `tcjudge1` attestation route, `deploy/judgement/`,
   `judgement-image.yml` with both profiles, and the pre-push boot checks.
6. **Development deploy.** The development app with `JevAdapter` and
   synthetic data. Measured-image smoke.
7. **Client.** `contributor/src/judgement/`, the compiled-in production pin
   set (empty until slice 9, so release daemons send nothing), and the
   development-only pin under its feature.
8. **Drill and runbook.** The ingest drill and its four wiring steps, plus
   `docs/operator/insights-judgement-service.md` with its index row.
9. **Production deploy.** Once a model passes section 5: deploy, read the
   measurement, compile it into a daemon release.

The addendum's daemon-side steps (1 digest builder, 2 judged fields and basis
switch, 5 consent copy and opt-in) are independent of slices 1 to 8. Step 5
must land before slice 9 ships to contributors.

## 12. Open questions for the owner

1. **Interim production model.** Use `Qwen/Qwen3.8-27B` (cheapest `vllm` model
   with `structured_outputs`; already calibrated in
   `docs/superpowers/reports/2026-09-19-perplexity-floor-calibration-qwen3-8.md`) until
   the decision model is on NEAR AI and passes section 5, or wait for the
   decision model?
2. **Production pin delivery.** Compiled-in pins mean every service upgrade
   pauses judgements until a daemon release. Is that acceptable, or should
   production measurements come from a list signed by an offline commons key?
3. **Gateway attestation per request.** The gateway sees the digest in
   transit. Should the service also verify the gateway's quote each boot
   (cheap), or each request (one more fetch per call)?
4. **Operator-wide monthly budget.** What figure? Unset refuses boot.
5. **Pooled-tenant accounts.** Excluded in v1 (section 3). Keep that, or key
   them on a per-account identifier the issuer would have to add?
6. **Data volume on Phala.** Confirm that a named volume survives an in-place
   `phala deploy` upgrade on the chosen instance type. If it does not, the
   ledger needs to live elsewhere, and this spec changes.
