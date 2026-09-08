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
- **`public_logs` and `public_sysinfo` are off in `app-compose.json`, and that
  did not reach the deployment.** dstack will serve container logs publicly if
  asked, and the first live CVM was deployed with both set to `true` -- because
  `phala deploy` never read the manifest this repository generates. See
  "The manifest we write is not the manifest that deploys" below. Setting them
  here is necessary and is not sufficient; check the stored manifest after
  every deploy.
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

The witness serves exactly two routes, both unauthenticated, neither
rate-limited by source, and without TLS of their own:

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
  service, your money. What that spend is *bounded* by is below — a
  concurrency limit and a request deadline, both measured — and what it is
  still not bounded by is below too.
- `/v1/attestation` is **a quote oracle**. Anyone who can reach it obtains a
  fresh TDX quote over a report body of their choosing in the nonce half. The
  quote proves what it says, and nothing about a caller.

A deployment is expected to put something in front of it. `gateway_enabled` is
on, so dstack-gateway terminates TLS — that is the TLS answer and not the abuse
answer.

**Part of the abuse answer is now in this directory, and part of it still is
not.** Be precise about which is which.

*What is configured here.* `POST /v1/witness` is bounded in two ways, both set
in the measured compose and therefore part of the enclave's identity — a
contributor who pins the measurement can verify that a bound exists and what it
is:

| Variable | Ships as | What it bounds |
|---|---|---|
| `TRACE_COMMONS_WITNESS_MAX_CONCURRENT_REQUESTS` | `4` | How many witness requests run at once. Over it, the witness answers `503` with `witness_saturated` and a `Retry-After`, immediately. It does not queue. |
| `TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS` | `300` | How long one request may take. Over it, `504` with `witness_request_timed_out`, and the slot is released. |

Neither refusal certifies anything. Together they bound what an anonymous
caller can spend: cores, and — because `full-pipeline` sends prose to a metered
external classifier — money. A concurrency bound was chosen over a rate limit
deliberately: behind dstack-gateway the peer address is the gateway's, so a
per-source limit here would have to trust a forwarded header, and keying a
limiter on identity is the correlation this witness is built not to have. A
concurrency bound needs no identity.

`GET /v1/attestation` is **not** bounded with it, on purpose. It reads no body
and does one enclave round trip, and it is what a contributor uses to pin this
witness before trusting it; making it unavailable under load on the other route
would make the enclave unpinnable exactly when someone is attacking it.

