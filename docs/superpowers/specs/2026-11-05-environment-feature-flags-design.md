# Environment feature flags

Status: approved by owner on 2026-11-05; implementation in progress.

## Intent and decisions

Merge incomplete features into main behind flags, then enable or disable them
remotely for whole environments without rebuilding installed applications.
The owner chose environment-wide targeting and accepted the recommendation to
publish versioned configuration manifests. No account targeting, percentage
rollouts, hosted flag vendor, or administration UI is needed in v1.

The details below are proposed implementation choices, not additional owner
decisions. Implementation begins after approval of this document.

## Approaches

1. Build-only profiles are reproducible but cannot change installed clients.
2. Versioned remote manifests plus build defaults meet the requested scope with
   a small runtime and a Git-reviewed publishing path. This is the selected approach.
3. A database and operator API support targeting and interactive administration,
   but introduce infrastructure unnecessary for whole-environment switches.

## Existing constraints

The contributor's channel module owns staging/production hosts and identities.
Native Swift reads channel identity through Rust FFI and checks bundle agreement.
Release checks prevent staging destinations and credentials entering production.
Keep these controls intact. A flag cannot select an endpoint, change credentials,
grant consent, or authorize a server operation.

The existing Swift campaign feed is useful transport precedent but must not own
feature decisions: headless CLI and daemon behavior also need the same policy.
Permissive client code must not depend on an AGPL server crate. Do not add new
dependencies without explicit owner approval.

## Scope

Build a shared flag registry/evaluator, client refresh/cache service, FFI snapshot,
native macOS observation, build-profile packaging, validation, and an explicit
manifest publication path. Include an operator guide for adding and retiring flags.
Existing product features retain their behavior until deliberately migrated;
the initial delivery proves the complete path using a test fixture flag, without
silently putting an existing feature behind a switch.

The evaluator and wire types live in permissive code reusable by server builds.
Hosted ingest refresh wiring and individual server feature migrations are follow-up
work; v1 must not claim to disable an independently deployed server feature.
Windows/Linux native UI integrations are also follow-ups; the shared contributor
runtime and CLI diagnostics work wherever that runtime is supported.

## Environment and build contract

Feature environments are `production`, `staging`, and `testing`. They are distinct
from service channels. Existing production and staging builds default to the
matching feature environment. A testing profile explicitly opts into testing
flags while using the existing staging service channel and app identity; it is
not a third independently installable app or service deployment. Production
release builds cannot select the testing profile.

Each artifact embeds its feature environment, registry/schema version, profile
digest, and enabled build capabilities. Registry entries define stable flag ID,
description, owner, safe default, required capabilities, and application timing
(`live` or `next_start`). New experimental flags default false. The build profile
declares which flags remote configuration may enable; a flag outside that set
stays false even when a manifest requests true. Retired IDs are never reused.

CLI and FFI artifacts in a bundle must agree on the profile digest and environment.
Do not rely on a Swift-only bundle value to enable Rust behavior. The builder
may choose the precise packaging/generation mechanism using existing tools.

## Remote manifest

Publish separate HTTPS documents for each environment from fixed, channel-owned
locations. No runtime arbitrary URL overrides in shipped builds. Testing feeds
are hosted on staging infrastructure. Transport is anonymous, sends no account
identifiers, refuses redirects, and bounds time and body size.

A manifest has schema version, environment, increasing revision, issue time,
expiry time, and a map of registered flag IDs to explicit booleans. It represents
a complete snapshot: omitted flags use registry defaults, never previous values.
The publisher validates IDs and capabilities; older clients ignore unknown IDs
for forward compatibility. Unsupported schema, wrong environment, malformed or
duplicate fields, invalid times, and invalid known flag values reject the whole
manifest. Require expiry after issue time and limit validity to seven days.

