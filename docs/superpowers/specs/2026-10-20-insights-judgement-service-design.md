# The Insights judgement service -- design

Date: 2026-10-20, revised 2026-10-21 for the #1501 review (poldsam) and the
slice reviews on #1529, #1530 and #1531, and 2026-11-01 for the `stuck`
judgement (section 14). Owner: Zaki. Status: draft.

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
   quote-bound receipt keys. Every request's receipt is still verified. The
   gateway's own quote is DCAP-verified at boot and its TLS key is pinned on
   every connection (section 5, check 5).
4. **The operator-wide monthly budget is $100,** in the measured compose. It
   sits above the $5 per account per month cap.
5. **Pooled-tenant accounts stay excluded in v1** (the section 3 default).
6. **Phala named-volume survival: checked 2026-10-20 against Phala's docs.**
   - **In-place upgrade:** a volume declared in the compose file survives. The
     production checklist says it is reattached to the new instance. The
     upgrade restarts the whole CVM.
   - **Delete and recreate:** the docs do not cover it, so the runbook says
     to upgrade in place with `phala cvms upgrade`, never delete and
     recreate. Section 6's ledger therefore stays on the `/data` volume.
   - **Secrets:** an upgrade replaces them all, so every upgrade passes the
     full set.
   - **Image tags:** an unchanged tag may not be re-pulled, so every release
     uses a new tag. `phala deploy` ignores `app-compose.json`; section 9
     carries the witness's deploy procedure for that, including the
     `compose_file` read-back.
   - Sources:
     https://docs.phala.network/phala-cloud/production-checklist
     https://docs.phala.com/phala-cloud/update/upgrade-application
     https://docs.phala.com/phala-cloud/faqs

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
  does not separate anything. So any image the app's KMS admits derives the
  same `GetKey` material as the pinned image. A `GetKey` key therefore cannot
  identify the pinned image, and this service never uses one to bind a
  request or a response to the instance (section 3, "Per-boot keys").
- **The witness relies on dstack-gateway's TLS and binds no request to the
  instance.** `deploy/witness/README.md` ("The two routes are
  unauthenticated"): "dstack-gateway terminates TLS". Its quote binds only the
  `GetKey` signing address and the nonce. That leaves the gap #1501 review
  point 1 describes: an operator who admits a second image under the same app
  id can route `/v1/attestation` to the pinned image and the data route to the
  other one, and every client check passes. This service does not copy the
  witness on that point. Its bodies are sealed to a per-boot key that only the
  attested process holds (section 3).
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
  no model outside the production allowlist (section 5). The static guard in
  section 4 ("A static CI guard") keeps the feature out of the production
  image's build line. The image workflow
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
against the `mrconfigid` in a freshly verified quote, section 9 step 5),
never from a locally hashed manifest. Responses are sealed, so the quote, not
a response, is what carries the measurement.

