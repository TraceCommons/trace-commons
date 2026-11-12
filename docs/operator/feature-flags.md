# Environment feature flags

Merge features disabled, then publish an environment manifest to change compatible
installed clients without another application release. Flags cover all builds in
an environment; there is no account or percentage targeting.

## Build profiles

`crates/trace-commons-contributor/feature-flags/registry.json` defines flag IDs, owners, descriptions, a
`class`, safe defaults, required capabilities, and `live` or `next_start`
application timing. `profiles/*.json` declares each environment's capabilities,
remote-enable list, and the manifest signing keys it trusts (`signing_keys`).

`class` is `product` or `privacy_sensitive`. Anything that touches consent, what
leaves the machine, upload or the witness is `privacy_sensitive`, and the
validator refuses it in any `remote_enable` list: such behavior ships through a
release, never a remote switch. The validator also refuses a `remote_enable`
flag whose `default` is true (a failed fetch must fail closed), and a profile
with any `remote_enable` entry but no `signing_keys`. The Rust validator and
`scripts/feature-flags/profile.py` (run by `scripts/ci/test-feature-profiles.sh`)
apply the same rules.
Bump the registry version and every profile's matching version when definitions
change. Never reuse a retired ID. Experimental flags default false.

The shipped registry starts empty. No existing feature is automatically gated.
Tests use fixture flags, which are absent from shipped registry/profile data.

Production builds use the production profile. Staging builds default to staging;
the staging workflow also offers a testing profile. Testing uses the same staging
services, app identity and user state, with a separate flag cache. It is not a
third independently installable application. Testing uses a separate update feed
under the staging bucket and does not publish a staging GitHub prerelease, so it
cannot move ordinary staging installs onto testing flags. Get the initial testing
build from its workflow artifacts; later updates stay on that testing feed. `TC_FEATURE_PROFILE` is a build-time
selection, not a runtime override. Production channel builds reject testing.
Bundles compare the CLI, FFI and plist feature identities before signing; the
native application checks its loaded FFI profile against the plist at startup.

## Add a feature

1. Register a stable ID with an owner, description, `class`, false default,
   capability IDs, and timing. Bump registry/profile versions together.
2. Add the flag to the desired profile's `remote_enable` list and declare only
   capabilities the build actually implements. Ship that compatible build.
3. Gate both the UI and the Rust behavior entry point using the daemon's snapshot.
   A missing flag or unavailable daemon is disabled. Never use flags as permission,
   enrollment, consent, credential or server authorization checks.
4. Test both decisions and safe handling of existing sessions. A live disable
   blocks new actions; it does not cancel ongoing work without a feature-specific
   cancellation design. Keep API/storage changes backward compatible.
5. Publish an environment manifest. A new flag in a remote manifest cannot add
   functionality to an older build or override its capability ceiling.

Rust behavior gates read `shared.feature_flags.snapshot().is_enabled("flag_id")`.
Native macOS views query `app.featureIsEnabled("flag_id")`; missing or unavailable
snapshots return false. For a `next_start` flag the effective decision remains
latched in Rust; Swift must not substitute the `pending` value.

## Inspect and validate

```sh
near-ai feature-flags
near-ai feature-flags --json
near-ai feature-flags build-profile --json
near-ai feature-flags validate --file manifest.json --environment staging
near-ai feature-flags verify --file served.json --environment staging
near-ai feature-flags public-key --pem key.pem
```

`validate` checks an unsigned manifest as an operator writes it. `verify` checks
a served, signed document exactly as a client of that build would.

Diagnostics report the environment, immutable profile digest, source, revision,
freshness, effective decisions and restart-pending values. A running daemon owns
the answer. Without a daemon the CLI reads cached/default decisions and reports
when sampled. An incompatible reachable daemon is an error, never a local guess.
No diagnostic read fetches or publishes configuration.

## Manifest and publication

`crates/trace-commons-contributor/feature-flags/manifests/*.json` are deliberately dated examples, not live
configuration. Prepare fresh UTC `issued_at` and `expires_at` timestamps (a
client accepts an `issued_at` up to five minutes ahead of its own clock), an
increasing positive `revision`, `schema_version: 1`, the target `environment`, and
a complete `flags` object of booleans. Validity cannot exceed seven days. Omitted
flags use their compiled defaults. Unknown IDs are ignored by older clients but
rejected by the publication validator. Duplicate fields and invalid snapshots
are rejected as a whole.