*What is still the deploying operator's, and is not configured here.* A
**per-source rate limit at an edge** — the thing that stops a single caller
re-arriving four at a time forever, which a concurrency bound does not; a
body-size limit at the edge below the witness's own; and the **reachability
decision** (public, or only from your contributor shells' egress). The bounds
above make an open witness survivable, not unattended. If you deploy this on a
public hostname with no edge in front of it, you have deployed a bounded but
open redaction service, and an open quote oracle.

---

## Files in this directory

| File | What it is |
|---|---|
| `Dockerfile` | Builds the witness image. Not reproducible; see below. |
| `docker-compose.yml` | The application. **Measured** — every value in it is part of the enclave's identity. |
| `app-compose.json` | dstack's manifest. Generated; embeds the compose file verbatim. |
| `build-app-compose.sh` | Regenerates the manifest, and `--check` fails if it has drifted. |

`docker-compose.yml` is the source of truth and `app-compose.json` is derived.
**Run `./build-app-compose.sh` after every compose edit and commit both.**
`./build-app-compose.sh --check` answers "is the manifest I am about to upload
the one this compose file describes" without modifying anything; run it before
a deploy.

**It prints no hash, and that is deliberate.** It once printed the SHA-256 of
the manifest it writes; that value is not `compose_hash` and cannot be made
into it, for the reason the next section gives. See "Reading the measurement"
for where the pinnable value comes from.

### The manifest we write is not the manifest that deploys

This section replaces an earlier claim that "only the second one deploys". It
was wrong, and it was wrong in the direction that matters: it described the
generated manifest as the thing with authority, when in fact `phala deploy`
takes `docker-compose.yml` and **builds its own manifest**, never reading
`app-compose.json` at all.

Measured on 2026-09-04 against the live CVM. `phala cvms get <id> --json`
reports `compose_file`, which is the manifest dstack actually stored. Compared
against what this directory generates:

| Field | We wrote | dstack stored |
|---|---|---|
| `public_logs` | `false` | **`true`** |
| `public_sysinfo` | `false` | **`true`** |
| `allowed_envs` | `[TRACE_NEAR_AI_PRIVACY_API_KEY]` | **`+ DSTACK_AUTHORIZED_KEYS`** |
| `pre_launch_script` | `""` | **~17 KB of Phala boot script** |
| `kms_enabled`, `local_key_provider_enabled`, `gateway_enabled`, `public_tcbinfo`, `no_instance_id` | as written | as written |

Every field that constrains what the enclave exposes was overridden. The three
that survived are the ones whose values happened to match Phala's defaults, so
agreement here is coincidence rather than the manifest being honoured.

Two of the overrides are worth stating plainly, because they change the threat
model this README argues elsewhere:

- **`public_logs: true`** on a service whose entire premise is that raw
  transcripts do not leave it. Container logs are the most direct way for one
  to escape, which is why the generated manifest sets it `false`.
- **`DSTACK_AUTHORIZED_KEYS` in `allowed_envs`**, combined with a
  `pre_launch_script` that writes that value to `/home/root/.ssh/
  authorized_keys` and sets a root password. The generated manifest allows
  exactly one injectable name and argues for it at length in
  `build-app-compose.sh`; the deployed manifest allows a second one that grants
  shell access. Nothing has been injected -- but the enclave's identity now
  admits it, and an operator who reads only our manifest would not know.

**This also settled the open question in `build-app-compose.sh`.** That script
used to print the SHA-256 of the manifest it writes, warning that the value
might not equal the `compose_hash` dstack derives and noting nobody had run
the comparison. It has been run and they differ: local `a12e930e...` against
deployed `c2511a8b...`, which is the value inside the live certificate's
MRCONFIGID. Because the deployer never reads the file being hashed, no local
derivation can be authoritative — this is a property of the deploy path, not a
bug in the hashing. **The script therefore prints no hash at all**: a wrong
number carrying a caveat is still a number somebody copies into a pin, and a
pin on a measurement no client presents refuses every attested submission.
**Read `compose_hash` from the instance.**

So the deploy procedure needs a step it does not yet have: after deploying,
read back `compose_file` and diff it against the intended manifest, and pin
`compose_hash` from the instance. Until that exists, treat every setting in
`app-compose.json` as a statement of intent that has not been enforced.

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

**There is no way to derive the compose-hash half locally, and nothing in this
repository offers one.** `build-app-compose.sh` used to print the SHA-256 of
the manifest it writes; the comparison against a live agent has since been run
and the values differ, because `phala deploy` never reads that file — it hashes
a manifest of its own. The script no longer prints a hash for that reason.
`compose_hash` comes from `phala cvms get <cvm-id> --json`, or from
`tcb_info` on a running instance, and nowhere else.

### The server side has no configuration surface yet

`WitnessPin` and `verify_witness_certificate` exist and are tested, but nothing
in `trace-commons-ingest` builds a pin from configuration. **There is no
environment variable to set today.** Verification of a real certificate by the
running server arrives with the plan that lets a certificate affect the PII
backstop; until then, this deployment produces certificates that the server can
verify in principle and does not verify in practice.

---

## Which KMS, and why that is not a detail

`phala deploy` takes `--kms-id`, and on a node that supports on-chain KMS it
refuses without one. **This deployment uses `phala-usc1`**, on node `18`
(`prod9`, US-WEST-1).

The reasoning, so the next person inherits it rather than rediscovering it at
the same prompt:

- The signing address derives from the **KMS and the app id together**. Moving
  to a different KMS later therefore rotates the signing address — which is a
  **key rotation**, not an upgrade, and re-allowlisting a measurement does not
  help, because it is not the measurement that moved. It has the same shape as
  the `/v0` → `/v1` guest-API change described below, and the same cost: every
  client that pinned the old address stops verifying.
- So this is chosen once and is expensive to revisit. It is not a deployment
  detail even though it looks like one on the command line.
- `phala-usc1` is Phala-operated and regionally matched to `prod9`. The
  alternatives offered were other `phala-*` instances and the on-chain
  `kms-eth-*` / `kms-base-*` families, which put key-release policy under a
  contract on Ethereum or Base respectively.
- Neither on-chain family was chosen because **nothing in this project already
  depends on those chains.** Credit settlement is on NEAR, so an Ethereum or
  Base KMS would introduce a second chain into the trust path for no benefit
  this deployment can name. That is a reason to revisit if the surrounding
  architecture ever moves on-chain in a way that makes one of them the natural
  home for key-release policy.

What choosing a KMS does *not* change: `kms_enabled: true` and
`local_key_provider_enabled: false` are settled and recorded in
`build-app-compose.sh`. A local key provider seals to one host TPM and would
tie the signing address to a single machine, losing the property that makes the
upgrade path below work at all.

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

#### Step 3 has a trap: the old container keeps answering

Observed on the 2026-09-05 redeploys. After `phala deploy` upgrades the CVM,
the old container went on answering for roughly a minute, and it answered with
the **old** measurement — while `phala cvms get --json` already reported the
new `compose_hash`. An operator who reads a measurement immediately after the
deploy therefore reads the one they just replaced, and pinning it makes every
client refuse the deployment that is actually running. `running` in
`phala cvms get` describes the CVM, not the container, so it does not close the
window.

Treat that as an observation from two redeploys and not as a timing guarantee:
do not wait out a minute, poll. Ask the witness for a certificate until the
measurement it carries names the new `compose_hash`, and pin only then. The
poll loop, and why the certificate rather than `/v1/attestation` is what
carries a plaintext measurement, are in `docs/operator/attested-inference.md`,
"The stale-container trap".

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

## Requiring attested inference

`TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE` is **off by default**, and
the default is deliberate rather than lax. Turning it on refuses a great deal
of honest traffic, and an operator has to want that.

When it is on, the witness refuses to certify a contribution unless the **last
inference call the contribution declares** carries a NEAR AI receipt that
verifies against that call's own raw request and response bodies, as they sit
in the session the witness was handed. The witness — not the contributor —
decides which exchange that is: it takes the last `HttpExchange` event in the
trace's own order.

### What the requirement establishes — and read this before writing any surface

At witness time, an attested NEAR AI enclave had produced that response for
that request, and both bodies were inside the session that was certified.

**The attested bytes are not the bytes the agent sent.** The receipt binds what
the *upstream provider* received and returned. On an IronWire route the request
is rewritten before forwarding — a policy model swap re-serialises it, the
privacy filter re-serialises it wholesale, and a cross-family route synthesises
a different document entirely, so the attested request on a NEAR AI route may
be a Chat Completions document built from an Anthropic one, carrying substituted
models and privacy-filter placeholders where the original held real values. The
attested response is the provider's own raw event stream, not the frames the
client saw.

So the claim is *these are the bytes the provider hashed*. It is never "this is
the request the agent made", and no wording may let a reader assume it is.

One consequence runs in our favour: capture sits downstream of the privacy
filter, so the attested bytes are already filtered.

It also does **not** establish that the session made the call — a contributor
holding a receipt and its bodies can paste them into a trace they wrote. It says
nothing about any other turn, tool result or file edit. And it says nothing
about the conversation history: the reason one body pair is worth having is that
a chat-completions request body repeats the whole conversation prefix, but a
session that compacted or truncated its context sends a summary instead, and the
witness cannot tell which it got.

Render the count `n_of_m` — one verified receipt over a trace declaring three
calls is `1_of_3`. Never "attested", never "genuine".

### The model, and which form of receipt binds it

`verify_receipt` supports both a two-part receipt — `<requestHash>:<responseHash>`,
binding no model — and a three-part one whose leading part is the model name and
is covered by the signature.

A hosted NEAR AI model returns the **three-part** form: the receipt text is
`{model}:{requestHash}:{responseHash}`, so the receipt names its own model.
That is the model this witness looks a pin up by. The `model` query parameter
on retrieval (`GET /v1/signature/{chat_id}?model=...`) is *not* signed and is
chosen by whoever fetches the receipt, so it establishes nothing and is not
read here.

An earlier revision of this document asserted the provider signs only the
two-part form and concluded that no model policy was possible. That was wrong
about the provider, and the conclusion drawn from it — pinning one
provider-wide key — could not work, because the signing key is per model. The
two-part form is still accepted where nothing is pinned; under pins it is
refused, because there is no model to look a pin up by.

### Pinning the receipt signing keys

> **Breaking change for anyone running the previous release.**
> `TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN` is **gone and is no longer read**.
> It pinned the gateway's ed25519 key, and the gateway key signs **no
> receipt** — so a witness with it set refused every real receipt, under the
> same folded `witness_inference_receipt_unverified` label as a forgery.
> **Unset that variable** and, if you want pinning, set
> `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` and/or
> `TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS` instead. A witness with the
> old variable set **refuses to start** and names the replacements.
>
> The gateway key is pinnable again, under a **different name**, and the
> refusal on the old one stays. `TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN`
> applied its key to *every* receipt, including the `provider_tee` receipts it
> could never match; silently re-reading a value you set under those semantics
> would give it a meaning you never chose, on the control that decides who may
> sign. Set the new name deliberately.

`TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` is **unset by default**, and unset is
exactly the behaviour that shipped before any of this existed.

Verifying a receipt establishes that a well-formed signature over these bytes
checks out against the key **the receipt itself names**. Any key satisfies
that, including one the submitter holds. What makes a receipt mean *an attested
NEAR AI enclave signed this* is comparing that key against one NEAR AI's
attestation report binds into a TDX quote — `report_data` is `signing_address
|| nonce`, so a key read from a report issued against a nonce you chose was
attested for you, now.

The contributor client already makes that comparison, against a report it
fetches itself. A check the submitter runs on its own submission is not a
bound: a patched client does not run it. This variable is the same comparison
made at the point the decision is enforced.

**Which key signs depends on the protocol, not on the model.** This is the
part that has bitten this deployment twice, so it is stated as a table. NEAR
AI issues two legitimate kinds of receipt for the **same hosted model**:

| your contributors' inference call | receipt `signature_kind` | signer | pinned by |
|---|---|---|---|
| Chat Completions, `POST /v1/chat/completions` | `provider_tee` | the per-model key in `model_attestations` | `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` |
| Responses API, `POST /v1/responses` | `gateway` | the single key in `gateway_attestation` | `TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS` |

Both were captured live against `Qwen/Qwen3.6-35B-A3B-FP8`: the same model,
the same day, two different signers.

**If your contributors use the Codex CLI you need the gateway pin.** Codex
speaks the Responses API exclusively — it dropped `wire_api = "chat"` — so
every Codex-driven receipt is gateway-signed. A witness pinning only model
keys refuses all of that traffic, under the same folded
`witness_inference_receipt_unverified` label as a forgery. Chat-completions
traffic needs the model pins. A deployment seeing both needs both variables
set.

For a `provider_tee` receipt the pin is a map, and the model it is looked up
by is the one in the **receipt's own signed text**
(`{model}:{requestHash}:{responseHash}`), not one supplied beside it. A
`gateway` receipt binds no model — the gateway signs for whatever was served —
so its pin is a bare key list with nothing to look up.

#### A gateway receipt attests the bytes, not the model

This is a **threat-model consequence**, not a formatting detail, and enabling
the gateway pin is the first configuration in which it arises. Read it before
you set the variable.

A gateway receipt's signed text is the two-part `{requestHash}:{responseHash}`.
There is no model in it, so `verify_receipt` returns no bound model and the
declared model is compared against nothing — not by the client, not by the
witness. The gateway key is a **single key shared across every hosted model**,
so it cannot distinguish them either: the same key signs whatever the Responses
API served.

The consequence, stated plainly: a contributor holding a genuine gateway
receipt for a cheap model can declare that exchange as **any other model**, and
both the client and a gateway-pinned witness will accept it. The receipt is
real, the signature verifies, the bytes are exactly the bytes — and the model
label beside them is an unattested claim.

Under model pins alone this could not happen, because a receipt binding no
model was always refused. Turning on the gateway pin is what admits an attested
receipt that carries no model claim at all.

This is inherent to NEAR AI's receipt format and **cannot be fixed here**. What
follows from it:

- A gateway-pinned witness attests the request and response **bytes**. It does
  **not** attest the model.
- Any downstream consumer that keys credit, scoring, pricing or eligibility off
  the **declared** model must not treat a gateway receipt as evidence of that
  model. The receipt is evidence about bytes only.
- A deployment that needs the model attested must require the chat-completions
  path, whose three-part receipt signs the model name — which means not
  accepting Codex traffic, since Codex speaks only the Responses API. That is a
  product trade-off, and this witness cannot make it for you.

The receipt's own `signature_kind` selects **one** set and never both. A
gateway receipt is not tried against a model key, nor the reverse: accepting
either would mean a key attested for one role could vouch for the other. A
`signature_kind` the witness does not recognise, or a receipt that carried
none, names no key set and is refused.

**On a witness that pins either kind, the kind you did not pin is refused.**
That is fail-closed and deliberate: pinning your model keys is not agreement
to accept any gateway-signed receipt from any signer. A witness that pins
nothing at all remains fully dormant.

Set `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS` to `model=key[,model=key...]`, each
key **64 hex characters, no `0x`**. Repeat a model to pin more than one key
for it, which is how a model served by several enclaves is pinned. Derive the
keys once, out of band:

```bash
NONCE=$(openssl rand -hex 32)
MODEL='Qwen/Qwen3.6-35B-A3B-FP8'
curl -s --get https://cloud-api.near.ai/v1/attestation/report \
  --data-urlencode "model=$MODEL" \
  --data-urlencode "signing_algo=ed25519" \
  --data-urlencode "nonce=$NONCE" \
| jq -r --arg n "$NONCE" --arg m "$MODEL" '
    .model_attestations[]
    | select(.model_name == $m)
    | select(.signing_algo == "ed25519")
    | (.intel_quote | ascii_downcase) as $q
    # The quote header must be v4 TDX before any offset in it means what we
    # think: bytes 0..2 are the version (LE 4 -> "0400") and bytes 4..8 the
    # TEE type (LE 0x81 -> "81000000"). Hex chars [0:4] and [8:16].
    | select($q[0:4] == "0400" and $q[8:16] == "81000000")
    # report_data lives INSIDE the quote, at byte offset 568, length 64.
    # In hex characters that is [1136 : 1264].
    | select($q[1136:1264] == (.signing_address + $n))
    | .signing_address'
```

`signing_algo=ed25519` is **not optional and not cosmetic**: it is a query
parameter that selects which attestations come back, and the default is ECDSA
— whose signer appears in no ed25519 attestation and signs nothing this witness
verifies. A report fetched without it looks entirely well formed and attests
the wrong thing. That omission is what produced the gateway-key mistake above.

**A `model_attestations` entry carries no `report_data` field.** Only
`gateway_attestation` has one, as a convenience echo of what its own quote
already says. On a model attestation the binding is inside `intel_quote`, at
the TDX quote's standard `report_data` position: byte offset 568, 64 bytes —
`signing_address` then `request_nonce`, exactly. That is what the slice above
reads, and it is what the code checks (`attested_ed25519_key` prefers the
quote and requires the JSON echo to agree with it wherever both exist). An
earlier draft of this section told you to filter on a `.report_data` field;
that filter matches nothing on a model attestation and would have yielded an
empty pin list.

The `select(...)` line is the binding check, and it is the reason for the
nonce: without it you are pinning a key some report once listed, rather than
one attested against a value you chose.

#### Upgrade order: three steps, and the order is load-bearing

Do this before deriving anything. Getting it wrong refuses live submissions
under the folded label, which is hard to diagnose from outside.

1. **Deploy the new witness with BOTH pin variables unset.** Unset means
   dormant: an offered receipt still has to verify against the key it names,
   and nothing is refused for its kind. This step upgrades the code that can
   *read* `signature_kind` without yet enforcing anything with it.
2. **Upgrade the contributor clients.** Only a client at this version sends
   `signature_kind` on the wire. This step must follow step 1 because the
   witness request body is `deny_unknown_fields`: an older witness refuses a
   body carrying the new field outright.
3. **Then set the pins.** Only now is it safe to enforce, because a witness
   that pins **either** kind refuses a receipt whose kind it cannot place —
   and an un-upgraded client sends no `signature_kind` at all, which reads as
   `Unrecognised`, resolves to an empty key set, and is refused. Setting the
   pins before every client is upgraded refuses those clients' submissions.

"Witness first, then clients" alone is **not** sufficient: it is necessary for
step 2, and step 3 is a separate gate on the same rollout.

Steps 1 and 3 are separate deployments of the witness. In the committed compose
the pins live inside the measurement, so step 3 moves the measurement and needs
re-allowlisting everywhere it is pinned — plan it as its own change.

#### Deriving the gateway pin

The **same fetch** answers both — one report carries `gateway_attestation` and
`model_attestations` — so this is a second `jq` over the response you already
have, not a second request. Set the result as
`TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS`: bare keys, `key[,key...]`,
64 hex characters each, **no `0x` and no `model=` prefix** (that spelling
belongs in the model variable and is refused here).

```bash
NONCE=$(openssl rand -hex 32)
MODEL='Qwen/Qwen3.6-35B-A3B-FP8'
curl -s --get https://cloud-api.near.ai/v1/attestation/report \
  --data-urlencode "model=$MODEL" \
  --data-urlencode "signing_algo=ed25519" \
  --data-urlencode "nonce=$NONCE" \
| jq -r --arg n "$NONCE" '
    .gateway_attestation
    | select(.signing_algo == "ed25519")
    | (.intel_quote | ascii_downcase) as $q
    # Same v4-TDX header check and same report_data offset as above.
    | select($q[0:4] == "0400" and $q[8:16] == "81000000")
    | select($q[1136:1264] == (.signing_address + $n))
    | .signing_address'
```

Two differences from the model derivation, both immaterial to the check.
`gateway_attestation` also carries a `report_data` *field*, a convenience echo
of what its own quote says; the witness reads the quote and requires the echo
to agree with it, so the `jq` above reads the quote too. And `model` is still
sent on the query because the endpoint requires it — the key that comes back
is the gateway's, the same one whichever model you name.

To confirm a receipt you already hold is the kind you think, read its own
`signature_kind`:

```bash
# A Responses-API exchange: the FULL `resp_`-prefixed id. Stripping the
# prefix 404s.
curl -s --get "https://cloud-api.near.ai/v1/signature/resp_<id>" \
  --data-urlencode "model=$MODEL" \
  --data-urlencode "signing_algo=ed25519" | jq '.signature_kind, .signing_address'
```

Two things this derivation does **not** do, and should not be read as doing:
it does not verify the quote's signature against Intel collateral, and it does
not check the quote's measurements. You are trusting the report you read, once,
at pin time.

The witness does not fetch the report on the request path — it makes no
outbound calls while serving, and a report fetched at request time would be
trusted over a path an attacker able to substitute a signing key is also
positioned to influence.

Four properties worth stating plainly:

- **ed25519 only.** The ECDSA signer NEAR AI also issues appears in **no**
  ed25519 attestation, so it cannot be pinned and an ECDSA receipt can never
  satisfy a pin, however well it verifies. Pinning is therefore also a
  decision to require the ed25519 form.
- **A `provider_tee` receipt binding no model cannot satisfy a pin.** The
  two-part `<requestHash>:<responseHash>` form commits to no model, so there
  is no model pin to place it against and a pinning witness refuses it.
  Choosing a pin for it from the request body would be checking the receipt
  against something its signature never covered. A `gateway` receipt is a
  different case: it is *expected* to bind no model, and its pin is keyed by
  nothing, so the two-part form is its normal shape.
- **The kind you did not pin is refused.** On a witness that pins either kind,
  a receipt of the other kind is checked against an empty set and refused, and
  so is one whose `signature_kind` this witness does not recognise. A witness
  that pins nothing is dormant for both.
- **Independent of the requirement.** The pins constrain *which key* is
  trusted; `TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE` decides *whether
  a receipt is needed*. A witness that requires nothing still refuses a
  receipt from an unpinned key when one is offered — certifying it would be
  the silent downgrade that accepting an invalid receipt already is not.
- **Malformed is a startup failure.** A value that is not `model=key` pairs of
  32-byte hex keys — the empty string included — refuses to start rather than
  becoming a pin that matches nothing, and so does a
  `TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS` that is not bare 32-byte
  hex keys. Neither value is echoed into a log; the process logs a
  `model_key_pins_sha256_prefix` with a `pinned_models` count and a
  `gateway_key_pins_sha256_prefix` with a `pinned_gateway_keys` count at
  startup, so an operator can confirm which sets it holds without putting a
  key or a model name on an operational surface.

Every pin failure — unpinned model, unpinned key, an unpinned or unrecognised
`signature_kind`, a `provider_tee` receipt binding no model,
an ECDSA receipt — is reported as `witness_inference_receipt_unverified`, the
same label as every other receipt failure, and that is deliberate. A label of
its own would make this route an oracle: a prober could learn from a refusal
alone whether its receipt was signed by a key you trust, and which models you
pin. The cost is that a **missing** pin for a model you actually serve looks
exactly like a forgery from outside, which is why the committed compose ships
with no pins rather than a guess at your model list.

What it still does not establish: quote verification of the report is not part
of this path. You pinned keys you decided to trust after reading a report, and
the strength of the pin is the strength of that one-time procedure.

Like `TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE`, this variable is not
set in the committed compose, so the deployment described here does not pin.
Setting it in `docker-compose.yml` puts the pinned keys inside the measurement
— which is the right place for them, and means rotating a model key, or adding
a model, moves the measurement and needs re-allowlisting everywhere it is
pinned.

### The bodies do not leave the enclave

The witness **removes** the inference request and response bodies — and the
header maps beside them — from the artifact it returns. The certificate is over
the stripped bytes: the order is redact, strip, hash, sign, so the digest names
the artifact the contributor actually holds.

They are removed rather than kept because by that point they are worthless. The
witness redacts the session it is given, bodies included, so what survives the
pass no longer hashes to what the receipt binds — a downstream party trying to
re-verify gets a request-hash mismatch it cannot distinguish from tampering.
The only way to keep them verifiable would be to exempt them from redaction,
which means shipping raw prompts and completions to ingest and storage. That is
strictly worse than useless bodies. The bodies were only ever input to a check
that happens once, inside the enclave, over bytes only the enclave holds.

Two consequences:

- **The 16 MB envelope cap is not at risk.** The bodies never reach ingest or
  storage, so the payload that would have pushed an attested trace past the cap
  does not exist downstream. That was a real blocker and it is gone.
- **Method, URL and status survive.** Those are ordinary trace content and a
  consumer may want them. Headers do not survive: an inference request carries
  its credential in `Authorization`, and this repository has measured that
  opaque bearer tokens are not reliably redacted.

Note that the deterministic pass already dropped bodies and headers for events
whose *tool name* contains `http`, `browser` or `web`. The strip is what makes
that hold for an exchange captured under any other name — the guarantee is now
the witness's, not a classifier profile's keyed on a string the capture chose.

### Verification happens once and cannot be repeated

The receipt binds the raw bodies; the witness publishes a redacted artifact.
Redaction destroys the attested bytes, so **nothing downstream can re-check the
receipt**. The witness is the only party that ever holds both, which is why it
is the only party that can verify at all. Do not build a surface that implies a
server or a consumer re-verified anything.

Note also what the certificate does **not** carry: there is no
attested-inference field on it. Adding one needs a v2 profile with its own
signing domain (see `redaction_witness/certificate.rs`, "Why there are no
inference fields") and a flag day across three independent implementations of
the wire format.

With the bodies stripped, that limitation no longer touches the artifact: a
certificate exists **if and only if** attestation passed, because a requiring
witness issues none otherwise, and the artifact carries nothing a reader could
mistake for re-verifiable evidence.

**The open item, unchanged and not fixed by stripping:** a server still cannot
distinguish a requiring witness from a permissive one at the same measurement.
The measurement pins the image, not the environment. So the measurement plus
this deployment's configuration is the entire basis of the claim, and an
operator who cannot vouch for the configuration cannot vouch for the claim.

### What it refuses, and why each refusal is honest

Every refusal below is a 403 with a label, and a refused submission carries no
certificate at all.

| Label | Meaning |
| --- | --- |
| `witness_inference_attestation_missing` | required, and no receipt was offered |
| `witness_inference_attestation_unavailable` | offered on `POST /v1/witness`'s **text** shape, which carries no event order and so cannot say which call was last |
| `witness_inference_call_absent` | the contribution declares no inference call at all |
| `witness_inference_call_unattestable` | the final call declares a restarted stream, for which no receipt exists or ever will |
| `witness_inference_body_not_in_session` | the last call carries no bodies — in practice, the contribution withheld tool payloads |
| `witness_inference_receipt_unverified` | the receipt did not verify against those bytes, **or** — where either pin variable is set — its `signature_kind` is one this witness does not pin or does not recognise, a `provider_tee` receipt binds an unpinned model or no model at all, or the signer is not one of the keys pinned for the set its kind selects. One label for all of them, deliberately: see the anti-oracle note above |
| `witness_inference_body_too_large` | a body exceeds `TRACE_COMMONS_WITNESS_MAX_INFERENCE_BODY_BYTES` |

`witness_inference_receipt_unverified` is the one to read carefully. SHA-256
answers one bit, so a capture that pretty-printed a body, reordered its keys or
re-serialised it from a parsed form produces **exactly** the same failure as a
receipt lifted from somewhere else. The witness cannot tell them apart and does
not pretend to. On an honest deployment, suspect the capture first. Where a
gateway key pin is configured, a receipt from an unpinned key folds into this
same label too — so if you have just set the pin, suspect the pin first.

### The restarted-stream hole

IronWire's resilience guard restarts a stalled stream, and a restarted stream
records no digest — so no receipt exists for it. Because the requirement attests
the **final** call, a trace whose last call was restarted mid-stream can never
satisfy it. That is a coverage hole, not a rare edge.

It has its own label so an operator is not sent looking for a client bug, but
the witness can only use that label when the capture side *declares* the
restart, at `structured_payload["response"]["stream_restarted"]`. **Nothing
writes that field today**; it is a contract IronWire must honour. Until it does,
a restarted final call surfaces as `witness_inference_attestation_missing` —
fail-closed, but less informative.

### Who this excludes, before you turn it on

- **Every trace from a provider that is not NEAR AI.** Claude Code, Codex,
  Gemini and Cline sessions have no receipt to offer.
- **Every trace that withheld tool payloads.** The bodies live in an
  `HttpExchange` event's `structured_payload["request"]["body"]` and `content`,
  and both are written only under `include_tool_payloads`. That flag has
  historically been off everywhere.
- **Every trace whose final call was a restarted stream**, per above.
- **Every trace on the text shape of `POST /v1/witness`**, including the smoke
  tests in this document. The requirement is enforceable only on the structured
  `raw_contribution` shape.
- **Everything, on today's binary.** `trace-commons-witness` never calls
  `with_contribution_redactor`, so the structured route is unavailable and
  answers `witness_contribution_path_unavailable`. Turning the requirement on
  against this binary therefore refuses every submission by one label or the
  other. Wiring the structured seam into the binary is a prerequisite, and it is
  not part of this change.

Nothing fetches a receipt today, either: `upstream_id` in IronWire's ledger *is*
the receipt `chat_id`, and the ledger already persists it with both digests and
the served model, but no code anywhere calls `GET /v1/signature/{chat_id}`. That
call belongs downstream of the ledger and does not exist yet.

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

## The deployment that exists

First deployed 2026-09-04. Recorded here because an app id is what a signing
address derives from, and a CVM that nobody can name is one nobody can audit.

| | |
|---|---|
| CVM ID | `fa62907e-209f-45cd-8b70-86e450a62399` |
| App ID | `39cdd01fcb7bba691f07ee6951de147f7814f829` |
| Node | `18` (`prod9`, US-WEST-1) |
| KMS | `phala-usc1` |
| dstack image | `dstack-0.5.9` |
| Sizing | 4 vCPU, 8 GB |
| Container image | `ghcr.io/tracecommons/trace-commons-witness@sha256:f1d4c00266656f0227292efe7239595d6ad0bd7b9083c750d610c0e11b2689bc` |
| Manifest hash | `ee2a8af364272fae211fb5b7443d3c28de6153b300c781d75d06aaa9db547e2e` |

The sizing is not a default. `phala deploy` defaults to 1 vCPU and 2048 MB, and
the compose admits four concurrent witness requests at a 64 MiB body cap — on
the order of a gigabyte of buffers at full occupancy before the runtime. On a
public unauthenticated route an OOM kill is the denial of service the
concurrency bound exists to prevent, so the memory is sized above the worst
case rather than onto it.

**Still to be read from the running instance, and not yet recorded here:** the
signing address, the measurement, and the gateway hostname. Until those exist
no client can be pinned, and until `tcb_info.compose_hash` has been compared
against the manifest hash above, none of them should be.

---

## What in this document is unverified

Stated plainly so nobody reads the rest as tested. **Revised 2026-09-06**: the
first four items below said this had never run on a real CVM. It has — see
"The production deployment, as of 2026-09-06" above, whose every value was read
back from the running instance. Those items are struck and replaced rather than
deleted, because what they were wrong about is worth knowing.

- ~~**No part of this has run on a real CVM.**~~ **Superseded.** It runs on
  one. The image was built for amd64, deployed to CVM
  `8b8e6543-9743-41fc-ac05-a6b414888d5e`, and upgraded in place — the signing
  address survived, which is the proof it was an upgrade and not a recreation.
- ~~**Nothing in this project has spoken to a live dstack guest agent.**~~
  **Superseded.** The `/v0` guest-agent method names and JSON encoding are
  correct against dstack 0.5.9, exercised live rather than against the test
  double alone.
- ~~**`app-compose.json`'s field set has not been validated by a dstack
  deployer.**~~ **Superseded, and by a worse answer than validation:**
  `phala deploy` **never reads `app-compose.json`**. It builds its own
  manifest, forces `public_logs` and `public_sysinfo` to true unless told
  otherwise, and adds `DSTACK_AUTHORIZED_KEYS`. So the file's field set is not
  what you deployed, and the only honest reading is the manifest fetched back
  from the instance. This is why the deployment section tells you to pass
  `--no-public-logs --no-public-sysinfo` and re-read afterwards.
- ~~**The compose-hash derivation is unconfirmed.**~~ **Confirmed, it
  disagreed, and it has been removed.** `build-app-compose.sh` printed
  `bcbd152e` for the deployment whose instance `compose_hash` is
  `e848cac038d7a3181b0a9dbbd7ba63fbec3a2bed6aa6e58ad9309992eb9756eb`. That is
  the previous item's consequence, not a hashing bug: the script hashed a
  manifest the deployer never used, so no local derivation could be right. The
  script now prints no hash. **`compose_hash` comes from the instance.**
  MRCONFIGID = `01` + the instance value + zero padding is separately
  confirmed.
- ~~**The `full-pipeline` sibling-container topology has not been built.**~~
  **Superseded.** Production runs `full-pipeline` with the NEAR AI classifier.
  The performance number quoted for it is still a pilot-host measurement, not
  an in-CVM one.

Still unverified:

- **The container-user / socket-permission question is open** — see First boot.
- **The image has never been reproduced**, by anyone, on any second machine.
- **The attested-inference path has not completed end to end.** The witness is
  deployed and its measurement is pinnable, but a hosted exchange yielding a
  verified receipt and a body-free stored envelope has not been observed. See
  `docs/superpowers/reports/2026-09-06-attested-inference-first-run.md`. The
  receipt-key pins are at step 1 of their three-step rollout: deployed with
  both pin variables unset, which is dormant.

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

## The production deployment, as of 2026-09-06

Read back from the running instance, not from what we asked for. Every value
below came from `phala cvms get --json` or from a live certificate.

Because every redeploy moves the measurement, the measurement blocks published
in release notes go stale as soon as one happens — this file is the living
source of the current measurement, and a release note is only a snapshot of
what was current when it was written.

| | |
|---|---|
| CVM | `8b8e6543-9743-41fc-ac05-a6b414888d5e` (upgraded, not recreated) |
| App | `f1654b0beac2ac2afae4235ee3d907096cd8f3de` |
| Image | `ghcr.io/tracecommons/trace-commons-witness@sha256:75af1945502ebcd88b597993695549553fb011d0ff578aff357657f7e78e2d5c` |
| Mode | `full-pipeline` (NEAR AI classifier) |
| Signing address | `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798` |
| Instance `compose_hash` | `612d996cd7197803ca6deefbddb50ba3d09d32ec91174065e5f5df035fc1ebf8` |
| Measurement | `mrtd:f06dfda6dce1cf904d4e2bab1dc370634cf95cefa2ceb2de2eee127c9382698090d7a4a13e14c536ec6c9c3c8fa87077+mrconfigid:01612d996cd7197803ca6deefbddb50ba3d09d32ec91174065e5f5df035fc1ebf8000000000000000000000000000000` |
| Policy version | `ironclaw-deterministic-secret-path-v3+privacy-filter-near-ai-v1` |
| `public_logs` / `public_sysinfo` / `public_tcbinfo` | `false` / `false` / `true` |
| `allowed_envs` | `["TRACE_NEAR_AI_PRIVACY_API_KEY"]` |

**The signing address survived the upgrade**, which is the proof this was an
upgrade rather than a recreation: the KMS re-derived the same key from the
stable app id. An address that had changed would have meant a new app id and
would have invalidated every pin.

**MRCONFIGID is `01` + the instance `compose_hash` + zero padding, confirmed.**
`0168ecca83...` against a `compose_hash` of `68ecca83...`. So the two answers
agree, and either can be used to check the other -- but both come from the
instance. Nothing in this repository derives that value; `build-app-compose.sh`
once did, printed `bcbd152e` for this same deployment, and no longer prints a
hash at all.

**The visibility flags are set by `phala deploy` arguments, not by the
manifest.** `--public-logs` and `--public-sysinfo` default to **true**, and
`phala deploy` never reads `app-compose.json`. An earlier deployment served
container logs publicly for exactly this reason -- the setting was written
where nothing reads it. Pass `--no-public-logs --no-public-sysinfo` on every
deploy, and read the manifest back afterwards.

Note that toggling either flag changes `compose_hash`, and therefore the
measurement. Re-read the measurement after any such change before publishing
it.
