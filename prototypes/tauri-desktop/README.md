# Trace Commons Tauri desktop prototype

Cross-platform desktop shell calling the existing Rust contributor daemon, with
daemon-backed macOS feature parity wired in the current prototype.

Architecture, completed slice, remaining phases, parity gates, and residual
risks live in [ARCHITECTURE.md](./ARCHITECTURE.md).

## Delivery phases

1. Shell foundation: Tauri, TypeScript, React, shared brand tokens, navbar, and profile landing page.
2. Profile capability: Rust command contract for read/publish/withdraw states; no direct UI access to daemon internals.
3. Insights capability: local file analysis, saved history, detail, assessment, and delete.
4. Queue/settings: review actions, pause/resume, mission drafts, consent, roots, and platform integrations.
5. Parity and release: native integrations, packaging, signing, accessibility, and removal of legacy shells only after acceptance.

Current work covers all top-level prototype routes, native-ordered resumable onboarding (source roots, invite resolution, cold-start/while-running invite deep links, invite enrollment, NEAR AI enrollment, NEAR wallet ceremony, consent, generated scrubber disclosure, optional third-party scan, project policy, done), tenant-scoped completion state, real Rust reads/mutations for profile, explicit public-profile acknowledgement before first publication, profile-aware navbar identity, grouped queue review with project-level submit, runtime safeguards, arming suggestions, certificate/attestation display, approval undo, bounded redacted transcript paging, turn indexing, original-session match counts, explicit admission preparation, explicit witness preview review, queue outcome disclosure, project-grouped history, withdrawal/community standing, signed credit-record presentation, account-owned session detail, exact reviewed public-run create/edit/unpublish flow, accepted-session skill learning with candidate review, held-out evaluation, install preview, digest-bound install, and rollback, local audit visibility, project policy, consent editing, witness trust configuration, behavior guardrails, privacy-evidence and token-review controls, local Insights snapshots/episodes/question cards/comparison tasks/specifications, bounded test-report evidence linking, local Git evidence linking, Rust-owned native folder picking, mission drafts, compute controls, private-AI credential/balance/funding status and configured-tool inventory, source roots, daemon controls, routing discovery/configuration/probes, native browser opening for allowlisted destinations, and a Tauri tray menu using the packaged app icon. Each phase keeps one public feature entry, explicit state ownership, and a review gate before the next phase.

## Run

From `prototypes/tauri-desktop`:

```bash
./scripts/dev.sh
```

`dev.sh` starts Vite on port 1420 and the Tauri shell with live frontend
reloads. Use `Ctrl-C` to stop both processes.

```bash
./scripts/build.sh        # Debug frontend and Rust build
./scripts/build.sh --release
./scripts/start.sh        # Build Release, then launch the static app
./scripts/start.sh --release
./scripts/start.sh --no-build
```

`dev.sh` and `build.sh` install frontend dependencies from frozen lockfile.
The local build uses Tauri's per-user application-data directory. It does not
use PostgreSQL or Docker to launch the desktop app. It does not touch the
production macOS, Windows, or GTK app bundles.

## What this proves

- Tauri owns one desktop window and frontend-to-Rust command bridge.
- TypeScript and React own UI composition; feature state stays inside its feature.
- Storybook stories exercise feature screens without starting Tauri.
- Existing `trace-commons-contributor` starts as an embedded Rust daemon.
- The frontend reads daemon status through one application command.
- Existing local Insights and mission-draft services run through bounded Rust commands.
- Rust validates native-picked source roots and Git repositories; Insights links local Git commits and bounded test reports without claiming success.
- External browser actions run through a Rust-owned allowlist; React cannot open arbitrary URLs.
- Insights episode grouping, deterministic question cards, comparison task freezing, and specification evaluation reuse existing versioned Rust contracts; the UI does not calculate derived facts.
- The current Swift, WinUI, and GTK shells remain untouched.

This is source/build plus unsigned universal-Darwin-bundle proof, not signed OS release proof. macOS
notification permission, login-item control, daemon-derived tray state,
weekly digest data, quit interception, credential/public-run/review route
validation, redacted keychain status, and a signed release path are source-wired.
Homebrew installs report managed updates; other installs report unmanaged
updates. OS permission/action smoke, self-update feed, deep certificate review,
native file grants, keychain migration proof, signing/notarization,
accessibility audit, and cross-platform smoke remain open. No self-update
dependency is included.
Private-AI browser credential start/status/cancel/forget, account-balance
display, verified funding destination, configured-tool inventory,
plan-before-commit harness wiring, and the queue's private-inference offer are
implemented; imported test reports remain producer assertions, and linking
does not run or verify tests. See [ARCHITECTURE.md](./ARCHITECTURE.md) for
phase gates and acceptance evidence.
