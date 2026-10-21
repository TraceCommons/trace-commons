# Deploying the Insights judgement service on dstack

Design: [`docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`](../../docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md)
(spec PR #1501). The service follows the redaction witness's deployment
([`deploy/witness/README.md`](../witness/README.md)); read that first. This
file records what differs and, once there is a deployment, its values.

The operator runbook (`docs/operator/insights-judgement-service.md`) is
slice 8. Until it lands, this file is the only deployment note.

## Recorded deployment values

| Profile | App id | CVM id | KMS | Image digest | Measurement | Signing address |
|---|---|---|---|---|---|---|
| production | not deployed (slice 9) | | | | | |
| development | not deployed (slice 6) | | | | | |

Read every value from the running instance, never from a local manifest:
`phala deploy` ignores `app-compose.json` and builds its own (see
`deploy/witness/build-app-compose.sh`). The measurement is `compose_hash`
from `phala cvms get --json`, cross-checked against the `mrconfigid` in a
quote from `GET /v1/attestation`.

## Two profiles, two images, two apps

| Profile | Image | Cargo features | Models it can call |
|---|---|---|---|
| `production` | `ghcr.io/tracecommons/trace-commons-judgement` | `near-attestation-collateral` | The production allowlist. Empty until slice 9. |
| `development` | `ghcr.io/tracecommons/trace-commons-judgement-development` | `judgement-development-models` | The mock now; Jev in slice 6. |

- A development model is not in the production binary, so no setting can
  turn one on there.
- Each profile is its own dstack app. Keys derive from the app id, so a
  development deployment never derives the production keys, even from an
  identical image.
- The production image cannot boot today. Owner decision 1: production
  waits for the TEE-hosted decision model, and the allowlist stays empty
  until it passes spec section 5.

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

## Boot order

Each step refuses with its own label, and the cheap local checks come
first:

1. the model is set, and this build may call it;
2. the receipt signer pins parse;
3. the issuer public keys parse;
4. both caps are set;
5. a volume is mounted on `/data` (a different filesystem from `/`);
6. the dstack guest agent derives both keys and a `tcjudge1` boot quote;
7. the sealed ledger opens (`judgement_limiter_unreadable`,
   `judgement_limiter_missing`);
8. the catalog is fetched and the model passes the production check;
9. for a NEAR AI model, the gateway report's DCAP-verified quotes bind
   every pinned signer key (`judgement_model_key_pin_unattested`).

## Keys

Both keys come from `GetKey` on the pinned `/v0/GetKey` surface, each on its
own path. Neither is the witness's `vms`.

| Path | Use |
|---|---|
| `judgement-signing` | secp256k1 key that signs every response |
| `judgement-limiter` | 32-byte root; HKDF gives the account-digest key and the ledger seal key |

dstack derives keys from the app id and the path, not from the measurement.
An in-place upgrade under the same app id keeps both keys, so the signing
address and every account's spend carry over. A new app id or KMS means a
new signing address and an unreadable ledger.

## The limiter volume

- One named volume at `/data` holds the sealed ledger
  (`limiter.v1.sealed`) and the marker `limiter.initialized`. Nothing else
  is written anywhere.
- The image creates `/data` owned by the service user, so an empty named
  volume starts writable. The service refuses to boot if nothing is mounted
  there, because a directory on the read-only root is not the volume.
- A ledger that does not decrypt refuses the boot. A missing ledger with the
  marker present refuses the boot. Neither is repaired automatically.

## Upgrades and secrets

- Upgrade in place with `phala cvms upgrade`. Never delete and recreate:
  Phala's docs do not say a volume survives that, and the ledger would be
  lost.
- An upgrade replaces every secret, so each upgrade passes the full set. The
  only secret is `TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY` (production).
- A new measurement pauses judgements for release daemons until a daemon
  release compiles it in (owner decision 2).

## What is not here yet

- The production model, its adapter and its deploy (slice 9).
- The Jev adapter and the development deploy (slice 6).
- The daemon client (slice 7), the ingest drill and the runbook (slice 8).
