# The redaction witness client

The half of the witness that runs on a contributor's machine: it verifies the
enclave's measurement, and **only then** hands that enclave a raw, unredacted
agent session. It receives back a redacted envelope and a signed certificate,
and forwards both to ingest.

The service half is
[`2026-09-02-redaction-witness-service-design.md`](2026-09-02-redaction-witness-service-design.md);
its plan's "Not in this plan" names this work. Read that spec's "Trust model"
and `deploy/witness/README.md`'s "What the witness sees, and what a compromise
costs" first. This document does not repeat them; it depends on them.

## The one property this design exists to hold

**The contributor sends the enclave their raw transcript.** Today those bytes
never leave the machine: `redact_to_envelope` runs locally and only the
redacted envelope is uploaded. This design deliberately gives them away, to a
service whose compromise -- a bug in its binary, a bug anywhere in the
dependency tree it links, a host escape, an operator who turns on container
logs -- hands an attacker whole unredacted sessions. That is a larger blast
radius than anything else in this system.

The only thing that makes it a reasonable trade is that the contributor
verified the measurement first. So the ordering is the design:

> **Verify the quote, and only then send anything.**

A client that sends first and checks after is not a weaker version of this
design. It is the absence of it: the raw bytes are already gone by the time the
check runs, and every outcome of the check is then a report about a disclosure
that already happened.

Stating that as a rule is not enough -- this repository keeps finding its
conventions at the bottom of its defects. So it is enforced by types, in the
shape the witness service already uses at its own edges (`ContributorNonce`
cannot be built except by parsing 32 bytes of hex; a certificate's digest
cannot come from anywhere but a `CorrespondenceProof`):

- `VerifiedWitness` is the only argument type that lets raw bytes be sent, and
  its fields are private.
- Its only constructor runs DCAP verification, checks the quote's report data
  against the nonce this client generated, and checks the measurement against
  a configured pin.
- The function that transmits a raw contribution takes `&VerifiedWitness` and
  is in a module that cannot construct one.

"Send first, check after" is then not a discouraged ordering. It does not
compile.

## What already exists, verified against this tree on 2026-09-02

Everything below was read out of the worktree, not recalled.

- **`crates/trace-commons-attestation` is permissive and does the verification
  half.** `quote::verify_quote(quote, collateral, now_unix) -> VerifiedQuote`
  exposes `mrtd`, `mr_config_id`, `rtmr[0..4]`, `report_data`, `tcb_status`;
  `measurements::ExpectedMeasurements::from_env_value` parses a
  `mrtd=<hex>,mrconfigid=<hex>` list; `measurements::check_measurements_opt`
  returns `MeasurementVerdict::Refused { control }` when nothing is pinned.
  `receipt::recover_eip191_signer` and `receipt::decode_address` are there for
  the signature half. `VerifiedQuote`'s fields are all public, so a test can
  construct one without a real quote.
- **The witness's report data layout is fixed and public**
  (`witness_service/enclave.rs`): 64 bytes, `b"tcwitns1"` at 0, the 20-byte
  signing address at 8, the 32-byte contributor nonce at 28, four zero bytes
  after. A client reconstructs those bytes and compares; a quote that does not
  carry this client's nonce is a replay.
- **The witness measurement string is `mrtd:<96 hex>+mrconfigid:<96 hex>`**,
  composed by the witness from its own boot quote. It is what
  `WitnessCertificate::claimed_witness_measurement` carries and what
  `WitnessPin` compares byte for byte.
- **The two witness routes are `POST /v1/witness` and
  `GET /v1/attestation?nonce=<64 hex chars>`**, both unauthenticated, no TLS of
  their own (dstack-gateway terminates). `/v1/attestation` returns
  `{"quote_hex", "signing_address"}` and deliberately **not** the measurement
  label -- a contributor must read the measurement out of the quote they
  verified, never off a field beside it.
- **The client's redaction seam is `redact_to_envelope`**
  (`trace-commons-contributor/src/envelope.rs:229`), with exactly two
  production call sites: `submit.rs:558` and `daemon/preview.rs:602`. Every
  other call is in tests. That is the seam this design replaces.
- **`residual_pii_risk` travels inside the envelope**, at
  `envelope.privacy.residual_pii_risk`, in the `POST /v1/traces` body
  (`submit.rs:1285`). It is a typed field of `PrivacyMetadata`, not a header
  and not a sibling of the envelope.
