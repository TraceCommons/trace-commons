# The redaction witness service

A service in an attested TEE that receives a contributor's raw agent
transcript, performs the PII redaction itself, and signs a certificate over the
redacted artifact **and its own residual-risk verdict**. Our server verifies
that certificate against bytes it already holds and, when the witness
measurement is pinned, trusts the verdict instead of re-running its own PII
backstop.

This is the authoritative witness spec. It replaces an earlier
correspondence-checking design whose verification core shipped in #533 and is
reused here unchanged; that design's *rationale* did not survive
reconnaissance, and the section below records why, because the reasoning is
worth more than the file was.

## What already exists

Verified against the tree on 2026-09-02.

- **Nothing in this project runs in a TEE.** `trace-commons-gate-enclave` is
  aspirational naming. The witness would be **the project's first real
  trusted-execution deployment**, and the operational cost of that --
  provisioning, measurement management, attestation serving, upgrade discipline
  -- belongs to this design and not to a later slice.
- **The server-side certificate verification is already on `main`** (#533,
  `crates/trace-commons-server/src/redaction_witness/`). It checks a signature,
  a digest against bytes the server holds, and a measurement against an
  operator's pin -- all three or none. It is reused as-is.
- **The redactor is not deterministic.** `DeterministicTraceRedactor` holds an
  optional model-based prose-PII classifier. Its pattern half reproduces; a
  model call does not. This is why the witness **performs** the redaction rather
  than recomputing and comparing one: any witness that replayed the classifier
  would fail on honest submissions. It is the single most important constraint
  here and it invalidates the obvious implementation.
- **`MAX_TRACE_ENVELOPE_BYTES` is 16,000,000.** Raw is larger than redacted, so
  the witness handles payloads above that.

## Why not bind to an inference receipt

The earlier design existed to bind a redacted artifact to a NEAR AI inference
receipt whose hashes cover the raw bytes. **No trace population in this repo
carries such a receipt.**

- The CLI path reads local agent transcripts -- Claude Code, Codex, Gemini CLI,
  Letta trajectory. `git grep chat_id -- crates/` on `main` matches only the
  server crate. There is no NEAR AI request, response, receipt or `chat_id`
  anywhere in the client.