Trust v1 publication through HTTPS and restricted existing deployment credentials.
Independent application signatures/key rotation are outside v1. A remote flag is
never an authorization boundary. Reject lower revisions than the highest accepted
revision; an identical revision must have identical content. Rollback publishes
old values under a new revision. Local cache tampering is outside this threat model.

## Resolution and lifecycle

One Rust evaluator owns resolution: embedded defaults, then the latest accepted
unexpired environment manifest, then capability restrictions. No user-facing
flag switches or production environment-variable overrides. Test-only injection
is allowed through explicit test APIs.

Fetch at runtime startup and every five minutes with jitter while running;
resume/wake requests a refresh without creating duplicate fetch loops. Requests
have a five-second timeout and a 64 KiB streamed size limit. Do not block startup
or foreground UI on network access. HTTP/CDN cache policy must keep online change
propagation within roughly six minutes; this is a polling target, not an offline
kill-switch guarantee.

Persist a validated snapshot and revision atomically in an environment/profile
scoped cache under the existing channel state directory. Revalidate on load.
On fetch failure use an unexpired accepted snapshot; with no valid snapshot or
after expiry use compiled safe defaults. Rejecting a response must not replace
a valid cache. Expiry is evaluated even while offline, and profile changes do not
carry cached enables into a different artifact profile.

Publish immutable effective snapshots to Rust callers and through existing
daemon/FFI patterns to Swift. Runtime state is owned by the daemon where running;
the GUI must not run an independent evaluator or poller. Standalone commands use
the same evaluator/cache semantics and report when their decision was sampled.
Avoid doing networking or disk writes inside synchronous FFI getter calls.

Live flags affect new actions immediately after snapshot publication. A gate is
checked at the behavior entry point as well as in UI visibility. Existing sessions
finish safely unless a feature-specific cancellation design explicitly exists.
`next_start` means daemon process startup, not opening another GUI window. Before
starting feature work, latch values synchronously from validated cache or defaults;
the nonblocking startup fetch updates pending values only. An initial install with
no cache therefore uses defaults until a successful refresh and a subsequent daemon
restart. Expiry updates pending values to defaults without mutating an already
latched decision. Features requiring a live expiry/disable must use `live` timing.
Standalone commands latch at command startup. Diagnostics distinguish current and
pending values and explicitly say when a daemon restart is required. Flags cannot
substitute for backward-compatible storage migrations or API contracts.

## Operations and diagnostics

Keep registry, build profiles, and environment manifests in Git. An explicit
publication command/workflow validates, shows the environment/revision/changes,
and publishes only the selected environment with scoped credentials and an atomic
object replacement. Ordinary application builds do not publish flags. Deploying
this implementation does not itself authorize publishing production changes.

Provide CLI diagnostics for environment, build profile digest, manifest revision,
source (`default`, `remote`, `cache`), freshness, effective flags, capability blocks,
and pending restart changes. No credentials or personal identifiers are logged.
Document that rollback means a new revision and offline clients cannot receive
an immediate disable. Remove a flag after its rollout and compatibility window.

## Verification and acceptance

Test resolution, capability ceilings, environment isolation, omitted/unknown flags,
invalid snapshots, expiry, cache failures, revision rollback, restart latching,
and bounded transport. Exercise restart/offline transitions with a fake clock and
local transport, and verify CLI/FFI/Swift snapshot agreement. Build packaging tests
must reject mismatched artifacts and production/testing profile combinations.

A local end-to-end fixture must demonstrate a supported flag changing off/on/off
without rebuilding, an unsupported enable staying off, and expiry restoring defaults.
Run relevant Rust/Swift checks and the repository license-boundary test. Preserve
existing staging/production release checks. No live environment publication,
production deployment, or full cross-platform UI validation is implied by local
verification.

## Left to implementation planning

Choose exact Rust module boundaries, existing IPC subscriptions to extend, profile
generation mechanics, and publication commands after inspecting those interfaces.
Prefer existing dependencies. If a dependency or a new service is necessary,
bring that concrete requirement back before adding it.