The feeds use existing update buckets at `feature-flags/<environment>.json`.
Production uses `tracecommons-flatpak`; staging and testing use
`tc-staging-updates-20261022`. URLs and publication destinations are fixed.
No new ingest service endpoint is required. Manifests contain no personal data.
Flags are never security boundaries.

### Signed manifests

The served object is a signed document:

```json
{ "payload": "<the manifest JSON text>",
  "signatures": [{ "key_id": "<16 hex>", "signature": "<128 hex>" }] }
```

Each signature is Ed25519 over `trace-commons.feature-manifest.v1\n` followed by
the payload bytes. A client reads the payload only after a signature verifies
under a key its build profile pins in `signing_keys` (raw public key, lowercase
hex); anything else is refused and the client keeps its cache or defaults. A
profile with no `signing_keys` refuses every manifest. Up to four signatures
are allowed, for key rotation.

Signing therefore, not the bucket ACL, decides what can turn a flag on. The
buckets are shared with release artifacts: `tracecommons-flatpak` is written by
the release workflow's `GCP_FLATPAK_PUBLISHER_SA` (`release-apps.yml`), and
`tc-staging-updates-20261022` by `TC_STAGING_GCP_PUBLISHER_SA`
(`staging-macos.yml`, bucket from `macos/channels/staging.env`), plus whoever
holds storage admin on those projects. Any of them can delete or replace a
manifest object, which returns clients to their cache and then their defaults;
none of them can enable a flag. They can, however, hold back a disable: leaving
an earlier signed manifest in place, or restoring it, keeps clients that have
not yet seen the newer revision on its values until its `expires_at`. Keep
validity short for flags whose disable is urgent.

### Current keys

Created 2026-11-10 in `tracecommons-pilot-2026`, `us-central1`, key ring
`tc-feature-flags`, algorithm `ec-sign-ed25519`, software protection.
`roles/cloudkms.signerVerifier` on each key is held by `zaki@iqlusion.io`
only; no project-level KMS role is granted, and `roles/editor` carries no
signing permission. KMS data-access audit logging is on for the project.

| Environment | Key (version 1) | Key id | Public key (raw, hex) |
|---|---|---|---|
| production | `feature-manifest-production` | `6c75f4c8298f1563` | `49ce745bfec653ca8187c1c0bb68d18c7afc9dfc6998bc7bc9424bd4081845a5` |
| staging | `feature-manifest-staging` | `026b03ad08ebdbb8` | `044cb512150ded39f6b345ade36aa8038f5eb4ae382fbb08d7ba05cce53babcb` |
| testing | `feature-manifest-testing` | `3678f0265d88264f` | `328a5ecf5805942071d6cfdf822acb98d8d33dc4ff44f2640b1c2ad3475e5a53` |

Each is pinned in its environment's `profiles/<env>.json`. The public keys are
not secret; export a PEM again at any time with the command below.

### The signing key (owner, once per environment)

Use one Cloud KMS key per environment, algorithm `ec-sign-ed25519`, created and
access-controlled as in [`witness-accepted-list.md`](./witness-accepted-list.md):
`roles/cloudkms.signerVerifier` for the owner only, never for a service account
a release, CI, ingest or witness host runs as, with KMS data-access audit logs
on. There is no key file. Do not reuse the witness list key.

```sh
gcloud kms keys versions get-public-key 1 --key <KEY> --keyring <KEYRING> \
  --location <LOCATION> --project <PROJECT> --public-key-format pem \
  --output-file flags-<environment>-v1.pem
near-ai feature-flags public-key --pem flags-<environment>-v1.pem
```

Add the printed `public_key_hex` to that environment's `profiles/<env>.json`
`signing_keys`, and ship that build before publishing. Adding a key changes the
profile digest, which drops cached enables on update but not the rollback
watermark. To rotate, ship a build pinning both keys, then sign with the new one.

Build a fresh CLI from the registry/profile source being reviewed, then preview:

```sh
python3 scripts/feature-flags/publish.py --environment staging \
  --manifest crates/trace-commons-contributor/feature-flags/manifests/staging.json --cli target/debug/near-ai
```