**Image checks before push** (the workflow, mirroring the witness's three):
the binary resolves its libraries (`--version`); boot refuses with no model
configured; a production image refuses a development model; boot refuses with no dstack guest agent; boot refuses with no
limiter volume.

**Keys.** One key comes from `GetKey` on `/v0/GetKey`; the other two are born
in memory at each boot and never leave it:

| Key | Source | Use |
|---|---|---|
| `judgement-limiter` | `GetKey` path `judgement-limiter` (app-id-bound, survives upgrades) | 32-byte root; HKDF-SHA256 gives `account-digest` (HMAC key) and `ledger-seal` (AEAD key). |
| Sealing key | Generated at boot, in memory (section 3) | Key agreement for the sealed request and response bodies. |
| Response-signing key | Generated at boot, in memory (section 3) | secp256k1 key that signs every response. |

There is no `judgement-signing` `GetKey` path. An earlier draft derived the
signing key from `GetKey`, which any admitted image can derive too (#1501
review point 1). The limiter root is not `vms`, so it never equals the
witness's key even under one app id. The NEAR AI API key is the one secret in
`allowed_envs`, injected encrypted at deploy, as on the witness.

**Persistent volume.** One named volume at `/data`, holding only the sealed
ledger. The root filesystem stays read-only with a small `tmpfs`, as on the
witness. The ledger is encrypted by the service itself (section 6), so whether
dstack also encrypts the volume does not affect confidentiality.

## 3. The wire

Two routes, and nothing else: no health or metrics route that reports state.

**What the outer TLS does and does not do.** dstack-gateway terminates the
outer TLS. The gateway, and any instance the operator routes a request to,
therefore sees every byte of the HTTP exchange. The design relies on that TLS
for nothing except hiding paths, sizes and timing from the network. Request
and response bodies are sealed end to end to a key that exists only inside the
attested process. Neither the claim (which carries `account_ref`) nor the
digest ever crosses the gateway in the clear. (#1501 review point 1.)

### Per-boot keys

At boot, after the dstack agent answers and before the listener binds, the
service generates two secp256k1 key pairs in memory. Each secret is 32 bytes
from `ring::rand::SystemRandom`, retried until it is a valid scalar:

- the **sealing key** `seal_sk`, public key `seal_pk` (33 bytes, SEC1
  compressed), used only for the key agreement below;
- the **response-signing key** `sign_sk`, EIP-191 address `signing_address`
  (20 bytes, the witness's address derivation), used only to sign responses.

Neither comes from `GetKey`. Neither is written to disk, logged, or returned
by any route except as a public key. Both die with the process, so a restart,
an upgrade and a second instance each have different keys. That is the
property point 1 needs: `GetKey` material is released to every image the app's
KMS admits, so it cannot tell the pinned image from another admitted one. A
key born in the pinned image's memory can. Two key pairs rather than one keep
the ECDH use and the ECDSA use of a secp256k1 key apart.

### `GET /v1/attestation?nonce=<64 hex>`

Response: `{"quote_hex", "seal_public_key", "signing_address"}` (hex). The
64-byte report data is:

```
report_data = "tcjudge1"            (8 bytes,  [0..8])
           || boot_binding[0..24]   (24 bytes, [8..32])
           || nonce                 (32 bytes, [32..64])

boot_binding = SHA-256("trace_commons.insights_judgement.boot_keys.v1\n"
                       || seal_pk (33) || signing_address (20))
```

- The daemon recomputes `boot_binding` from the two returned public values and
  requires the quote's report data to equal the line above byte for byte.
- Substituting other keys needs a second preimage of 24 bytes of SHA-256
  (2^192 work), and the nonce sits at `[32..64]`, NEAR AI's own convention.
- The 8-byte tag stays domain separation: the witness's report data is
  `"tcwitns1" | address(20) | nonce(32) | zeros(4)`, so a witness quote never
  reads as a judgement quote, or the reverse. The tag is not bumped from
  `tcjudge1`, because no instance ever served the earlier
  `tcjudge1 | address | nonce | zeros` layout (slice 5, #1531, is unmerged and
  undeployed).
- **Bounded on its own** (#1531 review point 7): at most 4 concurrent quote
  calls, each under a 10 s timeout. Either limit returns `503
  judgement_quote_unavailable` (retry after 30 s). This bound is separate from
  the judgement permits, so load on the judgement route cannot make the service
  unpinnable, and a flood of quote requests cannot starve judgements.

### `POST /v1/judgement`

`Content-Type: application/octet-stream`. There is **no `Authorization` and no
nonce header**. The claim and the request nonce travel inside the sealed body,
so neither the gateway nor a misrouted instance learns the account.

**Sealed request** (at most 16 KiB of plaintext, so at most 16,434 bytes on the
wire):

```
0x01 (seal version) || eph_pk (33, SEC1 compressed) || ciphertext || tag (16)
```

**The judgement seal.** It lives in `trace-commons-attestation` (permissive), so
the daemon and the service run one implementation:

1. For each request the daemon generates a fresh secp256k1 key `eph_sk`.
   `shared` is the x coordinate of `eph_sk * seal_pk` (32 bytes), computed with
   k256's point arithmetic. That is its default `arithmetic` feature, not the
   `ecdh` feature, so no feature, lockfile or dependency changes. An identity
   result is refused.
2. HKDF-SHA256 (`ring::hkdf`):
   `prk = Extract(salt = "trace_commons.insights_judgement.seal.v1", ikm = shared)`.
   `k_req = Expand(prk, "request" || eph_pk || seal_pk, 32)` and
   `k_resp = Expand(prk, "response" || eph_pk || seal_pk, 32)`.
3. AEAD is ChaCha20-Poly1305 (`ring::aead`). Each key seals exactly one
   message, so the nonce is 12 zero bytes. The AAD is `"tcjudge1/request" ||
   0x01` for the request and `"tcjudge1/response" || 0x01` for the response.

**Why this construction and not RFC 9180 HPKE.** No HPKE crate is in the
workspace, and the repository's dependency rule forbids adding one without
approval. `ring`'s X25519 consumes its private key on every agreement
(`agreement::EphemeralPrivateKey`), so it cannot hold a per-boot static key.
k256 can, and `trace-commons-attestation` already depends on both `k256` and
`ring` (the contributor crate reaches them through it). The construction keeps
HPKE's base-mode shape: an ephemeral-static DH, a key schedule bound to both
public keys, and one AEAD message per key. It is **not** RFC 9180
interoperable, and RFC 9180 defines no secp256k1 KEM. Frozen test vectors pin
it. Moving to the `hpke` crate is open decision S3 (section 12).

**Request plaintext** (JSON, closed, unknown fields refused):

```json
{
  "wire_version": 1,
  "request_nonce": "<64 hex, fresh per send>",
  "claim": "<judgement claim, an EdDSA JWT>",
  "digest_b64": "<standard base64 of the digest bytes>",
  "judgements": ["task", "event_kind", "finished"]
}
```

- The digest is at most `JUDGEMENT_DIGEST_MAX_BYTES` (8192) bytes, and the
  addendum's byte budgets keep every digest under it by construction
  (addendum, "Size"). The service refuses an oversized one anyway.
- `digest_b64` is base64 rather than a JSON string, so the digest is not
  escaped a second time. An escape-heavy digest would nearly double as a
  string; base64 is a fixed 4/3.
- The service hashes the decoded digest bytes (`digest_sha256`) and logs only
  that.
- `request_nonce` is generated by the daemon for this one send and never
  reused (section 4). The service keeps every request nonce it accepted in
  memory for 20 minutes (the claim's 15-minute lifetime plus skew). A repeat is
  refused as `judgement_request_replayed`. The set holds at most 65,536
  entries. When it is full, requests get `judgement_saturated`, so the check
  fails closed. A restart empties the set, and it also changes `seal_pk`, so a
  request sealed before the restart no longer opens.
- `event_kind` is judged for at most 4 marked events per digest, in digest
  order. Further marks come back `unjudged`, so one session costs at most six
  model calls.

**The service's check order** is chosen so an unauthenticated caller cannot
hold a judgement slot (#1531 review point 4):

1. **Read** the body under its own bound: at most 32 concurrent reads, 10 s per
   body, 16,434 bytes. Over the size gives `413 judgement_digest_too_large`.
   Over the time drops the connection.
2. **Open** the seal. If it does not open, the response is `400
   judgement_seal_stale`. The usual cause is a daemon that sealed to a
   previous boot's key.
3. **Parse** the request, then check the wire version and the digest cap.
4. **Verify the claim** offline and refuse pooled tenants (below).
5. **Record the request nonce**, refusing a repeat.
6. Only now **take a judgement permit** (`try_acquire`, 8 by default). If none
   is free, `judgement_saturated`.
7. Check the rotation latch and the catalog (section 5), reserve (section 6),
   make the model calls, check each receipt, settle.
8. **Build, sign and seal** the response under `k_resp`.

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

- **Lifetime and clock skew** (#1530 review point 2). The issuer clamps a
  judgement claim's lifetime to `min(max_ttl, 840)` seconds: 900 minus the
  60 s skew allowance. The verifier accepts `now <= exp + 60`,
  `exp - now <= 900 + 60` and `iat <= now + 60`. With the earlier clamp of 900
  and no skew on the `exp - now` bound, a service clock one second behind the
  issuer refused every claim once an operator raised the upload TTL to 900 or
  more. Tests verify at `issuer_now - 30` and `issuer_now + 30`.
- **The claim route's refusals are a closed set** (#1530 review point 3).
  `insights_judgement.rs` gains `JudgementClaimRefusal`, which has a label, a
  status and a retry class for each refusal. The handler returns only its
  labels, and a test pins every string. Each of today's refusals maps to one
  member:
  - permanent: request malformed, device key id mismatch, device key not
    registered, device key revoked, `judgement_account_ineligible` (not this
    month);
  - retry once with a fresh timestamp: `insights_judgement_claim_request_stale`;
  - retry after 30 s: registry unavailable (today's 503), issue failed (today's
    500 "failed to issue upload claim").

  The doc comment that called these "the device-key labels the upload-claim
  route already uses" is corrected. The upload-claim route spells them with
  spaces, and this route does not. The daemon's re-mint logic (slice 7)
  matches only the set, never prose.

**Response, `200`:** `application/octet-stream`, `0x01 || ciphertext || tag`,
sealed under `k_resp`. Only the daemon that made `eph_sk` can open it, so the
labels do not cross the gateway in the clear either. The plaintext is this
signed JSON:

```json
{
  "wire_version": 1,
  "request_nonce": "<64 hex, echoed>",
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
  domain is `trace_commons.insights_judgement.v1\n`. The signed fields are,
  in order: `wire_version`, `request_nonce`, `digest_sha256`, every result
  field (`judgement`, `event_index`, `label`, confidence, judge version,
  receipt), the `spend` bucket, and `signed_at`. (#1501 review point 12:
  `wire_version` and `spend` were unsigned, and the quote nonce stood in for a
  per-request one.) The signature is by `sign_sk`, the per-boot key bound in
  the quote.

**Refusals** are plaintext `{"error": "<label>"}` with a closed label set in
`insights_judgement.rs`, mirroring `AdmissionRefusal` in `admission.rs`. They
carry no data, so they are not sealed. A forged refusal can only defer a
judgement, because the daemon never treats a refusal as an answer. The client
matches the label against the set and never echoes the body.

| Status | Label | Retry |
|---|---|---|
| 503 | `judgement_saturated` | Capacity. Retry after `Retry-After` (30 s), as `witness_pacing`. |
| 503 | `judgement_account_budget_exhausted` | Capacity. Body carries `reset_at` (first instant of next UTC month); do not retry before it. |
| 503 | `judgement_operator_budget_exhausted` | Capacity. Same `reset_at` rule. |
| 400 | `judgement_request_malformed` | Never, for this digest. |
| 413 | `judgement_digest_too_large` | Never, for this digest. |
| 400 | `judgement_wire_version_unsupported` | After a daemon update. |
| 401 | `judgement_credential_refused` | After minting a fresh claim, once. |
| 400 | `judgement_seal_stale` | After re-verifying a fresh quote, once. The seal did not open, usually because the service restarted and has new keys. |
| 409 | `judgement_request_replayed` | Never. A daemon never resends a request nonce, so this means a replay on the path. |
| 503 | `judgement_quote_unavailable` | `GET /v1/attestation` only. Retry after 30 s. Not a judgement capacity label. |
| 403 | `judgement_account_ineligible` | Not this month. |
| 502 | `judgement_model_unattested` | Not today. The receipt or catalog check failed (section 5). |
| 504 | `judgement_timeout` | Not today. |
| 500 | `judgement_service_fault` | Not today. An internal fault after spend may already be recorded: the response could not be signed, or the settle could not be written. Never retried for this digest the same day, because a retry would be charged again (#1531 review point 6). |

`is_judgement_capacity(status, error)` is true only for `503` and the three
capacity labels, and a test pins the exact strings, as
`witness_pacing::tests` does. A model or timeout failure is not capacity: a
daemon that retried it would spend the account's cap on a broken path.

## 4. Client verification

**Order is the design, as for the witness:** fetch the quote with a fresh
nonce, verify it, and only then seal and send. Whatever the owner decides the
digest holds (addendum, owner decision O1), it is the first Insights data to
leave the machine (addendum, principle 3).

**Two nonces, with separate jobs** (#1501 review point 12):

- The **quote nonce** proves the quote is fresh. The daemon makes one per
  verification and uses it only in `GET /v1/attestation`.
- The **request nonce** binds one response to one send. `send_digest`
  generates it inside, with the ephemeral key, for that call alone. It is
  never a parameter and never stored, so it cannot be reused.

**Reuse.**

- `check_quote` in `crates/trace-commons-contributor/src/witness/verify.rs`
  and the attestation crate's `expected_report_data` take a **report-data
  layout**, not just a domain tag. The judgement layout hashes
  `seal_pk || signing_address` into `[8..32]` and puts the nonce at `[32..64]`
  (section 3). That is a different rebuild from the witness's
  `tag | address | nonce | zeros`, so a tag parameter alone cannot express it.
  The witness passes its existing layout and its tests stay unchanged. New
  tests cover the judgement layout, and that a witness quote is refused as a
  judgement quote and the reverse.
- The report-data rebuild, the response-signature recovery and the judgement
  seal live in `crates/trace-commons-attestation`, so the ingest drill (section
  10) can verify the service without depending on the contributor crate.
- A new `crates/trace-commons-contributor/src/judgement/` mirrors `witness/`.
  `VerifiedJudgementService` has private fields and one constructor that
  verifies. It holds `seal_pk`, `signing_address` and the verification time.
  `send_digest(&VerifiedJudgementService, ...)` lives in a module that cannot
  construct one, so "send first, check after" does not compile.
- A verified service is reused for at most one hour, then verified again with
  a new quote nonce. It is also verified again after `judgement_seal_stale`,
  and the send is retried once.

**What the daemon checks on a response.** Any failure discards every label in
it:

1. The body opens under `k_resp`.
2. The signer recovered from `signature` equals `signing_address` from the
   quote verified for this service.
3. The signed `request_nonce` equals the one this send generated.
4. The signed `digest_sha256` equals SHA-256 of the digest bytes this send
   sealed. Swapping two signed responses between sessions therefore fails.
5. `wire_version` is 1. Each requested judgement appears exactly once, and
   `event_kind` appears once per judged mark.
6. Every `event_index` is less than the number of event markers in the digest
   that was sent, and only the first 4 are judged.

**A release daemon accepts only production measurements.**

- The production pin set is (`mrtd`, `mrconfigid`), a compiled-in constant in
  `judgement/`. It holds no signing address. The address and the sealing key
  are per boot, and the daemon takes both from the quote it verified. There is
  no configuration key for any of it in a release build.
- A development pin can be configured only when the crate is built with a
  `judgement-development` feature, which release builds do not enable. A test
  over the release feature set asserts that the config parser has no
  judgement pin key.
- **A static CI guard keeps both development features out of everything that
  ships** (#1501 review point 7; #1531 review point 5). The unit test above
  runs under whatever features CI picks and never reads the release
  workflows. A daemon built with `judgement-development` could send real
  sessions to the development service, whose Jev adapter goes to OpenRouter
  outside any TEE. A new script, `scripts/ci/check-release-judgement-features.sh`,
  is a sibling of `scripts/ci/check-release-credential-store.sh` with the same
  `check` and `self-test` modes, wired into `ci.yml` beside it. It refuses:
  - `judgement-development` or `judgement-development-models` on any build
    line in the files that script already scans (`release-apps.yml`,
    `release-contributor.yml`, every `release-*.yml`, and the packaging
    scripts it lists), and `--all-features` on any of those lines;
  - either feature in the `default` list of `trace-commons-contributor`,
    `trace-commons-contributor-ffi` or `trace-commons-server`, or enabled
    from a shipped crate's `[dependencies]`;
  - in `deploy/**/Dockerfile` and `judgement-image.yml`: `--all-features`
    on any build line, and a `production)` profile arm whose feature list is
    anything other than exactly `near-attestation-collateral`.

  The self-test seeds one violation of each kind and proves each fails, so
  the check cannot rot into one that never fails. The server half lands with
  slice 5, which adds the image. The `judgement-development` name is refused
  from the start, before slice 7 adds the feature, so no window opens.
- The judgement service URL is also compiled in, following the
  `cloud-api.near.ai` precedent in `daemon/nearai_onboarding.rs`.
- Consequence: a new production measurement reaches contributors only in a
  daemon release. An in-place image upgrade therefore pauses judgements for
  release daemons until a release adds the new measurement. That is
  fail-closed and acceptable for an optional feature (owner decision 2).

## 5. The NEAR AI call

**One adapter trait**, held as `Arc<dyn JudgementModel>` so a different model
substitutes at the seam:

```rust
trait JudgementModel: Send + Sync {
    fn model_id(&self) -> &str;
    /// One typed choice over `choices`, from `prompt` and the digest.
    /// Returns the exact request bytes sent, the response bytes received and
    /// the completion id, so the caller verifies the receipt over them.
    /// It returns no answer and no usage.
    async fn decide(&self, call: DecisionCall<'_>) -> Result<RawDecision, ModelError>;
    /// Pure: the label, confidence and usage, read from `response_body`.
    /// Called only after the receipt over that body has verified.
    fn parse_answer(&self, call: &DecisionCall<'_>, response_body: &[u8])
        -> Result<ParsedAnswer, ParseError>;
}
```

**The label comes from the receipt-verified bytes** (#1529 review point 2).
`RawDecision` carries no `answer` and no `usage`. An earlier shape let an
adapter fill `answer` separately from the `response_body` the receipt covers.
A retry that kept attempt 1's body and receipt with attempt 2's answer, or a
wrong JSON path, would then have shipped a label the enclave never produced,
with a `provider_tee` receipt as its evidence. Now the wrapper verifies the
receipt over `response_body` first, and only then calls `parse_answer` on
those same bytes. The mock's faults (`LabelOutsideChoices`, `Unparseable`)
change the body itself, and its fake receipt signs the changed body. A test
gives the mock a body whose label differs from the one an adapter would have
reported, and asserts the result follows the body.

Implementations:

- `ChatStructuredAdapter` -- the Chat Completions adapter. It is **not** a
  production path: owner decision 1 leaves the production allowlist empty
  until the decision model passes this section. It serves the development
  build, and any later allowlisted model that speaks Chat Completions. `POST
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

1. Its id is in the compiled-in production allowlist. **The list is empty**
   (owner decision 1): `PRODUCTION_MODEL_ALLOWLIST: &[&str] = &[]`, and a unit
   test asserts it is empty. Slice 9 changes the test when it adds the
   decision model. With an empty list a production build refuses to boot,
   which is decision 1's "the production service refuses to start". Widening
   the list is a code change and therefore a new measurement.
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
   `judgement_model_unattested`. That one call is still metered, because NEAR
   AI billed it. A failure that means "the pins no longer match" also sets
   the rotation latch (below), so it is the only such call.

5. **The gateway that carries the call is attested, and its TLS key is
   pinned on the connection** (owner decision 3; #1501 review point 2). The
   cloud-api.near.ai gateway terminates the service's TLS and sees the
   plaintext digest. A receipt cannot cover that hop: the model enclave signs
   whatever the gateway forwarded. Without this check, a non-TEE proxy or a
   mis-issued certificate would see every digest while every receipt still
   verified.
   - **At boot, before the listener binds:** fetch
     `GET {gateway}/attestation/report?include_tls_fingerprint=true&signing_algo=ed25519&nonce=<fresh 32 bytes>`.
     DCAP-verify `gateway_attestation.intel_quote` with the same verifier and
     collateral path as check 3. Require the nonce at `report_data[32..64]`.
     Require `report_data[0..32] = SHA-256(signing_address || spki_hash)`,
     the fingerprint-mode binding in NEAR AI's verifier README
     (`docs/operator/near-attestation-drill.md`, "The zero-padding
     assertion"). Require the gateway's measurement to match
     `TRACE_COMMONS_JUDGEMENT_NEAR_GATEWAY_MEASUREMENTS`, in the existing
     `ExpectedMeasurements` syntax (`mrtd` + `mrconfigid` at least). Without
     a measurement pin, a DCAP-valid quote proves only that some TDX guest
     holds the TLS key, not that NEAR AI's gateway image does. The result is
     a verified `spki_hash`.
   - **On every connection:** the HTTP client the model adapter uses carries a
     custom certificate verifier. It runs normal WebPKI validation **and**
     requires SHA-256 of the leaf certificate's SubjectPublicKeyInfo to equal
     the verified `spki_hash`. A connection that fails either check is closed
     before any request byte is written, so nothing reaches an unverified
     gateway. One client serves the model call, the receipt fetch and the
     catalog fetch.
   - **On a mismatch** (NEAR AI rotated its certificate key): stop sending,
     re-run the boot fetch above, and adopt the new `spki_hash` only if a
     fresh, DCAP-verified, measurement-pinned report binds it. Until then,
     calls to every NEAR AI model are refused before any reservation with
     `judgement_model_unattested`. Nothing is metered, because nothing was
     sent.
   - **Two facts are not verifiable from the tree, so they gate slice 5's
     implementation of this check.** First, the response field that carries
     `spki_hash` in fingerprint mode: every captured fixture is default mode,
     and the drill asserts the zero padding. A live fingerprint-mode fixture is
     captured before the code is written. Second, the client hook: the
     verifier needs a direct `rustls` dependency in `trace-commons-server`.
     `trace-commons-ingest` already declares
     `rustls = { version = "0.23", default-features = false, features = ["ring"] }`,
     so no package is added to the lockfile, but it is a new direct edge and
     needs owner approval (open decision S2). The slice confirms the reqwest
     method that installs a preconfigured rustls config in the vendored
     source before naming it
     (`grep -rn use_preconfigured_tls ~/.cargo/registry/src/*/reqwest-0.12*/src/async_impl/client.rs`).
   - **Until both land, a NEAR AI-hosted model cannot boot.** Slice 5 adds the
     refusal: any model whose adapter calls NEAR AI refuses boot with
     `missing_control:judgement_near_gateway_pin` unless the gateway check is
     compiled in and configured. The mock and the development adapters do not
     call NEAR AI and are unaffected.

**The rotation latch** (#1501 review point 14; #1529 review point 1). NEAR AI
does not announce a new provider-TEE key in advance, so its pins cannot be
added before a rotation. Without a latch, every request after a rotation
would reserve, make the paid call, settle, and only then fail check 4: up to
six billed calls per request, draining every account's $5 and the operator's
$100 until a redeploy.

- The first receipt for a model that fails with an unpinned signer,
  `NotProviderTee` or `WrongModel` sets a per-model latch on the production
  check. So does a verified usage above the reservation (section 6).
- The call check refuses a latched model **before reserving**, with
  `judgement_model_unattested`. The next request makes no NEAR AI call and is
  charged nothing.
- The latch clears only when the pins change: at a boot whose pins the live
  gateway report attests, or through the pin-update path, if the owner
  chooses one (open decision S1, section 12).
- Test: after one `SignerUnpinned`, the next `judge` makes no model call (the
  mock's call count does not move) and charges nothing.
  `an_unpinned_signer_is_refused_but_still_metered` keeps its first-call
  assertion and gains this second-call one.
- The drill reports a latched model as `judgement_model_latched`, a blocking
  gap. That is the alert.

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
- What the gateway itself saw. Check 5 covers that hop separately: the
  gateway's TLS key is bound in a DCAP-verified, measurement-pinned quote, and
  every connection is pinned to it. So both hops are attested, as the
  addendum requires ("The judgement service"). A receipt alone would not show
  it.

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
- Before each model call the service **reserves** a real upper bound
  (#1501 review point 13; #1529 review point 3):
  `(request_bytes + 512) * input_price + max_tokens * output_price`.
  - `request_bytes` is the length of the exact HTTP request body the adapter
    will send, which contains the prompt and the digest plus JSON framing.
  - The bound rests on one stated assumption: a byte-level BPE tokenizer,
    Qwen's included, emits at most one token per byte of text. Digits split
    one per token and JSON punctuation can run near one byte per token, which
    is why the earlier `ceil(request_bytes / 3)` was not a bound.
  - The `512` covers chat-template tokens added on top of the text.
  - `max_tokens` bounds every completion token, reasoning included.
- **Check and reserve are one atomic step.** One lock covers the whole ledger:
  the account entry and the operator total are checked and both increased,
  and the sealed ledger is written, all before the lock is released. Two
  devices on one account share one entry, so they cannot both pass the check
  and then both add. If the reservation would pass either cap, the call is
  refused with the matching capacity label, and no NEAR AI call is made.
- **Settle only after the receipt verifies** (#1529 review point 4). If the
  receipt verifies, the charge becomes `usage.prompt_tokens * input_price +
  usage.completion_tokens * output_price` from the verified body. Reasoning
  tokens count as completion tokens. If the receipt fails, the charge is
  `max(reservation, reported)`, because NEAR AI billed the call whatever its
  body says. A swapped response claiming `usage {0, 0}` therefore cannot
  lower spend. If `usage` is absent, the reservation stands.
- **If verified usage ever exceeds the reservation,** the reported figure is
  charged. The service logs the label `judgement_usage_over_reservation` and
  latches that model, as for a rotated key (section 5, "The rotation latch"),
  because the bound's assumption has failed for it. The next reservation
  check then sees the true spend.
- A crash between reserve and settle leaves the reservation charged. Every
  failure errs toward over-counting.
- **The ledger month follows a signed clock** (#1529 review point 5). The
  month advances only inside a reservation, which runs only after the claim
  has verified. The claim check bounds the service clock to within 60 s of
  the issuer's signed `iat` and `exp` (section 3). The month may advance only
  to the month of the verified claim's `iat`, and never by more than one month
  past the last persisted month unless `iat` is in that month too. A
  CVM-clock reading in the far future therefore cannot persist: its claims
  fail the expiry check first, and the month guard is a second check. The
  ledger month is exposed, so a capacity refusal's `reset_at` is the first
  instant of the month after it.
- Rough scale, priced as if on `Qwen/Qwen3.8-27B`, the cheapest `vllm` catalog
  model with structured outputs. That is for sizing only; it is **not**
  allowlisted. A 6-call session at about 3,000 input and 64 output tokens per
  call costs about $0.0012, so the $5 cap is about 4,000 sessions a month.
  `max_tokens` is the guard against a reasoning model spending far past
  that.

**Caps** live in the measured compose environment, so a contributor can read
them from the published manifest:

- `TRACE_COMMONS_JUDGEMENT_ACCOUNT_MONTHLY_CAP_USD=5`;
- `TRACE_COMMONS_JUDGEMENT_OPERATOR_MONTHLY_BUDGET_USD=100` (owner decision 4).
  Unset or unparseable refuses boot (section 8).

The month is the UTC calendar month. A cap change is a new measurement.

**Where the state lives.** This is the service's only state.

- In memory: a map from `account_key` to `(month, spent_nanousd)`, plus the
  operator `(month, spent_nanousd)`.
- On disk: `/data/limiter.v1.sealed`, AEAD-encrypted under the `ledger-seal`
  key, with the file name, the format version and the fixed label
  `insights-judgement/ledger` as associated data. It no longer binds the
  signing address, which is now per boot (section 3) and would make the ledger
  unreadable after every restart. The binding to the app comes from
  `ledger-seal` itself, which derives from the app-id-bound limiter root. Written by temp file, fsync and atomic rename after every
  reserve and every settle, before the response leaves.
- At a month rollover, the previous month's entries are dropped, so the file
  never holds more than one month. At about 50 bytes an entry it stays small.
- **Restart and upgrade:** the volume and the app id persist, so the ledger is
  read back and spend carries over.
- **Boot rules:** if the file exists but does not decrypt, boot refuses
  (`judgement_limiter_unreadable`). If the file is absent and
  `/data/limiter.initialized` exists, boot refuses. If both are absent, the
  service creates both and logs `limiter_initialized`.
- **Losing the volume resets every cap, and nothing detects it** (#1531
  review point 8). The ledger and the marker live on the same volume. A
  recreated CVM, a volume that is not reattached, or a rolled-back snapshot
  boots as fresh with zero spend. That is the residual risk until a counter
  exists outside the volume: each such event allows up to one more $5 per
  account and one more $100 of operator budget in that month. The only
  signal is the `limiter_initialized` log label at a boot the operator did
  not intend.
- **Recovering from the two boot refusals.** Neither is repaired
  automatically, and each recovery is a deliberate reset that is recorded:
  - `judgement_limiter_unreadable` (the file exists and does not decrypt).
    Compare the app id and KMS against `deploy/judgement/README.md`. If either
    changed, this is the "Rotate" case of section 9. Otherwise the file is
    corrupt. In both cases, keep a copy of the file, delete
    `/data/limiter.v1.sealed` and `/data/limiter.initialized`, and boot.
  - `judgement_limiter_missing` (the marker is present, the ledger is absent).
    The ledger was deleted or lost. Delete the marker and boot.
  - Either way, add a row to the ledger-reset log in
    `deploy/judgement/README.md`: date, cause, and who. Spend for the month
    restarts at zero.
- **What an operator can still do:** delete the whole volume, or roll the file
  back to an older copy, and so reset spend. Both only spend the operator's
  own money; neither reveals anything, because the file is sealed. The
  operator is the payer, so this is not a defence against the operator. The
  per-account cap defends the operator's budget against contributors.
- **What the operator can learn, stated plainly** (#1501 review point 10).
  An earlier draft said the operator could not read which accounts used the
  service. That was wrong in two ways:
  - **Through KMS admission.** The limiter root comes from `GetKey`, and the
    KMS releases it to every image the app admits. The operator controls
    admission. An admitted debug image can derive `account-digest` and
    `ledger-seal` and open the ledger. `account_ref` is a tenant id the
    operator already holds, so HMAC over the known ids links every entry to
    its account. The boundary is the app's KMS admission policy, not the
    enclave. What admission does **not** give: the digests, claims and labels.
    Those are sealed to the pinned instance's per-boot keys (section 3), and
    daemons send only to a pinned measurement.
  - **Through the issuer.** Every send needs a fresh judgement claim from the
    issuer on ingest, which authenticates the device, and so the account. The
    issuer process therefore sees which account asked for a claim, and when.
    The operator can match mint times against the service's request times and
    the client IPs Phala's gateway sees.
- **What reduces that, and what it does not remove:**
  - The judgement-claim route logs no tenant, account, device key id or
    `device_ref`, unlike the upload-claim route
    (`trace_upload_claim_issuer.rs`). A tracing-capture test mints once and
    hits every refusal, and asserts that no captured line contains the tenant
    id or the device key id. That includes the malformed-stored-key path in
    `device_public_key_bytes`, which logs `device_key_id` today (#1530 review
    point 1).
  - The edge access log drops the device headers. In
    `deploy/pilot-gcp/Caddyfile.template`, the issuer host's log filter (and
    the ingest host's, for parity) gains
    `request>headers>X-Trace-Device-Key-Id delete` and
    `request>headers>X-Trace-Device-Signature delete`. Caddy logs request
    headers by default, so without this every claim request leaves the
    plaintext device key id beside the path and the time.
  - The daemon mints judgement claims on a fixed schedule while judgements
    are on, one every 10 minutes whether or not it sends, and holds at most
    one unexpired claim. Mint times then mark "judgements are on", which the
    issuer knows anyway, rather than each send.
  - None of this hides from the issuer process which accounts have
    judgements on. The consent copy never says "the operator cannot see",
    and it says calls are tied to the account.

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
  covers everything that changes what a label means (#1529 review point 6):
  the prompt bytes, the choice list, the output schema, `max_tokens`
  (`JUDGEMENT_MAX_TOKENS`, since a lower cap truncates more answers into the
  fallback), the temperature and reasoning settings, the per-kind fallback
  label, the fallback confidence bucket, and the decision-adapter thresholds. A unit test
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
| No dstack agent, limiter key derivation fails, per-boot key generation fails, or quote does not carry own report data (the `boot_binding` layout of section 3) | Boot refuses (witness rule). |
| Sealed body does not open | `judgement_seal_stale`; nothing else is read. |
| Request nonce seen before, or the nonce set is full | `judgement_request_replayed`, or `judgement_saturated` when full. No model call. |
| Body read exceeds 10 s or the read bound is full | Connection dropped, or `judgement_saturated`. No judgement permit was taken. |
| Quote call exceeds 10 s or the attestation bound is full | `judgement_quote_unavailable`. |
| No model configured, model not on the production allowlist, catalog check or signer pin fails at boot | Boot refuses. |
| Issuer public key absent or malformed | Boot refuses. |
| Account cap or operator budget unset or unparseable | Boot refuses. |
| Production build with an empty model allowlist (the state until slice 9) | Boot refuses. |
| Limiter volume absent, or ledger unreadable or missing with the marker present | Boot refuses. Recovery is a recorded, deliberate reset (section 6). |
| Ledger and marker both absent (a lost or replaced volume) | Boots fresh with zero spend and logs `limiter_initialized`. Residual risk, stated in section 6. |
| Catalog flag flips, or refresh stale over 24 h | That model's calls refused, `judgement_model_unattested`. |
| NEAR AI gateway quote fails DCAP, its measurement is not pinned, its report data does not bind the nonce and `spki_hash`, or the fingerprint field is absent, at boot | Boot refuses. |
| A NEAR AI-hosted model is configured but the gateway check is not compiled in or not configured | Boot refuses, `missing_control:judgement_near_gateway_pin`. |
| A connection's leaf SPKI differs from the verified `spki_hash` | Connection closed before any request byte. Calls refused, nothing metered, until a fresh verified report binds the new key. |
| Receipt absent, unverifiable, `gateway`, two-part, wrong model, or unpinned signer | Label discarded; charged `max(reservation, reported)`; `judgement_model_unattested`. For an unpinned signer, a receipt that is not `provider_tee`, or a wrong model, the rotation latch is set, so no further call to that model is made until the pins change (section 5). |
| `usage` absent | Reservation stands as the charge. |
| Ledger write fails at reserve | No model call; `judgement_saturated`. A service that cannot record spend does not spend. |
| Ledger write fails at settle, or the response cannot be signed | `judgement_service_fault` (not capacity, not retryable today), because the call was already charged. |
| Verified usage exceeds the reservation | The reported figure is charged; `judgement_usage_over_reservation` is logged; the model is latched. |
| Per-account or operator cap reached | Capacity refusal with `reset_at`; no NEAR AI call. |
| Daemon: quote fails, measurement not in the compiled-in set, quote nonce or `boot_binding` mismatch | Nothing sent. The session keeps its heuristic basis. |
| Daemon: response does not open, or its signer, request nonce, `digest_sha256`, wire version or an `event_index` is wrong (section 4) | Labels discarded. |
| Daemon: any refusal | Session stays on the heuristic basis. The answer's (i) counts it among sessions without a judgement. A missing judgement is never treated as `no`/`other`. |

## 9. Operator runbook outline

The runbook is `docs/operator/insights-judgement-service.md`. The slice that
adds it also adds its row to the index in `docs/operator/README.md`, as the
repo rules require. `deploy/judgement/README.md` holds the recorded deployment
values (app ids, CVM ids, KMS, measurements), as the witness README does.

- **Deploy, with the witness's `phala deploy` lessons** (#1501 review point 6;
  `deploy/witness/README.md`, the `compose_file` read-back and the visibility
  flags). `phala deploy` never reads `app-compose.json`. It builds its own
  manifest, and by default it turns `public_logs` and `public_sysinfo` on and
  adds `DSTACK_AUTHORIZED_KEYS` to `allowed_envs`, with a pre-launch script
  that installs root SSH keys. A pinned measurement with those settings would
  publish the service's logs and admit a root shell that can read digests and
  claims in memory and call `GetKey` for the limiter root. So:
  1. Build with `judgement-image.yml` (`profile`). Pin the image digest in
     `deploy/judgement/docker-compose.yml`. Every release uses a new tag.
  2. Deploy with `phala deploy --kms-id <id> --no-public-logs
     --no-public-sysinfo`. Pass both `--no-` flags on every deploy. For an
     upgrade, use `phala cvms upgrade` with the same flags where it accepts
     them; step 3 is the check either way.
  3. **Read the manifest back**: `phala cvms get <id> --json`, field
     `compose_file`. Require all of these:
     - `public_logs: false` and `public_sysinfo: false`;
     - `allowed_envs` exactly `["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"]`
       (development: `[]`, since the development adapters need no key in
       `allowed_envs`);
     - no `DSTACK_AUTHORIZED_KEYS` anywhere;
     - a `pre_launch_script` that is empty, or that does not write
       `authorized_keys` or set a password.
     A deploy that fails any of these is torn down. Its measurement is never
     recorded or pinned. Toggling a flag changes `compose_hash`, so read again
     after any change.
  4. Commit the read-back `compose_file` verbatim as
     `deploy/judgement/compose-readback.<profile>.json`. A CI script
     (`scripts/ci/check-judgement-compose-readback.sh`, with self-tests)
     refuses a committed read-back that fails step 3's rules.
  5. Read `compose_hash` from the same output. Then **poll** `GET
     /v1/attestation` with fresh nonces until a verified quote's `mrconfigid`
     carries that `compose_hash` (the `01 || compose_hash` form the witness
     README records). Only then record the app id and measurement. This is
     the witness's stale-container trap: for about a minute after an upgrade,
     the old container answers with the old measurement. The signing address
     and sealing key are per boot, so they are not recorded or pinned.
  6. The drill (section 10) repeats step 3 from the quote: it hashes the
     committed read-back for the pinned measurement, requires the hash to
     equal the `compose_hash` in the live quote's `mrconfigid`, and checks
     the step 3 fields in that file. A pinned `mrconfigid` therefore implies
     the hardened manifest, and anyone can check it.
- **Pin.** Add the production measurement (`mrtd`, `mrconfigid`) to the
  daemon's compiled-in set in a release. Development pins go only into development
  config.
- **Upgrade.** In place, same app id. Spend carries over. Release daemons
  refuse the new measurement until a release adds it. Ship that release, then
  drop the old measurement in a later release.
- **Rotate.**
  - NEAR AI model key. NEAR AI does not announce new keys ahead of time, so
    they cannot be pinned in advance. **A rotation pauses judgements**: the
    first failing receipt latches the model (section 5), the drill reports
    `judgement_model_latched`, and nothing more is spent. Until open
    decision S1 is answered, the recovery is to set the new key in
    `TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS`, redeploy (a new measurement),
    and ship a daemon release carrying that measurement. Judgements stay
    paused until the release reaches contributors.
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
    crash between reserve and settle; reported usage above the reservation;
    many concurrent reservations on one account and on the operator total,
    where exactly as many succeed as fit; a failed receipt with `usage {0,0}`
    that still charges the reservation; a future-dated clock whose month is
    never persisted;
  - ledger seal round trip and refusal on a wrong key, and a ledger written
    under one boot's keys read back under the next boot's;
  - per-boot keys: two boots give different `seal_pk` and `signing_address`,
    and neither is ever derived from `GetKey` (a structural test over
    `enclave.rs`);
  - the check order: a body that does not open, a bad claim and a replayed
    request nonce each take no judgement permit, and a slow body is cut at
    10 s;
  - the boot rules, each refusing;
  - the production allowlist test with the development feature off;
  - prompt-hash pins;
  - the signing-bytes encoder against a fixed vector;
  - a log-capture test that no line carries the account, a label or digest
    text.
- **Receipt path**, against the captured live fixtures the attestation crate
  already uses for `provider_tee`, plus refusal tests for `gateway`,
  two-part, wrong model and unpinned signer.
- **Protocol and attestation crate:** the exact refusal strings, and
  `is_judgement_capacity`, pinned like `witness_pacing::tests`. The judgement
  seal against frozen vectors (fixed `eph_sk`, `seal_sk`, plaintext), a
  tampered ciphertext, AAD or `eph_pk` refusing to open, and a response that
  does not open under another request's keys. The `boot_binding` report-data
  layout against a frozen vector.
- **Client:**
  - `check_quote` with both report-data layouts, and cross-layout refusal;
  - the misrouting scenario of #1501 review point 1: a quote from instance P,
    a request answered by instance M with other per-boot keys. M cannot open
    the body, and a response M signs is discarded;
  - each response check of section 4 discards on its own: wrong signer, wrong
    request nonce, a response for another digest, an `event_index` out of
    range, a wrong wire version;
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
    verification from section 4, then seals its request to the quoted
    `seal_pk`;
  - mints a drill claim for a dedicated drill account;
  - sends a fixed synthetic digest and verifies the signature;
  - requires every result to carry a verified `provider_tee` receipt and the
    expected judge versions;
  - hashes the committed `compose-readback.production.json` and requires it to
    equal the `compose_hash` in the live quote's `mrconfigid`, with
    `public_logs` and `public_sysinfo` false, `allowed_envs` exactly the NEAR
    AI key, and no SSH pre-launch step (section 9, "Deploy").

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
   limiter `GetKey` path and the per-boot keys, the `tcjudge1` attestation
   route, `deploy/judgement/` (its README carries section 9's deploy steps,
   the read-back rules and the stale-container poll), `judgement-image.yml`
   with both profiles, and the pre-push boot checks.
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

**What the owner decision on digest contents blocks.** The addendum's O1 (what
leaves the machine) is open. Until it is decided, nothing that sends a digest
merges: not the digest builder (#1525) and not slice 7 (the daemon client).
Slices 1 to 6 and 8 carry no contributor data and may proceed.

## 12. Decisions and open questions

**Answered** (the owner decisions at the top of this spec): 1, the interim
production model is none, and production waits for the decision model; 2,
production pins are compiled into the daemon; 3, the NEAR AI gateway is
verified at each boot (section 5, check 5); 4, the operator budget is $100 a
month; 5, pooled tenants stay excluded in v1; 6, the Phala volume behaviour is
checked, and the runbook upgrades in place. The addendum's decisions 1, 3, 4
and 5 stand as recorded there.

**Open, for the owner:**

- **The addendum's O1 to O4:** what leaves the machine, the folder rule, the
  consent wording, and how existing Insights users are told. O1 blocks the
  digest builder (#1525) and slice 7.
- **K1 to K3, the `stuck` judgement** (section 14): its consent, whether
  `usual_calls` leaves the machine, and its ask cap. They block the
  economizer's `stuck` client, not slices 1 to 6.
- **S1. Updating NEAR AI pins without a daemon release.** Today a NEAR AI
  key rotation latches the model (section 5) and needs a redeploy plus a
  daemon release, because the pins sit in the measured compose. Options:
  - (a) keep that. Rotations are free, because of the latch, but judgements
    pause until the release reaches contributors;
  - (b) a pin document signed by an offline Trace Commons key whose public
    half is in the measured compose. It holds the model signer keys and the
    gateway measurement pins, and arrives through a new route whose only
    authentication is that signature. Its version number is kept in the sealed
    ledger and only ever increases. Every key it names must still be
    quote-bound in a live, DCAP-verified NEAR AI report before use, so the
    offline key can choose only among keys NEAR AI's TEEs attest.

  Recommendation: (b), because NEAR AI rotates without notice. If taken, it
  reverses owner decision 2's "no signed list" for the service's NEAR AI pins
  only. The daemon's pins of this service stay compiled in.
- **S2. A direct `rustls` dependency in `trace-commons-server`,** for the
  gateway TLS pin (section 5, check 5). No package is added to the lockfile,
  because `trace-commons-ingest` already declares the same requirement. It is
  still a new direct edge, so it needs approval under the dependency policy.
  Without it, check 5 cannot be built, and a NEAR AI-hosted model cannot boot.
- **S3. RFC 9180 HPKE instead of the judgement seal.** The `hpke` crate would
  be a new dependency, and RFC 9180 has no secp256k1 KEM, so the per-boot key
  would become X25519 through a crate that supports static keys. The
  recommendation is to keep the seal of section 3, which uses only `k256` and
  `ring`, unless a reviewer wants the standard construction enough to take a
  new crate.

## 13. Changes the open slice PRs owe this revision

The 2026-10-21 revision changes slices that are already open. This list
combines the #1501 review with the slice reviews on #1529, #1530 and #1531.
The slice fixers work from it. Paths are relative to the repository root.

### #1527, slice 1: protocol (`crates/trace-commons-protocol/src/insights_judgement.rs`)

1. Drop `JUDGEMENT_NONCE_HEADER` from the judgement route. Keep
   `JudgementNonce` for the quote nonce only. Add `JudgementRequestNonce` (32
   bytes, hex on the wire).
2. Add the closed request plaintext `{wire_version, request_nonce, claim,
   digest_b64, judgements}` (unknown fields refused). The digest cap applies to
   the decoded bytes. Add `JUDGEMENT_SEALED_REQUEST_MAX_BYTES = 16_434`, which
   is the 16 KiB plaintext cap plus 50 bytes of seal overhead.
3. Add `request_nonce` to `JudgementResponse`. `judgement_signing_bytes` signs,
   in order, `wire_version`, `request_nonce`, `digest_sha256`, every result
   field, the `spend` bucket and `signed_at`. Re-freeze `FROZEN_VECTOR`, and
   add tests that changing `wire_version`, `spend` or `request_nonce` changes
   the bytes.
4. Add four refusals: `judgement_seal_stale` (400), `judgement_request_replayed`
   (409), `judgement_quote_unavailable` (503, the attestation route only, not
   a capacity label) and `judgement_service_fault` (500, not retryable today).
   `ALL` grows to 14. `is_judgement_capacity` stays true for exactly the three
   capacity labels. The string pins are updated.
5. Change the attestation response type to `{quote_hex, seal_public_key,
   signing_address}`.
6. Add a pure response-binding check for slice 7 and the drill to share:
   given the sent digest bytes, the request nonce and the digest's
   event-marker count, it requires the `digest_sha256`, the `request_nonce`,
   `wire_version == 1`, each requested judgement exactly once, and every
   `event_index` below the marker count (only the first 4 judged).

### #1528, slice 2: shared verification (`crates/trace-commons-attestation`, `crates/trace-commons-contributor/src/witness/verify.rs`)

1. In `report_data.rs`, make the layout a parameter (a `ReportDataLayout`
   with `Witness` and `Judgement`) rather than only the domain tag. The
   judgement layout is `"tcjudge1" | SHA-256("trace_commons.insights_judgement.boot_keys.v1\n"
   || seal_pk(33) || signing_address(20))[0..24] | nonce(32)`, with the nonce
   at `[32..64]`. The witness layout and its tests do not change.
   `check_report_data` takes the layout.
2. Add `judgement_seal.rs` (section 3): ECDH by k256 point arithmetic (no
   `ecdh` feature), HKDF-SHA256 from `ring::hkdf`, ChaCha20-Poly1305 from
   `ring::aead`, and zero nonces with single-use keys. The API is one-shot
   types: sealing a request returns a response opener that `open` consumes,
   and opening a request returns a response sealer. Tests: frozen vectors (a
   test-only constructor injects `eph_sk`), refusal of a tampered ciphertext,
   tag, `eph_pk` or version byte, an identity point refused, and a response
   that does not open under another request's keys. Confirm that `cargo tree
   -p trace-commons-attestation` and `Cargo.lock` are unchanged.
3. `check_quote` takes the layout and its inputs. The witness passes
   `Witness`. New tests: a judgement quote is accepted, a witness quote is
   refused as a judgement quote, and the reverse.
4. The `license_boundary.rs` expected sets do not change.

### #1529, slice 3: service library (`crates/trace-commons-server/src/judgement_service/`)

1. `model.rs`: `RawDecision` loses `answer` and `usage`, and the trait gains a
   pure `parse_answer(&call, response_body)`. `receipt_check.rs` builds the
   label, confidence and usage from `response_body`, and only after
   `check_receipt` passes. The mock's faults change the body and re-sign the
   fake receipt over it. New test: the label follows the body. (#1529 review
   point 2.)
2. `receipt_check.rs`: change the order to reserve, call, check the receipt,
   then settle. On a verified receipt, settle to the parsed usage. On a
   failed receipt, charge `max(reservation, reported)`. Test: `ResponseSwapped`
   with `usage {0,0}` still charges the reservation. (#1529 review point 4.)
3. `production.rs`: add a per-model latch on `ProductionCheck`. It is set on
   `SignerUnpinned`, `NotProviderTee`, `WrongModel`, and verified usage above
   the reservation. `call_check` refuses a latched model before reserving,
   and only new pins clear it. Test: after one `SignerUnpinned`, the next
   `judge` makes no model call (the mock's `calls()` does not move) and
   charges nothing. `an_unpinned_signer_is_refused_but_still_metered` gains
   that second-call assertion. (#1501 point 14, #1529 point 1.)
4. `limiter.rs`: reserve `(request_bytes + 512) * input_price + max_tokens *
   output_price`, with a named constant for the 512-token template allowance.
   Settle may exceed the reservation only by charging the reported figure,
   logging `judgement_usage_over_reservation` and setting the latch. Correct
   the "upper bound" docs (`limiter.rs` module docs, `model.rs`) to state the
   one-token-per-byte assumption. Tests: usage above the reservation; N
   concurrent reservations of which exactly k fit, on one account and on the
   operator total; two devices on one account. (#1501 point 13, #1529 point 3.)
5. `limiter.rs` `roll_month`: advance the month only inside a reservation, only
   to the month of the verified claim's `iat`, and never more than one month
   past the last persisted month unless `iat` is in that month. Expose
   `ledger_month()` for `reset_at`. Tests: a future clock's month is never
   persisted, and `reset_at` derives from the ledger month. (#1529 point 5.)
6. `prompts.rs` `material_sha256`: also hash `JUDGEMENT_MAX_TOKENS`, the
   temperature and reasoning settings, the per-kind fallback label, the
   fallback confidence and the decision thresholds. Re-pin the three
   versions. (#1529 point 6.)
7. `ledger.rs`: the associated data drops `signing_address` and uses the fixed
   label `insights-judgement/ledger` with the file name and the format
   version. The constructor stops taking an address. Test: a ledger written
   under one set of per-boot keys reads back under another.
8. `service.rs`: `judge` takes the opened plaintext, with the claim inside,
   not headers. Add the request-nonce replay set: 20-minute TTL, at most
   65,536 entries, and `judgement_saturated` when it is full.
9. `claim.rs`: accept `now <= exp + 60`, `exp - now <= 960` and
   `iat <= now + 60`. Expose `iat` for the limiter. Tests verify at the
   issuer's time minus 30 s and plus 30 s. (#1530 point 2, verifier half.)
10. `production.rs` or `catalog.rs`: `PRODUCTION_MODEL_ALLOWLIST` is an empty
    constant, with a test that it is empty. Remove any Qwen production
    wording. (#1501 point 15.)

### #1530, slice 4: issuer (`trace_upload_claim_issuer.rs`, protocol `insights_judgement.rs`, `deploy/pilot-gcp/Caddyfile.template`)

1. Clamp the claim lifetime to `min(max_ttl, 840)`, that is
   `JUDGEMENT_CLAIM_MAX_LIFETIME_SECS - JUDGEMENT_CLAIM_CLOCK_SKEW_SECS`.
   Test with slice 3's verifier at `issuer_now - 30` and `issuer_now + 30`.
   (#1530 point 2.)
2. Add a closed `JudgementClaimRefusal` (label, status, retry class) to the
   protocol module. The handler returns only its labels, including for
   today's 503 registry text and its 500 "failed to issue upload claim". A
   test pins the strings. Correct the doc at `insights_judgement.rs` that
   called them the upload-claim route's labels. (#1530 point 3.)
3. Log no tenant, account, device key id or `device_ref` on the judgement
   claim path. That includes `device_public_key_bytes`'s malformed-key line
   when this route calls it: log a fixed label instead. Add a tracing-capture
   test that mints once, hits each refusal, and asserts no captured line
   contains the tenant id or the device key id. (#1530 point 1, #1501 point
   10.)
4. In `Caddyfile.template`, add `request>headers>X-Trace-Device-Key-Id delete`
   and `request>headers>X-Trace-Device-Signature delete` to the issuer host's
   log filter, and to the ingest host's for parity. Narrow the handler comment
   and the PR body: the issuer process still sees which account mints a claim,
   and when. (#1530 point 1.)

### #1531, slice 5: binary, enclave seam and image

1. `enclave.rs`: remove `JUDGEMENT_SIGNING_KEY_PATH` and the signing `GetKey`.
   `JudgementEnclave::connect` generates the sealing and signing keys in
   memory (`ring::rand::SystemRandom` into `k256::SecretKey`) and builds the
   judgement report-data layout from slice 2. The boot self-check compares
   that layout. Tests: two connects give different keys, and a structural
   test asserts no `GetKey` call for signing. (#1501 point 1, #1531 point 1.)
2. `http.rs` attestation route: return `{quote_hex, seal_public_key,
   signing_address}`. Put a 4-permit semaphore and a 10 s
   `tokio::time::timeout` around `identity.attest`, and map either limit to
   `judgement_quote_unavailable`. (#1531 point 7.)
3. `http.rs` judgement route: `application/octet-stream`, with no Bearer or
   nonce header. Read the body under its own 32-permit semaphore, with a 10 s
   timeout and a 16,434-byte cap, before taking any judgement permit. Then
   open the seal (`judgement_seal_stale` on failure), parse, verify the
   claim, check replay, and only then `try_acquire` a judgement permit.
   Responses are sealed under `k_resp`; refusals stay plaintext. Test: a slow,
   unauthenticated body holds no judgement permit. (#1531 point 4.)
4. `http.rs`: a signing or serialization failure after settle returns `500
   judgement_service_fault` with no `Retry-After`. (#1531 point 6.)
5. `boot.rs`: any adapter that calls NEAR AI refuses boot with
   `missing_control:judgement_near_gateway_pin` until check 5 is compiled in
   and configured. Check 5 itself (the fingerprint-mode report, DCAP, the
   gateway measurement pin `TRACE_COMMONS_JUDGEMENT_NEAR_GATEWAY_MEASUREMENTS`,
   the SPKI binding, and the pinned verifier on the adapter's client) lands
   before slice 9. It waits on owner decision S2 (the `rustls` edge) and on a
   live fingerprint-mode fixture. (#1501 point 2, #1531 point 2.)
6. `boot.rs`: refuse boot when the account cap or the operator budget is
   unset or unparseable, and when a production build has an empty allowlist.
   (#1501 point 15.)
7. `deploy/judgement/docker-compose.yml`: set
   `TRACE_COMMONS_JUDGEMENT_OPERATOR_MONTHLY_BUDGET_USD=100`.
8. `deploy/judgement/README.md`: add a Deploy section with the exact `phala
   deploy --kms-id <id> --no-public-logs --no-public-sysinfo` and upgrade
   commands. Add the mandatory `phala cvms get --json` `compose_file` read-back:
   `public_logs` and `public_sysinfo` false, `allowed_envs ==
   ["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"]` (development: `[]`), no
   `DSTACK_AUTHORIZED_KEYS`, and no SSH pre-launch step. No measurement is
   recorded unless the read-back passes. Also add the committed read-back
   file, the stale-container poll, the ledger-reset log, the recovery steps for
   `judgement_limiter_unreadable` and `judgement_limiter_missing`, the
   volume-loss residual risk, and "a NEAR AI key rotation pauses
   judgements". Drop the signing address from the measurement table.
   (#1501 points 6 and 14, #1531 points 3 and 8.)
9. Add `scripts/ci/check-judgement-compose-readback.sh` with a self-test.
10. Add `scripts/ci/check-release-judgement-features.sh` with a self-test, wired
    into `ci.yml` beside `check-release-credential-store.sh`. It refuses
    `judgement-development` and `judgement-development-models`, and
    `--all-features`, on release and image build lines. It refuses either
    feature in a shipped crate's `default`, and a production Dockerfile arm
    that is not exactly `near-attestation-collateral`. (#1501 point 7, #1531
    point 5.)
11. Tests for the misrouting scenario at the service seam: a request sealed to
    instance P's key does not open at instance M.

### Outside the five service slices

- **#1525 (the digest builder)** does not merge until the addendum's O1 is
  decided. Under option B it also needs: the scrub inside the builder
  (`deterministic_only`, the masker, the residual gate, the whole text before
  the cut); byte budgets of 320 and 240 bytes; 64 saturated turn-outcome
  rows; 16 event markers; and the fixtures of the addendum's "The scrub" and
  "Size".
- **Slice 7 (the daemon client)** does not merge until O1 is decided. It
  implements section 4's checks, the one-hour re-verification, the fixed
  claim-mint schedule (section 6), the folder rule, and the send-time
  in-memory build with no digest on disk (addendum).
- **Slice 8 (the drill and runbook)** adds `judgement_model_latched` and the
  read-back-versus-quote check to the drill.

## 14. The `stuck` judgement (added 2026-11-01)

The contributor economizer (#1818) watches a managed session's turn and can
tell the agent when it is going round in circles. Its rules catch the plain
cases locally. For the unclear middle it asks this service one question:
is this turn stuck? That is the fourth judgement, `stuck`.

### What changes on the wire (slice 1, #1527)

- `JudgementKind::Stuck`, wire `stuck`, signing tag 4, with labels `stuck`,
  `productive` and `unclear`. A `stuck` request carries `stuck` alone.
- Its digest is a `TurnDigest`, not a session digest: counts for the running
  turn (calls, edits, files edited, most edits to one file, failures,
  repeated failures, checks, whether the latest check since the last edit
  failed, optionally `usual_calls`) and the kind, failed and repeated flags
  of the last 24 calls. It holds no prompts, paths, commands, file contents
  or output. It parses closed, its counts must be consistent, and it must be
  sent in its canonical encoding, so nothing can be padded onto it.
- Both ends refuse a `stuck` request whose digest is not a valid canonical
  turn digest, and refuse any other judgement whose digest names a turn
  digest version. A newer turn digest is `wire_version_unsupported`.
- Wire v1 is not frozen: no daemon client has shipped and the production
  allowlist is empty. After a client ships, a wire change bumps the version.

### What changes in the service (slice 3, #1529)

- Pinned prompt `stuck-v1`, written for a decision model; under the
  development chat adapters it also asks for a confidence bucket.
- The model sees the turn digest re-encoded, never the bytes sent.
- A `stuck` request logs no digest hash: a turn digest is small integers in
  a fixed layout, so its hash could be reversed by enumeration.
- It is charged to the same account month as the other judgements, under
  the same limiter. K3 bounds how much of that it may use.

### Owner decisions it needs (open)

- **K1. Consent.** A turn digest leaves the machine while the turn runs,
  possibly more than once per turn, under the judgement claim, so the
  service learns the account's working cadence. Recommendation: it rides the
  Insights judgement consent (addendum step 5), whose copy gains one
  sentence saying a running turn's counts may be checked. The wording goes
  to the owner under the copy guide's rule 9.
- **K2. `usual_calls`.** It is the 90th percentile of this folder's past good
  turns, so it is cross-session history. Recommendation: do not send it.
  The daemon keeps it local and the judge decides from the turn alone; the
  field stays optional on the wire so this can be revisited.
- **K3. Ask cap.** Each ask is a paid call from the $5 account month.
  Recommendation: the daemon asks at most 2 times per turn and 30 times per
  UTC day, and the service enforces the same daily count per account key in
  the limiter, refusing past it with `judgement_saturated` so it never eats
  the budget the session judgements need.

Until K1 to K3 are decided, the economizer's `stuck` client does not send,
and the production allowlist stays empty under owner decision 1 in any case.

