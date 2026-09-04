# Deploying the redaction witness on dstack

This is the first trusted-execution deployment in this project. Nothing else
here runs in a confidential VM, so there is no house style to copy and no
operator who has already made these mistakes. Read this before you deploy, and
read the two sections that constrain what you may claim — **What a certificate
attests** and **Reproducibility** — before you pin anything or tell a
contributor the witness is safe.

---

## What the witness is for

A contributor will not send a raw transcript to a server. Today the server
therefore takes the contributor's word for whether the redaction was any good:
`residual_pii_risk` arrives as a client-computed field, which is authorization
by self-report.

The witness replaces that with a signature. A contributor sends the **raw**
transcript to an enclave whose measurement they have verified first; the
enclave redacts it, reaches a residual-PII verdict with the same function
ingest runs, and signs a certificate over the redacted bytes with a key derived
inside the enclave. The contributor forwards the redacted artifact and the
certificate. The server verifies the certificate and never holds raw bytes.

## What a certificate attests — and what it does not

A certificate attests **mechanics and a verdict over the originating redaction
pass**:

- the redacted artifact is the SHA-256 the certificate names;
- an enclave reporting a specific measurement produced it;
- that enclave's residual-PII verdict over its own redaction report.

It does **not** attest that the artifact is clean, and no operator surface,
alert or dashboard built on it may say that it does.

**The concrete limit.** There are two orderings in this codebase that apply the
prose-PII classifier. The witness runs the *originating* one — deterministic
secret pass, then classifier, classifier output written verbatim — and only
when it is configured `full-pipeline`; in `deterministic-only` it does not run
the classifier at all, which narrows the verdict further again. The server's
PII backstop runs the other — classifier, then a trailing deterministic sweep.
That trailing sweep exists because the classifier is trained on prose PII and
not on credential formats, so it will write an AWS key straight back into a
field it rewrites. **A credential the classifier itself emits survives the
witness's pass.** That failure is the documented cause of this pilot's entire
quarantine backlog.

So a witness certificate **cannot license skipping the PII backstop
wholesale.** At most it can license skipping the backstop's *classifier* stage;
the trailing sweep must still run, or the skip re-opens exactly the hole the
sweep closes. Deciding that is a server change with its own plan — see
`docs/superpowers/plans/2026-09-02-redaction-witness-service.md`, "Not in this
plan".

If you find yourself writing a sentence about this deployment that sounds
stronger than the three bullets above, that is the sentence to cut.

## What the witness sees, and what a compromise costs

**The witness holds every raw transcript that passes through it.** That is a
larger blast radius than anything else in this system. Ingest holds redacted
envelopes; the gate holds scores; the witness holds the unredacted originals of
whatever traffic it serves, in memory, for the life of a request.

Compromise of the guest — a bug in this binary, a bug anywhere in the
dependency tree it links, a host escape, an operator who turns on container
logs — hands the attacker raw contributor transcripts. Not metadata about them.
Them.

What reduces it, and none of these remove it:

- **Nothing is persisted.** The container's root filesystem is read-only and
  the only writable path is a 16 MiB `noexec` tmpfs. There is no database, no
  object store, no cache and no log of content.
- **No route can be asked what the witness has seen.** There is deliberately no
  health route that reports state, no metrics, and nothing that lists anything.
  A witness that can be interrogated about its history is not one that holds
  nothing.
- **`public_logs` and `public_sysinfo` are off** in `app-compose.json`. dstack
  will serve container logs publicly if asked. Do not ask, on a deployment
  carrying real traffic, for debugging or otherwise.
- **The contributor pins the measurement before sending.** A client that cannot
  verify must refuse to send, not warn and proceed.

**And a sizing point that is part of the blast radius.** The witness binary
links the whole `trace-commons-server` library. It uses a small slice of it,
but the image carries the ingest dependency tree — webauthn, postgres, the
HTTP stacks, `openssl-sys` through `webauthn-rs` — none of which the witness
calls. That is a much larger attack surface than the witness's own code, and it
is the largest single reduction available to a future revision of this
deployment.

## The two routes are unauthenticated. That is deliberate, and it has a cost.

The witness serves exactly two routes, both unauthenticated, unrated and
without TLS of its own:

- `POST /v1/witness` — raw transcript in, redacted artifact plus certificate out.
- `GET /v1/attestation?nonce=<64 hex chars>` — a nonce-bound quote and the
  signing address, so a contributor can verify the enclave *before* sending
  anything.

