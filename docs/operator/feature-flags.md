# Environment feature flags

Merge features disabled, then publish an environment manifest to change compatible
installed clients without another application release. Flags cover all builds in
an environment; there is no account or percentage targeting.

## Build profiles

`crates/trace-commons-contributor/feature-flags/registry.json` defines flag IDs, owners, descriptions, safe
defaults, required capabilities, and `live` or `next_start` application timing.
`profiles/*.json` declares each environment's capabilities and remote-enable list.
Bump the registry version and every profile's matching version when definitions
change. Never reuse a retired ID. Experimental flags default false.

The shipped registry includes `insights_skills`, a live flag with a false default.
All three build profiles declare `insights_skills_v1` and permit a remote enable
of that flag. Missing, unavailable, expired, or omitted decisions leave Skills
disabled. The dated example manifests keep it false; these files have not been
published. Other existing features retain their behavior.

Skills checks the daemon decision at its inventory, publisher-follow and
background-refresh entry points, and before scheduling skill usage extraction.
The native tab consumes the same daemon snapshot. Disabling it hides the tab and
blocks new Skills work while ordinary Insights remains available. This does not
grant folder access, consent, or permission to update installed skills.

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

1. Register a stable ID with an owner, description, false default, capability IDs,
   and timing. Bump registry/profile versions together.
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
```

Diagnostics report the environment, immutable profile digest, source, revision,
freshness, effective decisions and restart-pending values. A running daemon owns
the answer. Without a daemon the CLI reads cached/default decisions and reports
when sampled. An incompatible reachable daemon is an error, never a local guess.
No diagnostic read fetches or publishes configuration.

## Manifest and publication

`crates/trace-commons-contributor/feature-flags/manifests/*.json` are deliberately dated examples, not live
configuration. Prepare fresh UTC `issued_at` and `expires_at` timestamps, an
increasing positive `revision`, `schema_version: 1`, the target `environment`, and
a complete `flags` object of booleans. Validity cannot exceed seven days. Omitted
flags use their compiled defaults. Unknown IDs are ignored by older clients but
rejected by the publication validator. Duplicate fields and invalid snapshots
are rejected as a whole.

The feeds use existing update buckets at `feature-flags/<environment>.json`.
Production uses `tracecommons-flatpak`; staging and testing use
`tc-staging-updates-20261022`. URLs and publication destinations are fixed.
No new ingest service endpoint is required. Manifests contain no personal data.
Publication trusts HTTPS and the existing restricted storage credentials; v1
does not independently sign manifests. Flags are never security boundaries.

Build a fresh CLI from the registry/profile source being reviewed, then preview:

```sh
python3 scripts/feature-flags/publish.py --environment staging \
  --manifest crates/trace-commons-contributor/feature-flags/manifests/staging.json --cli target/debug/near-ai
```

The default previews the revision, value diff, SHA-256, Git commit, destination,
and current object generation. It reads current storage metadata but writes
nothing. Add `--publish` for an intentional change. Use `--account` to select an
already-authenticated account without changing gcloud defaults. Builds never
publish flags. Production publication is a separate explicit operation.

The publisher uploads a validated copy of the reviewed bytes, checks freshness
again, and uses a generation precondition. Concurrent publication fails rather
than silently overwriting a newer revision. It does not create buckets or change
access controls. Before relying on live delivery, separately verify a public
anonymous HTTPS GET returns the exact uploaded bytes and expected cache headers.
No environment feed has been deployed merely by merging this implementation.

## Refresh, offline use and rollback

The daemon refreshes at startup, on macOS wake, and approximately every five
minutes with jitter. Requests are anonymous, refuse redirects, time out after
five seconds, and stream at most 64 KiB. The publisher sets a 60-second cache
lifetime. Online propagation is roughly six minutes, not an immediate kill switch.
An empty registry performs no remote requests.

A validated cache is atomic, isolated by environment/profile digest, and bounded.
Fetch failure retains an unexpired accepted snapshot. Expiry restores safe
defaults for live flags, even offline; a running process cannot resurrect an
expired enable after its wall clock moves backward. Revision/content watermarks
remain after expiry and restart. Local cache modification is outside this policy's
threat model. Reinstalling/removing the cache also removes its revision watermark.

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