- **`trace-commons-contributor` already has `reqwest` 0.12 with
  `rustls-tls-native-roots`, `ring`, `hex`, `serde_json` and `sha2`.** The
  transport and the randomness this design needs are already present.
- **`trace-commons-attestation` is already in the permissive set pinned by
  `tests/license_boundary.rs:242`.** Depending on it from a contributor crate
  is inside the boundary.

## What the lead believed, and what the tree says

The brief for this work carried six claims. Four hold; two are wrong in ways
that change the plan.

**Holds.** There is no real measurement to pin: nothing in this project has run
on a real CVM, nothing has spoken to a live dstack guest agent, the exercised
image build was arm64, and the compose-hash derivation is unconfirmed --
`deploy/witness/README.md` says all four in its own words.

**Holds.** The client belongs in the permissive `trace-commons-contributor`
crate. The submission path is there, the daemon is there, and all three shells
reach it: the macOS app hosts the daemon (`macos/Sources/TraceCommonsApp/DaemonHost.swift`),
the GTK shell depends on the crate directly, and the FFI surface is three
functions wide and carries no submission of its own. Refinement, not
correction: the change lands at `redact_to_envelope`'s two call sites, one of
which is the daemon's **preview** path, which builds the envelope long before
the upload happens. See "The approved-envelope path" below.

**Holds.** Spans are retired. The witness performs the redaction and returns
the result; nothing in this design sends a span list, and
`DeterministicTraceRedactor` does not produce one to send.

**Holds.** Whole-transcript versus per-turn is unsettled *as an API question* --
but see the correction below, which settles it for this client.

**Wrong: the sizing premise.** The 3.4:1 raw-to-envelope ratio and the 7% of
sessions over the 16 MB cap are real numbers, and they do not reach this
client. `raw_contribution_size_ok` (`envelope.rs:269`) refuses any raw
contribution whose serialization exceeds `MAX_ENVELOPE_BYTES` --
`MAX_TRACE_ENVELOPE_BYTES`, 16,000,000 -- **before** the redaction pass runs.
So this client never holds a raw payload above 16 MB that it would have
occasion to send, the witness's 64 MiB bound is four times what it can offer,
and the "measure a real 60 MB POST" gate does not block this work. It blocks
raising that pre-redaction ceiling, which is a different change with a
different reason. What this client must do instead is refuse locally at its own
bound rather than discover the witness's, and say so by name.

**Wrong: that the merged service can serve this client at all.** Two gaps, both
in the service half, both prerequisites here.

## The two gaps in the merged service

### The wire shape is text, and the client's redaction is not

`POST /v1/witness` takes `{raw_transcript: String, consent}` and returns a
redacted `String`. The client's redaction takes a `RawTraceContribution` and
returns a `TraceContributionEnvelope`. These are not the same operation and one
cannot be substituted for the other:

- `redact_trace` walks events individually, applies tool-payload profiles by
  tool name, canonicalizes structured payloads, computes
  `redaction_hash(events, counts)`, builds a trace card, and derives
  `residual_pii_risk` from the merged report. `redact_text` is one pass over a
  flat string and produces none of that.
- **A correction is deliberately not scrubbed** (`trace_contribution.rs`, the
  S5 rule): neither rewriting pass runs over `outcome.human_correction`, and a
  credential-shaped correction is *refused* rather than masked. A text pass
  over a serialized `RawTraceContribution` would rewrite the correction, which
  is a behaviour change to the one field the pipeline is careful not to touch.
- The certificate's digest must match bytes the server holds. If the witness
  returns a string, the client must parse it back into an envelope and
  re-serialize, and the digest survives only if that round trip is byte-exact.
  It is not worth depending on that.

**The service must accept a `RawTraceContribution` and return a
`TraceContributionEnvelope`.** Both types are in the permissive protocol crate,
so the AGPL witness may use them.

And the digest must be taken over something the client does not subsequently
change. It cannot be the serialized envelope as uploaded: `stamp_granted_scopes`
rewrites the envelope after redaction, and rewrites it again on a claim re-mint
during upload retry (`submit.rs:1259`). The stable target is the bytes
`redaction_hash` already covers -- `serde_json::to_vec(events) ||
serde_json::to_vec(counts)`, taken after `canonicalize_event_payloads`. Those
are exactly the redaction-determined bytes, they are unaffected by scope
stamping, and the server can reconstruct them from the envelope it receives.

### Nothing supplies Intel collateral to the client