`--publish` also needs the signing key: `--kms-location`, `--kms-keyring`,
`--kms-key`, optionally `--kms-key-version` (default 1) and `--kms-project`, and
`--public-key-pem`. The script has the CLI write the exact bytes to sign, signs
them with `gcloud kms asymmetric-sign`, has the CLI check the signature against
a key pinned for that environment, verifies the document as a client would, and
only then uploads it. A dry run does not sign.

The default previews the revision, value diff, SHA-256, Git commit, destination,
and current object generation. It reads current storage metadata but writes
nothing. Add `--publish` for an intentional change. Use `--account` to select an
already-authenticated account without changing gcloud defaults. Builds never
publish flags. Production publication is a separate explicit operation.

With `--publish` the script also refuses unless the manifest is tracked and
unmodified at HEAD, HEAD is contained in `origin/main` (`git merge-base
--is-ancestor HEAD origin/main`; run `git fetch origin main` first), and the
`--cli` binary's `--version` commit (a short SHA) is a prefix of HEAD. An
unreadable or `unknown` build commit refuses. A dry run prints these as
warnings. A binary built from HEAD with uncommitted source edits still matches;
build from a clean tree.

If no object exists, publication is refused: clients keep their highest accepted
revision after expiry and reject a lower one as `feature-manifest-rollback`, so
republishing revision 1 over a deleted object would silently strand them. Pass
`--initial` only for the very first publish of an environment, with a revision
above any ever published. `--initial` is refused when an object exists; then the
revision must exceed the current one.

The publisher uploads a validated copy of the reviewed bytes, checks freshness
again, and uses a generation precondition. Concurrent publication fails rather
than silently overwriting a newer revision. It does not create buckets or change
access controls. Before relying on live delivery, separately verify a public
anonymous HTTPS GET returns the exact uploaded bytes and expected cache headers.
No environment feed has been deployed merely by merging this implementation.

## Refresh, offline use and rollback

The daemon refreshes at startup, on macOS wake, and approximately every five
minutes with jitter. A wake or `feature_flags_refresh` starts a fetch at most
once every 30 seconds; one arriving sooner is deferred to the end of that
window, not dropped. While a manifest applies the daemon wakes at its expiry,
and at least once a minute to check the wall clock (a Mac's monotonic clock
stops during sleep); otherwise it does no timed work. Requests are anonymous, refuse redirects, time out after
five seconds, and stream at most 64 KiB. The publisher sets a 60-second cache
lifetime. Online propagation is roughly six minutes, not an immediate kill switch.
An empty registry performs no remote requests.

A validated cache is atomic, isolated by environment/profile digest, and bounded.
Fetch failure retains an unexpired accepted snapshot. Expiry restores safe
defaults for live flags, even offline; a running process cannot resurrect an
expired enable after its wall clock moves backward. Revision/content watermarks
remain after expiry and restart. Local cache modification is outside this policy's
threat model. The revision watermark is kept per environment
(`feature-flags-<environment>-watermark.json`), separately from the
profile-scoped cache, so a release that changes the build profile drops cached
enables but still refuses an older revision. Caches of other profiles are
removed at startup. Reinstalling/removing the state directory also removes the
watermark.

`next_start` values latch at daemon startup from valid cache or defaults. A fresh
download changes pending values; restarting a window is insufficient. On a first
install without cache, wait for a successful refresh before restarting the daemon.
Expiry changes pending values without mutating an already-running latched feature.
Use live timing for features that must stop accepting new actions on expiry.

To roll back, publish previous flag values with a new higher revision and fresh
timestamps. Do not restore an old object revision: clients reject lower revisions
and reject changed content at the same revision. Offline clients receive no remote
disable until reconnecting; expiry and feature-specific lifecycle rules still apply.

Retire the gate after rollout and the compatibility window; remove its code and
profile entries, keeping the ID reserved in project history.

## Scope and verification

The contributor runtime/CLI are shared across platforms. Native macOS consumes
the daemon through existing IPC/FFI. Hosted ingest runtime integration and
Windows/Linux UI integrations are follow-up work; these flags cannot disable an
independently deployed server feature. Local tests do not prove live publication,
production deployment, or physical-device/cross-platform UI validation.
