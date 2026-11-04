# Deploying the Insights judgement service on dstack

Design: [`docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`](../../docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md)
(spec PR #1501). The service follows the redaction witness's deployment
([`deploy/witness/README.md`](../witness/README.md)); read that first. This
file records what differs and, once there is a deployment, its values.

The operator runbook (`docs/operator/insights-judgement-service.md`) is
slice 8. Until it lands, this file is the only deployment note.

## Recorded deployment values

| Profile | App id | CVM id | KMS | Image digest | Measurement (`mrtd`, `mrconfigid`) |
|---|---|---|---|---|---|
| production | not deployed (slice 9) | | | | |
| development | not deployed (slice 6) | | | | |

Read every value from the running instance, never from a local manifest:
`phala deploy` ignores `app-compose.json` and builds its own (see
`deploy/witness/build-app-compose.sh`). **Record nothing here unless the
read-back in "Deploy" step 3 passed, and the poll in step 5 has seen the new
`compose_hash` in a verified quote.**

The sealing key and the signing address are generated in memory at every
boot, so they are not recorded and nothing pins them. The quote binds them;
the measurement is what a daemon pins.

## Two profiles, two images, two apps

| Profile | Image | Cargo features | Models it can call |
|---|---|---|---|
| `production` | `ghcr.io/tracecommons/trace-commons-judgement` | `near-attestation-collateral` | The production allowlist. Empty until slice 9. |
| `development` | `ghcr.io/tracecommons/trace-commons-judgement-development` | `judgement-development-models` | The mock now; Jev in slice 6. |

- A development model is not in the production binary, so no setting can
  turn one on there. `scripts/ci/check-release-judgement-features.sh` (run
  in `ci.yml`) refuses a change that would put `judgement-development-models`
  in the production arm of the Dockerfile, in the server crate's `default`,
  in an `--all-features` image build, or in any release workflow.
- Each profile is its own dstack app. The limiter key derives from the app
  id, so a development deployment never derives the production limiter key,
  even from an identical image.
- The production image cannot boot today. Owner decision 1: production
  waits for the TEE-hosted decision model, and the allowlist stays empty
  until it passes spec section 5. A NEAR AI model also refuses to boot with
  `missing_control:judgement_near_gateway_pin` until the gateway check
  (spec section 5, check 5) is built; see "Boot order".

## Build

Run the `judgement-image` workflow (`.github/workflows/judgement-image.yml`)
by hand, with `profile` set. It builds amd64 on a public runner, stamps
`SOURCE_DATE_EPOCH` from the commit, runs the pre-push checks below, pushes,
and prints the digest to pin. It never runs on a pull request. The build is
not reproducible, so a measurement pins one artifact.

Every release uses a new image tag (the commit): Phala may not re-pull an
unchanged tag. Pin the digest, never the tag.

## Pre-push checks

Each check runs the image with a read-only root, no network and no dstack
socket, and requires the exact refusal label, not just a non-zero exit. The
binary prints `judgement_boot_refused: <label>` and exits 1.

| Check | Profile | Expected |
|---|---|---|
| `--version` resolves its libraries and carries the commit date | both | prints a version line |
| no model configured | both | `judgement_model_unconfigured` |
| `typesafe/jev-1.13` configured | both, until slice 6 | `judgement_model_not_allowlisted` |
| the mock configured | production | `judgement_model_not_allowlisted` |
| no limiter volume mounted on `/data` | development | `judgement_limiter_volume_absent` |
| a volume, no dstack guest agent | development | `judgement_enclave_unavailable` |

The volume and guest-agent checks run on the development image only. A
production image refuses at the model check whatever it is told, so it
cannot reach them until slice 9 gives it a model.

## Deploy

This section carries over the witness's `phala deploy` lessons
(`deploy/witness/README.md`, the `compose_file` read-back and the visibility
flags). `phala deploy` never reads `app-compose.json`. It builds its own
manifest, and by default it turns `public_logs` and `public_sysinfo` on and
adds `DSTACK_AUTHORIZED_KEYS` to `allowed_envs`, with a pre-launch script
that installs root SSH keys. A pinned measurement with those settings would
publish the service's logs and admit a root shell that can read digests and
claims in memory and call `GetKey` for the limiter root.

1. **Build** with `judgement-image.yml` (`profile`). Pin the image digest in
   `deploy/judgement/docker-compose.yml` (or `.development.yml`).
2. **Deploy** with both visibility flags, every time:

   ```
   phala deploy --kms-id <id> --no-public-logs --no-public-sysinfo
   ```

   passing the compose file and the one secret the way the witness deploy
   does (`deploy/witness/README.md`). **Upgrade** in place with
   `phala cvms upgrade`, passing the same flags where it accepts them and the
   full secret set (an upgrade replaces every secret). Step 3 is the check
   either way: a flag that was not honoured shows up there. Never delete and
   recreate: that loses the limiter volume (see "The limiter volume").
3. **Read the manifest back. Mandatory.** Run `phala cvms get <id> --json`
   and take the `compose_file` field. Require all of these:
   - `public_logs: false` and `public_sysinfo: false`;
   - `allowed_envs` exactly `["TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY"]`
     (production) or `[]` (development, whose models need no key);
   - no `DSTACK_AUTHORIZED_KEYS` anywhere;
   - a `pre_launch_script` that is empty, or that writes no `authorized_keys`
     and sets no password.

   A deploy that fails any of these is torn down. Its measurement is never
   recorded or pinned. Toggling a flag changes `compose_hash`, so read again
   after any change.
4. **Commit the read-back** `compose_file` verbatim as
   `deploy/judgement/compose-readback.<profile>.json`.
   `scripts/ci/check-judgement-compose-readback.sh` (run in `ci.yml`, with a
   self-test) refuses a committed read-back that fails step 3's rules.
5. **Poll before recording.** Read `compose_hash` from the same output. Then
   ask `GET /v1/attestation` with fresh nonces until a verified quote's
   `mrconfigid` carries that `compose_hash` (the `01 || compose_hash` form the
   witness README records). Only then record the app id and the measurement
   above. This is the witness's stale-container trap: for about a minute
   after an upgrade the old container answers, with the old measurement,
   while `phala cvms get` already reports the new `compose_hash`. Do not wait
   out a minute; poll.
6. The drill (slice 8) repeats step 3 from the live quote: it hashes the
   committed read-back for the pinned measurement and requires that hash to
   equal the `compose_hash` in the quote's `mrconfigid`.

A new measurement pauses judgements for release daemons until a daemon
release compiles it in (owner decision 2). Ship that release, then drop the
old measurement in a later release.

## Boot order

Each step refuses with its own label, and the cheap local checks come
first:

1. the model is set, and this build may call it (a production build with an
   empty allowlist refuses every model, `judgement_model_not_allowlisted`);
2. the receipt signer pins parse;
3. the issuer public keys parse;
4. both caps are set and parse (`judgement_account_cap_absent`,
   `judgement_operator_budget_absent`, `judgement_caps_malformed`);
5. a model that calls NEAR AI refuses with
   `missing_control:judgement_near_gateway_pin`: NEAR AI's gateway quote
   check and the TLS key pin on every connection (spec section 5, check 5)
   are not built yet. They wait on a live fingerprint-mode fixture and on a
   direct `rustls` dependency in the server crate (design open decision S2),
   and land before slice 9;
6. a volume is mounted on `/data` (a different filesystem from `/`);
7. the dstack guest agent derives the limiter root, and a `tcjudge1` boot
   quote binds the two per-boot keys;
8. the sealed ledger opens (`judgement_limiter_unreadable`,
   `judgement_limiter_missing`);
9. the catalog is fetched and the model passes the production check;
10. for a NEAR AI model, the gateway report's DCAP-verified quotes bind
    every pinned signer key (`judgement_model_key_pin_unattested`).

## Keys

| Key | Source | Use |
|---|---|---|
| `judgement-limiter` | `GetKey` on the pinned `/v0/GetKey`, path `judgement-limiter` | 32-byte root; HKDF gives the account-digest key and the ledger seal key |
| sealing key | generated in memory at each boot | key agreement for the sealed request and response bodies |
| response-signing key | generated in memory at each boot | secp256k1 key that signs every response |

- There is no `GetKey` signing path. `GetKey` material is released to every
  image the app's KMS admits, so a key derived there cannot tell the pinned
  image from another admitted one. The two per-boot keys never leave the
  process and are returned only as public values by `GET /v1/attestation`,
  whose quote binds them.
- The limiter path is not the witness's `vms`.
- dstack derives the limiter key from the app id and the path, not from the
  measurement. An in-place upgrade under the same app id keeps it, so every
  account's spend carries over. A new app id or KMS means a new limiter key
  and an unreadable ledger (see the recovery steps below).

## The limiter volume

- One named volume at `/data` holds the sealed ledger
  (`limiter.v1.sealed`) and the marker `limiter.initialized`. Nothing else
  is written anywhere.
- The image creates `/data` owned by the service user, so an empty named
  volume starts writable. The service refuses to boot if nothing is mounted
  there, because a directory on the read-only root is not the volume.
- A ledger that does not decrypt refuses the boot. A missing ledger with the
  marker present refuses the boot. Neither is repaired automatically.
- If both are absent the service creates both and logs
  `event=limiter_initialized`.

### Losing the volume resets every cap, and nothing detects it

The ledger and the marker live on the same volume. A recreated CVM, a volume
that is not reattached, or a rolled-back snapshot boots as fresh, with zero
spend and no refusal. That is a residual risk until a counter exists outside
the volume: each such event allows up to one more $5 per account and one
more $100 of operator budget in that month. The only signal is
`event=limiter_initialized` in the log at a boot the operator did not intend.
Treat that line at any boot but the first as an incident, and record it in
the ledger-reset log below.

### Recovering from the two limiter refusals

Each recovery is a deliberate reset of the month's spend, so each is
recorded.

- **`judgement_limiter_unreadable`** (the file exists and does not decrypt).
  Compare the app id and KMS against "Recorded deployment values". If either
  changed, this is a key rotation (spec section 9, "Rotate"). Otherwise the
  file is corrupt. In both cases keep a copy of the file, delete
  `/data/limiter.v1.sealed` and `/data/limiter.initialized`, and boot.
- **`judgement_limiter_missing`** (the marker is present, the ledger is
  absent). The ledger was deleted or lost. Delete `/data/limiter.initialized`
  and boot.
- Either way, add a row to the ledger-reset log. Spend for the month
  restarts at zero.

### Ledger-reset log

| Date | Profile | Cause | Who |
|---|---|---|---|
| none yet | | | |

## Rotations and secrets

- **A NEAR AI key rotation pauses judgements.** NEAR AI does not announce a
  new provider-TEE key ahead of time, so it cannot be pinned in advance. The
  first receipt from the new key latches the model (`judgement_model_latched`
  in the log): no further call to it is made or charged until the pins
  change. The recovery today is to set the new key in
  `TRACE_COMMONS_JUDGEMENT_MODEL_KEY_PINS`, redeploy (a new measurement), and
  ship a daemon release carrying that measurement; judgements stay paused
  until the release reaches contributors. Updating pins without a daemon
  release is open decision S1.
- Issuer key: list both keys in `TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS`
  during the overlap.
- The only secret is `TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY`
  (production). An upgrade replaces every secret, so each upgrade passes it.

## What is not here yet

- The gateway check (spec section 5, check 5), before slice 9.
- The production model, its adapter and its deploy (slice 9).
- The Jev adapter and the development deploy (slice 6).
- The daemon client (slice 7), the ingest drill and the runbook (slice 8).