`verify_quote` takes collateral as a parameter. `dcap-qvl`'s own collateral
client is behind the `collateral-client` feature, and the server's manifest
records why it must not simply be turned on here: that feature pulls a second
`reqwest` with `rustls`/aws-lc-rs alongside this workspace's ring provider, and
rustls then refuses to guess and **panics at the first TLS use** unless a
binary installs a default provider explicitly. `trace-commons-contributor` is a
library consumed by a CLI, a GTK binary, a Swift app and a Windows shell. A
landmine that only a `main()` can defuse does not belong in it.

`/v1/attestation` returns a quote and no collateral, so as merged there is
nothing for a client to verify against.

**The hosted ingest server serves the collateral.** It already builds with
`near-attestation-collateral`, already installs the ring provider in `main()`,
and already talks to a PCCS. A new unauthenticated route takes a quote and
returns Intel's collateral JSON for it. Collateral is Intel-signed and its
validity window is checked against the clock the *client* passes, so a
malicious or stale intermediary cannot forge it and can only withhold it -- and
a client that gets no collateral refuses, which is the correct direction.

Serving it from the witness instead was considered and rejected: it would give
the enclave an outbound dependency on Intel's PCS, enlarge the image and
therefore the measurement, and add a second thing that must be reachable from
inside the CVM. The witness's smallness is load-bearing.

## What to pin, and what not to

`deploy/witness/README.md`'s "What to pin, and what not to" is authoritative
and this design follows it rather than inventing a policy.

**Pin MRTD and MRCONFIGID.** MRTD is the dstack OS image; MRCONFIGID commits to
the compose hash, and the compose file pins the image by digest and carries
every setting the witness reads, so the redaction mode and the body bound are
inside it. An operator cannot quietly downgrade a `full-pipeline` witness to
`deterministic-only` without moving MRCONFIGID.

**Do not pin RTMR3.** It is extended with an `instance-id` seeded from
`getrandom` at deployment, so two instances of byte-identical code differ. A
pin over it fails closed the first time a second replica runs.

**Do not pin RTMR0.** Its event chain hashes SMBIOS tables that scale with `-m`
and `-cpu`, so resizing the CVM fails the pin with no code change at all.

`ExpectedMeasurements` will happily pin either, because an operator nailing
down one instance or one machine shape may want to. This client's configuration
documentation names `mrtd` and `mrconfigid` and says plainly why the other two
are traps.

Two further constraints follow from the README and are carried here rather than
re-derived:

- **A pin is a set, not a value.** dstack derives the signing key from a stable
  app id, so an image upgrade moves the measurement and leaves the signing
  address. The client therefore pins one address and a *set* of measurements,
  and an upgrade is a re-allowlisting. `ExpectedMeasurements` pins one value per
  register, so the client holds a list of `ExpectedMeasurements` and accepts a
  quote that satisfies any member. This is the difference between an upgrade
  and a fleet-wide outage.
- **The measurement is not verifiable against source.** The image is not
  reproducibly buildable and has never been reproduced. A pin proves the
  deployment has not changed under the contributor and that two contributors
  are talking to the same enclave. It does not prove the running code is the
  code in this repository, and no contributor-facing string this client renders
  may say otherwise.

## Fail closed, and what that means at each gate

Every one of these refuses the **submission**, not merely the witness step.
Falling back to local redaction when a witness was configured is not a safe
default dressed as a cautious one: the contributor's bytes stay home, but the
envelope then carries a self-reported `residual_pii_risk` while the contributor
believes it carried a certificate, and the operator sees an uncertified
submission from a contributor they had enrolled as certified. Silence about a
downgrade is the failure this whole design is aimed at.

| Condition | Missing control | Behaviour |
|---|---|---|
| Witness URL set, no measurements configured | `witness_expected_measurement` | Refuse. Send nothing. |
| Witness URL set, no signing address configured | `witness_signing_address` | Refuse. Send nothing. |
| Attestation route unreachable or malformed | `witness_attestation_unavailable` | Refuse. Send nothing. |
| Collateral unobtainable | `witness_collateral_unavailable` | Refuse. Send nothing. |
| Quote fails DCAP verification | `witness_quote_unverified` | Refuse. Send nothing. |
| Report data does not carry this client's nonce | `witness_quote_replayed` | Refuse. Send nothing. |
| Report data names an address other than the pinned one | `witness_signer_unexpected` | Refuse. Send nothing. |
| Measurement matches no pinned set | `witness_measurement_unpinned` | Refuse. Send nothing. |
| Witness host not in the client's host allowlist | `witness_host_not_allowed` | Refuse. Send nothing. |
| Raw contribution over the local bound | `witness_payload_too_large` | Refuse this session. |
| Witness refuses or is unreachable **after** verification | `witness_unavailable` | Refuse this session. |
| Certificate signature does not verify against the pinned address | `witness_certificate_unverified` | Refuse this session. |
| Certificate digest does not match the envelope received | `witness_certificate_mismatched` | Refuse this session. |