- IronWire (PR #513, unmerged) does not rescue it. Grepping its 5,054-line diff
  for `chat_id|request_hash|response_hash|receipt|signature|raw_request|raw_response`
  yields one hit, in a plan document. `RoutedExchange` carries
  `id, started_at, client_session_id, total_ms, facade, backend, requested_model,
  served_model, rung, attempts, token counts, cost_usd, status`. It records that
  an inference happened. It binds no bytes.
- On that path the client does not hold the bodies either: it polls the proxy's
  log endpoint over loopback and receives only the metadata row above.

So the content-binding story had nothing to bind to. Two consequences follow,
and both are improvements.

**The certificate's inference fields are dropped.** `chat_id`, `model`,
`prompt_tokens` and `completion_tokens` are unpopulatable, not merely
unpopulated. A certificate carrying fields no honest path can fill is an
invitation to fill them dishonestly.

**The witness aims at a live failure instead of a hypothetical one.** Ingest
decides whether to hold a trace from `envelope.privacy.residual_pii_risk`, which
is **client-asserted** -- `rescrub_trace_envelope` re-derives it server-side
precisely because of that. An attested witness that signs its own verdict
replaces a self-report with something verifiable. That is the same trust upgrade
the receipt story promised, pointed at the PII backstop, which is currently
wedged with a large held backlog.

## The exposure argument, corrected

An earlier note held that routing raw text to NEAR AI's classifier adds no new
exposure, because NEAR AI already saw those bytes. **That is true only for
traces that came from NEAR AI inference, and there are none.** For a local
Claude Code session NEAR AI has never seen the bytes, and routing them there is
a new third-party disclosure.

Stated plainly so the decision is made rather than inherited: the pilot today
runs a self-hosted filter on loopback (`TRACE_PRIVACY_FILTER_BACKEND=self-hosted`,
`openai/privacy-filter`) at 4,000-token windows and concurrency 3. NEAR AI's
measured classify limit is 1,000 tokens at concurrency 1, with 2,000-24,000
token requests returning HTTP 502 -- the measurement table is in
`privacy_filter_near_ai.rs:26-79`. Even assuming NEAR AI restores capacity, the
self-hosted path is both more private for these traces and four times the
window.

The classifier backend is therefore a **deployment choice of the witness image**,
not a property of this design. Whichever it is, it is inside the measurement a
contributor pins, which is what makes the choice auditable.

## What the witness does

1. Accepts a raw transcript and the contributor's declared consent flags.
2. Runs the redaction pipeline -- the deterministic secret path plus the prose
   PII classifier -- producing the redacted artifact.
3. Computes the residual-risk verdict with `residual_risk()` from
   `trace-commons-protocol`, the **same function ingest uses**. That crate is
   permissive, so the enclave and the server run identical code and cannot drift.
4. Returns the redacted artifact and a signed certificate.

The contributor uploads the redacted artifact through the existing path. If they
alter it, the digest no longer matches the certificate and the server refuses.

Correspondence checking against a client-supplied span list is **not** part of
this design. The witness produces the redaction, so correspondence holds by
construction, and no span list ever leaves the contributor's machine -- which
also retires the earlier spec's open question about whether a span list's shape
leaks what the detector found.

## The certificate

```
redacted_sha256
residual_risk_verdict
redaction_policy_version
witness_measurement
timestamp
```

`residual_risk_verdict` is the load-bearing field and the reason the certificate
exists.

`redaction_policy_version` is **an alias, never an authority**. Today
`redaction_pipeline_version()` concatenates hardcoded constants selected by
backend family: every self-hosted deployment reports
`ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1` regardless
of which model checkpoint loads, which window size is set, or which detector
regexes shipped. It is a string somebody typed. The **measurement** is the real
policy identity, because the image contains the rule set and pins the classifier
configuration. The server checks the alias against an allowlist and trusts the
measurement.

Signing bytes keep the length-prefixed encoder from #533. The certificate has no
`Serialize`: a dependency enabling `serde_json/preserve_order` shifted every
untyped-JSON digest in this workspace on 2026-09-01, and the encoder exists so
this cannot be caught by that.

## What the server verifies

`verify_witness_certificate` from #533, unchanged in shape: the signature, the
digest against bytes on hand, and the measurement against an operator's pin --
all three or none, with `pin: None` refusing under `witness_expected_measurement`.
A valid signature from an unpinned enclave proves only that some enclave signed
something.

Then, and only for a `VerifiedWitnessCertificate`: if the verdict is clean and
the policy alias is allowlisted, `corpus_status_with_pii_backstop_hold` does not
hold the trace.

Four surfaces read that state and each must be considered:
`Quarantined | AwaitingPiiBackstop` are counted together as `pending_review`;
the contributor receipt says the trace is held pending a verdict; the `Accepted`
gates enforce off stored status; and the requeue route re-enters through it.
Pending credit is already preserved through the hold, so a bypass keeps credit
correct.

## Attestation and pinning

**Pin `MRTD` and `MRCONFIGID`. Do not pin RTMR3. Treat RTMR0 as advisory.**

- `MRCONFIGID` is the stable identity of what code runs. **Which fields it
  commits to depends on the config-id version, and this was measured rather than
  assumed:** the live NEAR AI fixture we verify against emits **v1** -- `01`
  followed by the compose hash and fifteen zero bytes. V2 additionally commits
  to the 20-byte app id and the key-provider identity. Either version pins the
  compose hash, which is the code identity we need, so the pin is sound today.
  **Do not write an operator doc claiming app-id binding until the witness's own
  dstack version is confirmed to emit v2.**
- **RTMR3 is unpinnable across instances**, not merely across upgrades -- it is
  extended with an `instance-id` seeded from `getrandom` at deployment.
- **RTMR0 is a function of VM shape, not only the image**: its event chain
  hashes SMBIOS tables that scale with `-m` and `-cpu`, so resizing the CVM
  fails a pinned RTMR0 closed.

Our verifier does not read `mr_config_id` today. `dcap-qvl` already exposes it
on the parsed report; the work is one `MeasurementField` variant. `verify_quote`
and `check_measurements` are otherwise not NEAR-specific and are reused as-is.

**The signing key survives upgrades.** dstack's KMS derives the app key from
`app_id` alone -- no measurement register participates. `app_id` is the first 20
bytes of the *initial* compose hash and is then persisted, so an upgrade moves
the compose hash and leaves the key. Measurements gate release, not derivation.
So the signer and the measurement are pinned **separately**: an upgrade
allowlists a new measurement without rotating the signing address, and no
operator is ever presented with "disable verification or lose every client at
once".

**The witness must serve its own nonce-bound quote.** `GetQuote(report_data)`
is on the guest agent's unix socket, not network-reachable, and takes 64 bytes.
The witness proxies it on an HTTP route: the contributor supplies the nonce, the
witness packs the nonce plus its signing public key. **Return the raw quote
bytes, not a v1 `VersionedAttestation`** -- 0.5.9 rewired that to msgpack, which
we have no decoder for, and returning raw sidesteps it entirely.

Version drift 0.5.5 to 0.5.11 requires no remediation in our code: we pin
operator-supplied values rather than deriving them, and the RTMR0 work is inside
`dstack-mr`, the tool that *predicts* a measurement. Whoever produces the pinned
hex must run a `dstack-mr` matching the deployed OVMF.

## Trust model

The contributor trusts the witness with raw bytes, grounded the way NEAR AI
grounds our trust in them: the witness publishes a nonce-bound attestation, and
the client verifies the measurement **before** sending anything. A client that
cannot verify must refuse to send, not warn and proceed.

The server trusts the signature and transitively the measurement, and never sees
raw bytes.

Nobody trusts the client. That is the point: today's alternative is a
client-computed `residual_pii_risk`, which is authorization by self-report.

**Residual exposure, stated plainly.** A compromised witness sees every raw
transcript passing through it -- a larger blast radius than anything currently
in this system, and the price of the property. Short retention, no persistence
of raw, memory-only processing and client-side measurement pinning reduce it and
do not remove it.

## The licensing cost

This is the largest unbudgeted item, and it is structural rather than
incidental.

The contributor must verify the witness's quote before sending raw bytes. Quote
parsing and DCAP verification, measurement pinning, and EIP-191 signer recovery
exist **only in AGPL crates** (`crates/trace-commons-server/src/near_attestation/`),
and `dcap-qvl` is declared only in the server's manifest. The contributor crates
are MIT/Apache because they ship inside proprietary harnesses, and
`tests/license_boundary.rs` enforces the direction.

So `quote.rs`, `measurements.rs` and `receipt.rs` must be extracted into a
permissive crate before any client-side verification exists. `receipt.rs` is the
easy one -- k256 plus Keccak, no server types. `client.rs` is coupled to server
config and is the piece to rewrite on `trace_commons_operator_client::Client`
rather than move. `dcap-qvl` is Phala's crate and needs re-auditing under
`cargo deny` on the permissive side.

## Deployment

Phala Cloud, CPU-only TDX. The witness runs no model in-enclave, so no GPU TEE.
`tdx.large` (4 vCPU / 8 GB) is about $169/mo running, plus storage billed while
stopped. Ingress is stable and supports custom domains.

Availability is a real commitment: the witness sits on the submission path for
attestation-gated contributors, and if it is down they cannot submit.
Fail-closed, because a witness bypass is an admission bypass. It holds no
database, no queue and no state beyond its signing key, so it replicates behind
a load balancer and restarts freely. Every stateful concern -- replay, caps,
dedup -- stays on the server where it already lives.

Recorded alternative: GCP is viable only through `dstack-cloud`, never
`dstack-vmm`. A GCP Confidential VM is a TDX *guest*, and a guest cannot be a
TDX host. TDX is verified available in our project (75 TDX-capable images,
Sapphire Rapids in all four us-central1 zones, C3 quota 300 vCPU, no blocking
org policies), at roughly $170/mo, with `--maintenance-policy=TERMINATE`
mandatory because TDX has no live migration.

## Open items

- **Whole-transcript or per-turn.** Phala documents no request-body ceiling, and
  dstack-gateway is a TCP/TLS proxy with no body parser -- its limits are
  `idle = 10m`, `total = 5h`. A 50 MB upload plus classification fits if the
  witness streams rather than going silent. **Measure with a real 60 MB POST
  before committing to whole-transcript.** Raw runs about 3.4x the redacted
  envelope, and 7% of real sessions already exceed the 16 MB cap before that
  multiplier.
- **Is NEAR AI's classify endpoint covered by the same attestation as
  completions?** We verified attestation against the completions path. If
  classify runs elsewhere, that link is unattested. Moot if the witness ships
  the self-hosted filter, which is the current recommendation.
- **The `approved_envelope` path cannot be witnessed.** It reuses a previously
  built envelope and never re-redacts. Witnessing must happen at build time and
  travel with the approved envelope, or that path is permanently unwitnessed.
- **Traffic mix.** Nothing in the tree records expected CLI versus IronWire
  volumes. This is a question for the operator, not the code.
- **Guest-agent v0 versus v1 derive different key material for the same inputs.**
  Pick one client surface and stay on it.
- **Does a signed clean verdict change what the corpus means?** The certificate
  would assert something closer to *sufficiency* than the #533 module was
  willing to claim. That is defensible only while the measurement genuinely pins
  the rule set and the classifier. It must never be described as "verified
  clean".

## Sequencing

1. Add `MRCONFIGID` to the measurement verifier. Small, and everything else
   depends on pinning the right value.
2. Extract quote, measurement and receipt verification into a permissive crate.
   Nothing client-side can verify a witness until this exists.
3. Build the witness service: redact, verdict, sign, plus the nonce-bound quote
   route.
4. Package for dstack and deploy to Phala. Measure the real body-size ceiling.
5. Wire the server-side backstop skip, behind a pin and default-off.
6. Client-side: verify the measurement, refuse if it cannot, then send.