They are unauthenticated on purpose: authenticating at the witness would give
the witness an identity to correlate against content, which is the one thing
the design is trying not to hand it.

**State the consequence rather than assuming it is understood:**

- `/v1/witness` is **unauthenticated compute over a 64 MiB body**, and that
  compute is a redaction pass over the whole of it. Anyone who can reach the
  route can spend the CVM's cores. In `full-pipeline` mode they can also spend
  your classifier's capacity, and if that classifier is a metered external
  service, your money.
- `/v1/attestation` is **a quote oracle**. Anyone who can reach it obtains a
  fresh TDX quote over a report body of their choosing in the nonce half. The
  quote proves what it says, and nothing about a caller.

A deployment is expected to put something in front of it. `gateway_enabled` is
on, so dstack-gateway terminates TLS — that is the TLS answer and not the abuse
answer. **The abuse answer is not in this directory:** rate limiting per source,
a body-size limit at the edge below the witness's own, and a reachability
decision (public, or only from your contributor shells' egress) are the
deploying operator's, and none of them are configured here. If you deploy this
on a public hostname with no edge in front of it, you have deployed an open
redaction service and an open quote oracle.

---

## Files in this directory

| File | What it is |
|---|---|
| `Dockerfile` | Builds the witness image. Not reproducible; see below. |
| `docker-compose.yml` | The application. **Measured** — every value in it is part of the enclave's identity. |
| `app-compose.json` | dstack's manifest. Generated; embeds the compose file verbatim. |
| `build-app-compose.sh` | Regenerates the manifest, and `--check` fails if it has drifted. |

`docker-compose.yml` is the source of truth and `app-compose.json` is derived.
They are two copies of the same thing and only the second one deploys, so
**run `./build-app-compose.sh` after every compose edit and commit both.**
`./build-app-compose.sh --check` answers "is the manifest I am about to upload
the one this compose file describes" without modifying anything; run it before
a deploy.

---

## Reproducibility — read this before pinning

**This image is not reproducibly buildable. Two builds of the same commit
produce different digests, and therefore different measurements.**

That is worth knowing *before* anyone pins a measurement, because it settles
what a pin means here. A measurement pins a binary. If the binary cannot be
re-derived, the measurement pins **a specific artifact that only its builder
can produce** — it still proves the deployment did not change under you, and it
still proves two contributors are talking to the same enclave, but it does not
let a third party rebuild from source and confirm the running code is the code
in this repository. Anyone auditing this deployment is auditing an image, not a
commit.

The `Dockerfile` narrows the drift rather than removing it. What it fixes:

- **The build timestamp.** `trace-commons-build-info` stamps
  `SystemTime::now()` into every binary. Its build script honours
  `SOURCE_DATE_EPOCH`, and the `Dockerfile` takes it as a build argument, so
  the stamp is deterministic when you pass one. Left unset it defaults to `0` —
  deterministic, and visibly wrong rather than invisibly varying.
- **The dependency set.** `cargo build --locked` refuses to update
  `Cargo.lock`. A build that silently resolved a new patch release would move
  the measurement with no commit behind it.
- **Host paths.** `--remap-path-prefix` keeps the build machine's directory
  layout out of the artifact.
- **The toolchain version**, pinned in the `RUST_IMAGE` tag. There is no
  `rust-toolchain.toml` in this repository, so an unpinned builder image is a
  floating compiler.

What is still not fixed, in rough order of how much it costs:

1. **Base images are pinned by tag, not digest.** `rust:1.96.1-bookworm` and
   `debian:bookworm-slim` are both rebuilt upstream. Override `RUST_IMAGE` and
   `RUNTIME_IMAGE` with `name@sha256:...` to close this one; it is the easiest
   of the four and the largest.
2. **`apt-get install` resolves whatever the Debian mirror serves that day.**
   No versions are pinned and no snapshot mirror is used, so `libssl3` and
   `ca-certificates` float.
3. **Docker layer metadata.** BuildKit will rewrite layer timestamps from
   `SOURCE_DATE_EPOCH`, but this file does not currently drive that, and image
   config ordering is not guaranteed stable across BuildKit versions.
4. **`cargo build` itself has not been demonstrated bit-identical for this
   dependency set on two machines.** It is *mostly* deterministic with the
   above in place; nobody on this project has run the experiment, and until
   someone does that is an assumption rather than a fact.

**Nobody has reproduced this image.** Do not describe the measurement as
"verifiable against source" in any contributor-facing text.

---

## Building and pushing

```sh
cd /path/to/trace-commons-server
docker build \
  -f deploy/witness/Dockerfile \
  --build-arg TRACE_COMMONS_BUILD_COMMIT="$(git rev-parse --short HEAD)" \
  --build-arg SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" \
  -t ghcr.io/OWNER/trace-commons-witness:"$(git rev-parse --short HEAD)" \
  .
docker push ghcr.io/OWNER/trace-commons-witness:"$(git rev-parse --short HEAD)"
```

Then read back the digest and put **that** in `docker-compose.yml`:

```sh
docker inspect --format='{{index .RepoDigests 0}}' \
  ghcr.io/OWNER/trace-commons-witness:"$(git rev-parse --short HEAD)"
```

`docker-compose.yml` ships a placeholder digest of all zeros. It is not a
default to inherit — a tag is a moving target, and a measurement pinned over a
moving target is pinning nothing.

Then regenerate and commit the manifest:

```sh
deploy/witness/build-app-compose.sh
```

The build context is the repository root, not this directory: the image builds
the workspace.

### Check the image before you push it

Three runs, no state of any kind — read-only root, no network, no dstack socket
mounted, no configuration. They take seconds and they catch the failures that
are expensive to diagnose on a CVM.

```sh
img=trace-commons-witness:local
run="docker run --rm --read-only --tmpfs /tmp --network none"

# 1. The runtime image resolves every library the binary links.
$run "$img" --version
# -> trace-commons-witness 0.1.0 (commit <sha>, built <iso8601>)   exit 0

# 2. The redaction mode is required, in both directions.
$run "$img"
# -> error: the following required arguments were not provided:
#      --redaction <REDACTION>                                      exit 2

# 3. Boot is fail-closed: no enclave identity, no listener.
$run -e TRACE_COMMONS_WITNESS_REDACTION=deterministic-only "$img"
# -> Error: could not derive a signing identity from the dstack guest agent
#    Caused by: the guest agent could not be reached                exit 1
```

Run 1 also tells you whether `SOURCE_DATE_EPOCH` took: the timestamp it prints
should be your commit's, not the wall clock at build time. If it is the wall
clock, you passed the build argument wrongly and every rebuild of this commit
will produce a different image.

---

## What to pin, and what not to

The witness reports its measurement as a single string:

```
mrtd:<96 hex chars>+mrconfigid:<96 hex chars>
```

That whole string is what an operator pins, and it is what the server compares
byte for byte (`WitnessPin::new`, in
`crates/trace-commons-server/src/redaction_witness/verification.rs`). Comparison
is exact, including case: a case difference against an honest witness fails
closed and is diagnosable from the reported value, which is better than a
case-folding comparison that could conflate two distinct pins.

### Pin MRTD and MRCONFIGID

- **MRTD** is the measurement of the guest firmware and initial memory — in
  practice, the dstack OS image. It moves when you change dstack versions, not
  when you change the application.
- **MRCONFIGID** is the stable identity of *what code runs*. It commits to the
  compose hash, and the compose hash covers `app-compose.json`, which embeds
  the compose file, which pins the image by digest.

### Do not pin RTMR3

RTMR3 is extended with an `instance-id` seeded from `getrandom` at deployment.
**Two instances of byte-identical code report different RTMR3 values.** It is
unpinnable across instances, not merely across upgrades, so a pin over it fails
closed the first time you run a second replica.

dstack offers `no_instance_id: true`, which would remove that extension. It is
`false` here. Nobody has evaluated what else it changes, and switching it on
for the convenience of one register is not a trade this deployment has priced.

### Do not pin RTMR0

RTMR0's event chain hashes SMBIOS tables that scale with `-m` and `-cpu`.
**Resizing the CVM changes RTMR0 with no code change at all**, and a pinned
RTMR0 then fails closed on a resize that changed nothing about what runs.
Treat it as advisory.

### The config-id version caveat, and it is a real one

MRCONFIGID's contents depend on the config-id version.

- **v1** is `01`, the 32-byte compose hash, then fifteen zero bytes.
- **v2** additionally commits to the 20-byte app id and the key-provider
  identity.

Either version pins the compose hash, which is the code identity we need, so
the pin is sound in both cases. But **a live dstack attestation report captured
during this work — NEAR AI's, on 2026-09-02 — is config-id v1.**

So: **do not claim app-id binding for this deployment** in any operator or
contributor text until the witness's own dstack version has been confirmed to
emit v2. On a v1 deployment, MRCONFIGID says "this compose" and nothing about
which application id it was launched under.

### The configuration is measured, and that is the point

Every setting the witness reads is set in `docker-compose.yml`. `allowed_envs`
in the manifest is empty, so nothing is injectable at runtime.

That means the redaction mode, the body bound, the bind address and the log
level are all inside MRCONFIGID. **An operator cannot quietly downgrade a
`full-pipeline` witness to `deterministic-only`**: doing so changes the compose
hash, changes MRCONFIGID, and a client pinning the old measurement refuses the
new deployment until it is re-allowlisted. Adding an entry to `allowed_envs`
would open exactly that hole, so do not add one without deciding you want it.

### Where an operator reads these values

Three routes, in decreasing order of how much they prove:

1. **The witness's own attestation route** — the authoritative one, because it
   is nonce-bound:

   ```sh
   nonce=$(openssl rand -hex 32)   # exactly 64 bare hex chars, no 0x
   curl -s "https://<witness-host>/v1/attestation?nonce=${nonce}"
   ```

   Returns `{"quote_hex": ..., "signing_address": ...}` — the raw TDX quote
   bytes, hex-encoded, and not a dstack `VersionedAttestation` envelope. dstack
   0.5.9 rewired that envelope to msgpack and we have no decoder for it, so the
   route returns raw and sidesteps it. Parse the quote's TD report body for
   `mr_td` and `mr_config_id`. A quote that does not carry your nonce is a
   replay; the witness's own parser refuses one, and so must yours.

2. **The witness's boot log**, which prints the signing address and the
   measurement string and nothing else. Convenient, and it proves only that the
   process said so.

3. **dstack's own TCB info**, exposed because `public_tcbinfo` is `true`. It
   carries `mrtd`, `compose_hash`, `os_image_hash` and `rtmr0..3`. Note that
   **`tcb_info` has no MRCONFIGID field at all** — that is why the witness reads
   its measurement from a boot-time quote's TD report body rather than from the
   agent's `Info` method. If you are looking for MRCONFIGID in `tcb_info`, you
   will not find it, and its absence is not a fault.

You can also derive the compose-hash half locally without a running instance:
`./build-app-compose.sh` prints the SHA-256 of the manifest. **Compare it
against a running instance's `tcb_info.compose_hash` before trusting it.**
Nobody on this project has run that comparison against a live agent, so it is
the derivation that is unconfirmed, not the value.

### The server side has no configuration surface yet

`WitnessPin` and `verify_witness_certificate` exist and are tested, but nothing
in `trace-commons-ingest` builds a pin from configuration. **There is no
environment variable to set today.** Verification of a real certificate by the
running server arrives with the plan that lets a certificate affect the PII
backstop; until then, this deployment produces certificates that the server can
verify in principle and does not verify in practice.

---

## Upgrades — the order matters, and one case breaks it

### The ordinary case: a new image

dstack's KMS derives the app signing key from a **stable app id**, not from any
measurement register. The app id is the first 20 bytes of the *initial* compose
hash and is then persisted. So an image upgrade moves the compose hash, moves
MRCONFIGID, and **leaves the signing address exactly where it was.** Measurements
gate key *release*, not key *derivation*.

That is what makes an upgrade a re-allowlisting rather than a fleet-wide break,
and it is why the pin holds an address and a **set** of measurements rather than
one of each. The order follows directly:

1. Build the new image, push it, read its digest.
2. Update `docker-compose.yml` with the digest, run `./build-app-compose.sh`,
   commit both.
3. Deploy to **one** instance and read its measurement from `/v1/attestation`.
   Do not deploy the fleet yet.
4. **Add the new measurement to the pinned set — everywhere, and before you
   deploy further.** The set now admits both the old and the new.
5. Roll the rest of the fleet.
6. After every instance reports the new measurement, and no earlier, drop the
   old one from the set.

Do steps 4 and 5 in that order and no client is ever broken by an upgrade it
has not been told about. Do them the other way and every contributor who has
pinned correctly refuses the new deployment, which is the pin working as
designed and will look like an outage.

Steps 3 and 6 are the ones people skip. Step 3 exists because the measurement
is read from a running instance, not predicted — see Reproducibility. Step 6
exists because a pin set that only ever grows stops being a pin.

### The case that breaks it: changing the guest-API surface

**A guest-API surface migration is not an image upgrade and this rollout does
not cover it.**

dstack's `/v1` guest API derives **different key material** from the `v0`
surface, by design and with no compatibility mode. Moving from one to the other
**changes the signing address**. Every client that has pinned the address stops
verifying, and re-allowlisting a measurement does not help, because it is not
the measurement that moved.

This deployment therefore names the surface explicitly rather than using the
agent's unversioned alias — an alias is a thing that can be repointed, and
repointing it here would rotate a signing key. In
`crates/trace-commons-server/src/witness_service/enclave.rs`:

```rust
pub const GET_KEY_PATH: &str = "/v0/GetKey";
pub const GET_QUOTE_PATH: &str = "/v0/GetQuote";
```

Those constants are not a detail of the HTTP client. **They are part of the
signing identity.** Changing the `v0` in either one rotates the signing address
of every deployment that picks up the change.

If a surface migration ever becomes necessary, it is a **key rotation**, and it
needs a rotation plan — an overlap window in which both addresses are accepted,
or a coordinated cutover — not this section's steps.

---

## Choosing a redaction mode

`TRACE_COMMONS_WITNESS_REDACTION` is required, with no default in either
direction, so that nobody deploys either mode by leaving a variable unset.

### `deterministic-only`

The deterministic secret pass and nothing else. No network dependency at all,
which is why it remains available.

**It redacts less than ingest does.** The prose-PII classifier never runs, and
the certificate's `redaction_policy_version` carries the deterministic alias, so
a server that requires the classifier can and should refuse the certificate. A
`deterministic-only` witness is honest about being narrower — it is not a
witness whose verdict silently means less than it appears to.

The reference compose no longer ships this mode. It remains available, and it
is the right choice for a deployment unwilling to put any classifier operator
inside its trust boundary.

### `full-pipeline`

The deterministic pass, then the prose-PII classifier over its output — the
same two stages, in the same order, that ingest applies to every event it
receives. This is the mode the design is aiming at.

It requires a classifier backend, resolved from
`TRACE_PRIVACY_FILTER_BACKEND` **at startup**. A witness told to run
`full-pipeline` with no backend configured does not start, and a backend that
fails mid-request refuses rather than degrading to the deterministic result.
Both are correct: a certificate that quietly claimed coverage the pass did not
have is the failure this whole design exists to prevent.

The two ways to supply one, and the cost of each:

- **A sibling container inside this CVM** (`TRACE_PRIVACY_FILTER_BACKEND=self-hosted`,
  pointed at the compose network). The classifier is then covered by the
  compose hash and therefore by the measurement, and no text leaves the
  enclave. The cost is real: it is a multi-gigabyte model running on CVM vCPUs
  with no GPU. On this project's CPU-only pilot host, `openai/privacy-filter`
  measured around **58 characters per second**. Size the CVM against that
  number before choosing this, and note that a slow classifier on an
  unauthenticated route is also a cheaper denial-of-service target.
- **An external endpoint** (`near-ai`, or a `self-hosted` URL outside the CVM).
  Faster, and it **sends partially-redacted text out of the enclave.** The
  deterministic pass has run first, so credentials and local paths are masked —
  that ordering is deliberate and is why it is this way round — but prose PII
  is still present in what leaves. If you choose this, you have decided that
  the classifier operator is inside your trust boundary. Decide it explicitly.

**What this deployment ships.** `full-pipeline` against `near-ai`, pinned in
the compose to `https://cloud-api.near.ai/v1` and `openai/privacy-filter`. That
is a decision that partially-redacted text leaves the enclave and that NEAR AI
is inside the trust boundary of every transcript this witness sees. The
deterministic pass runs first, so credentials and local paths are masked in
what goes out; prose PII is not.

The API key is the one value in `allowed_envs` — injected encrypted at deploy
time rather than written into the measured compose, because this repository is
public and the manifest is committed. The destination and the model stay
measured, so an injected key can change which account is billed and cannot
change where a transcript goes. See `build-app-compose.sh` for the argument in
full, and make it again before adding a second name to that list.

Changing the mode changes the measurement. See "The configuration is measured".

---

## First boot

Expect exactly two things in the log, and nothing else about any request:

```
witness ready signing_address=0x... witness_measurement=mrtd:...+mrconfigid:... max_request_bytes=67108864
```

Boot is fail-closed by design: the agent round trip that derives the signing key
and reads the measurement happens **before** the listener binds. A witness that
cannot reach the dstack agent, cannot derive its key, or cannot read its own
measurement exits non-zero rather than accepting a request it will refuse. That
is the difference between an operator seeing the failure and a contributor
seeing it.

Failures you should expect to meet, and what they mean:

- **`could not derive a signing identity from the dstack guest agent`** — the
  socket is not reachable. Check that `/var/run/dstack.sock` is mounted, and
  check the container user can open it. The image runs as uid 10001; if your
  dstack version creates that socket root-only, either grant the socket's group
  to that user or drop the `USER witness` line in the `Dockerfile` and rebuild
  — noting that rebuilding changes the measurement. **This has not been tested
  against a live agent**, and it is the most likely thing to go wrong on a
  first deployment.
- **`MalformedResponse`** — the agent answered, and its JSON encoding is not
  what this client expects. The failure direction is right (refuse at boot
  rather than run on a misread key), but this is the other unverified thing:
  nothing in this project has spoken to a live dstack agent. Confirm on a real
  instance before a deployment carries traffic.
- **`TRACE_COMMONS_WITNESS_REDACTION must be ...`** — the variable is unset or
  misspelled. There is no default; see above.
- **Nothing arrives at the witness, but it looks healthy** — the bind address.
  The binary defaults to `127.0.0.1:8088`, which is correct for a host
  deployment and unreachable inside a container. The compose sets
  `0.0.0.0:8088`.

Smoke test from outside, using only the attestation route so the test does not
require a transcript:

```sh
nonce=$(openssl rand -hex 32)
curl -sS -o /dev/null -w '%{http_code}\n' \
  "https://<witness-host>/v1/attestation?nonce=${nonce}"
```

A malformed nonce is rejected rather than padded — `parse_hex` accepts exactly
64 bare hex characters, no `0x` prefix — so a `400` here is usually your nonce,
not the witness.

---

## What in this document is unverified

Stated plainly so nobody reads the rest as tested:

- **No part of this has run on a real CVM.** The image build was exercised on a
  developer machine and the binary was confirmed to start, and fail closed
  without a guest agent, in the runtime image; the deployment was not.
- **The image whose build was exercised is arm64.** The developer machine is
  Apple Silicon. TDX is Intel, so the image you deploy is an amd64 one, and the
  digest you pin must come from an amd64 build. The `Dockerfile` is
  architecture-neutral and nothing in it is conditional on one, but that is an
  argument, not a build log.
- **Nothing in this project has spoken to a live dstack guest agent.** The
  socket path, the `/v0` method names and the JSON encoding of the agent's
  responses are taken from dstack's guest-API documentation and are exercised
  only against a test double.
- **`app-compose.json`'s field set has not been validated by a dstack
  deployer.** The manifest is written from dstack's documented schema, not from
  a rejected-then-corrected upload. Confirm the keys against the version you
  deploy — in particular `public_tcbinfo` and `no_instance_id`, which are the
  two whose spelling this project has never seen an agent accept. An unknown
  key that a deployer silently drops changes nothing visible except the compose
  hash you pinned.
- **The compose-hash derivation is unconfirmed.** `build-app-compose.sh`
  computes SHA-256 over the manifest bytes. That the value equals a running
  instance's `tcb_info.compose_hash` is the assumption to check first on a real
  instance.
- **The container-user / socket-permission question is open** — see First boot.
- **The image has never been reproduced**, by anyone, on any second machine.
- **The `full-pipeline` sibling-container topology has not been built.** The
  performance number quoted for it is measured, but on the pilot host rather
  than inside a CVM.

---

## See also

- `docs/superpowers/specs/2026-09-02-redaction-witness-service-design.md` —
  the design and its threat model.
- `docs/superpowers/plans/2026-09-02-redaction-witness-service.md` — the plan,
  including what is deliberately not in it.
- `crates/trace-commons-server/src/witness_service/` — the service.
- `crates/trace-commons-server/src/redaction_witness/` — the certificate and
  the server-side verification.
- `docs/operator/pii-backstop.md` — the backstop this certificate does **not**
  replace.