The last two matter more than they look. The client verifies the certificate it
is about to forward. It is the only party holding both the raw input and the
returned artifact, and a witness that returned an artifact its own certificate
does not cover is exactly the failure the server cannot detect on its own --
the server would verify a certificate against bytes it holds and find them
consistent, having never seen what was sent.

Every label above is a fixed string. None of them carries a byte count, an
offset, a path, a session identifier or any quantity derived from content; the
existing refusal labels in `submit.rs` set that pattern and it holds here.

## Ship disabled

The switch is `witness` in `ContributorConfig`, `#[serde(default)]`, `None`,
with a `--witness-url` flag and `TRACE_COMMONS_WITNESS_URL` /
`TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` /
`TRACE_COMMONS_WITNESS_SIGNING_ADDRESS` for the environment. **Absent means the
witness path does not exist**: `redact_to_envelope` runs locally exactly as it
does today, byte for byte, and no code on the witness path executes.

There is no "auto", no discovery, and no server-pushed enablement. A
contributor's raw session must never leave their machine because a default
moved, a server advertised a capability, or an update shipped a new config
schema. Turning it on is a local, explicit act that requires the contributor
to hold a measurement pin -- which is the point: a contributor who cannot say
which enclave they trust has not made the decision this design is asking them
to make.

The witness host is additionally subject to the existing `HostAllowlist`
(`config::allowlist_for`), the same gate `issuer_url` and `ingest_url` pass.

## The approved-envelope path

The desktop shells do not build the envelope at upload time. `daemon/preview.rs`
builds it to render the preview card, the contributor approves those exact
bytes, and `submit` later uploads them without re-redacting -- deliberately, so
the bytes shown are the bytes sent.

So the witness call belongs at **envelope-build time**, and the certificate
must be persisted beside the approved envelope and travel with it. A
certificate obtained later would be over different bytes, and a certificate
obtained earlier and not stored is a certificate the upload cannot produce.

This makes the service spec's "the `approved_envelope` path cannot be
witnessed" an artefact of witnessing at upload time. Witness at build time and
it can be, provided the stored artefact is (envelope bytes, certificate,
signature) as one unit and the existing approval fingerprint covers the
certificate too -- otherwise an approved envelope could be paired with a
certificate for something else.

## What travels to ingest, and the rollout overlap

The certificate is **not** a new field on `TraceContributionEnvelope`. Adding
one changes the envelope's serialization and moves the golden digest pinned in
the contributor crate, and it would make the certificate part of the bytes the
certificate is over.

It travels as request headers on `POST /v1/traces`:
`x-trace-witness-certificate` (the certificate's fields as compact JSON) and
`x-trace-witness-signature` (65 bytes of `0x`-prefixed hex). Headers because
the envelope body is exactly what the digest is derived from and must not grow
a field describing itself.

During rollout both shapes are on the wire at once, and that is fine because
the fields do not overlap: `envelope.privacy.residual_pii_risk` remains
present, client-computed, and treated by ingest exactly as it is today. The
certificate is additional evidence a server may choose to weigh. Whether it
does -- and what it licenses -- is the server-side plan named in the service
plan's "Not in this plan", and nothing in this client depends on that plan
having run. A client emitting certificates at a server that ignores them
submits successfully and loses nothing.

Note also the limit that plan will have to respect and this one must not
overstate: the witness's verdict is a **pass** verdict over the originating
redaction, and the classifier can echo a credential back into a field it
rewrites. A certificate cannot license skipping the backstop's trailing
deterministic sweep.

## Whole session or per turn

This client sends the whole `RawTraceContribution` in one request. The reasons
are specific to it and are not a general answer:

- It cannot exceed 16 MB, because `raw_contribution_size_ok` already refuses
  above that before redaction. The witness's bound is 64 MiB.
- The redaction pipeline is not per-turn. `redact_trace` merges one
  `RedactionReport` across all events, and `residual_risk` is derived from the
  merged report and the consent flags together. Splitting the session yields
  per-turn verdicts that cannot be combined into the verdict ingest computes.
- A per-turn shape multiplies the number of requests carrying raw text, and
  each one is an independent opportunity for the transport to be wrong.

**What falsifies this:** raising the pre-redaction ceiling above 16 MB, or a
measured request-body limit below it anywhere on the path (dstack-gateway,
Phala's ingress, or a reverse proxy an operator puts in front). The first is a
deliberate change and would reopen this question with it. The second is
unmeasured today and is one of the things a real deployment must answer before
this client is enabled against it.

## Trust model, stated for the contributor

The contributor is deciding one thing: **that a specific enclave measurement is
worth their raw sessions.** Everything else follows.

- They trust the witness with plaintext, for the life of a request. Nothing
  reduces that to zero.
- They do not trust the transport: the quote is nonce-bound, the collateral is
  Intel-signed, and the certificate is verified locally before it is forwarded.
- They do not trust the server to check the witness for them. The server checks
  a certificate against bytes it holds; only the client can check it against
  what was sent.
- The server continues not to trust the client. That is unchanged and is the
  point of the exercise: today `residual_pii_risk` is authorization by
  self-report, and a certificate replaces the contributor's word with a known
  program's.

## What cannot be verified until a real instance exists

Listed plainly so nothing here is read as tested:

- **There is no measurement to pin.** No pinned set can ship with this code,
  and none is invented. The client refuses without one, so the feature is
  inert until an operator has read a measurement off a running instance.
- **No quote from this witness has ever been parsed.** The report-data layout
  is a public constant in this repository and the client reconstructs it
  exactly, but that a live dstack agent returns a quote carrying it is
  unconfirmed -- nothing in this project has spoken to a live guest agent.
- **The dstack config-id version is unknown**, so MRCONFIGID may be v1 (compose
  hash only) or v2 (additionally app id and key provider). Either pins the
  compose hash, so the pin is sound either way, and no contributor-facing text
  may claim app-id binding until v2 is confirmed.
- **The real body-size ceiling on the path is unmeasured.** It does not block
  this client at 16 MB, and it is not proven either.
- **Whether ingest's PCCS can produce collateral for the witness's platform**
  is unconfirmed; it has only ever been asked for NEAR AI's.

## Open questions

- **A new third-party dependency reaches shipped clients, and needs approval.**
  Depending on `trace-commons-attestation` from `trace-commons-contributor`
  adds **59 packages** to that crate's tree, headed by `dcap-qvl` 0.6.3
  (Phala's, already audited under `deny.toml` on the server side) and including
  `dcap-qvl-webpki`, `x509-cert`, `der`, `parity-scale-codec`, `scale-info`,
  `k256` and `wasm-bindgen`. Counted with `cargo tree -e normal` on both crates
  and differenced. Three consequences to weigh rather than discover: the
  Windows and macOS shells link this through the FFI dylib; the GTK crate is
  its own workspace with its own lockfile and vendored Flatpak set, which
  drifts on any dep change and is only caught by an `app-v*` tag; and
  `dcap-qvl`'s mandatory `std` feature turns on `serde_json/preserve_order`,
  so the contributor crate's standalone build gains an insertion-ordered
  `serde_json::Map`. That last one is survivable -- `canonical_json` exists
  precisely for it, and `redaction_hash` runs after
  `canonicalize_event_payloads` -- but it must be demonstrated, not assumed.
- **Where a contributor gets the measurement to pin.** Reading it off
  `/v1/attestation` is circular: it pins whatever is answering. A pin has to
  arrive out of band -- release notes, a signed operator statement, the
  repository -- and nothing in this project distributes one today.
- **Whether a `deterministic-only` witness is worth calling at all.** Its
  certificate is honest and narrower, and a server requiring the classifier
  will refuse it, so a contributor would be sending raw bytes for a certificate
  that buys nothing. Refusing to send to one whose policy alias is the
  deterministic alias is arguably the right default; it is not settled here
  because the alias is readable only after the exchange.
- **Whether the client should also pin `tcb_status`.** `VerifiedQuote` carries
  Intel's verdict and the advisory IDs. Refusing anything but `UpToDate` is a
  stronger check and a sharper failure the first time Intel publishes an
  advisory. Not decided; the pin config is shaped so it can be added.
