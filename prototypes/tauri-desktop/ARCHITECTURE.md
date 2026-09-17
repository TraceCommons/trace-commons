# Tauri desktop architecture

Status: active Tauri/React/Rust buildout. Daemon-backed feature parity and
source-level macOS native adapters are wired for the current shell. Signed
runtime, provider, and cross-platform evidence remain phase-gated below. This
document is the source of truth for what is done, what is intentionally
deferred, and what evidence is required before legacy shells can be removed.

## Decision

Use Tauri 2 as one desktop shell, React + TypeScript as the UI, and the existing Rust contributor crate as the local application core.

The frontend never imports Rust crates or reaches into daemon state. It calls a small command boundary in `src-tauri`. Rust owns daemon lifecycle and domain behavior. Platform-specific features remain thin Tauri/plugin adapters.

## Ownership

| Area | Owner | Boundary |
| --- | --- | --- |
| Window, tray, native lifecycle | Tauri/Rust shell | Tauri commands and events |
| Navigation composition | `src/app` | Route IDs and public feature entries |
| Profile UI/draft and navbar identity | `src/features/profile` + `src/app` composition | `ProfilePage` public entry; app shell owns shared profile read |
| Rust status transport | `src/lib/tauri` | Parsed `CoreStatus` |
| Enrollment flows | `src/features/onboarding` + `src-tauri/src/commands/native_flows.rs` | Invite, deep-link, NEAR AI, and NEAR wallet commands |
| Queue grouping/approval/undo, certificate and attestation facts, history grouping/credit record/detail/withdrawal/community rollup/public-run publication, accepted-session skill learning/install, project policy, consent, local audit, privacy, witness, behavior guardrails, routing, daemon, Insights, mission drafts, compute | `trace-commons-contributor` | Existing Rust API and local stores |
| Admission, witness preview, and queue outcomes | `src/features/waiting` + `src-tauri/src/commands/native_flows.rs` | Explicit review actions, daemon-owned refusal views, read-only outcome counts |
| Deep-link intake and browser safety | `src-tauri/src/commands/platform.rs` | Rust validation, pending cold-start/while-running delivery, same-origin wallet opening |
| Keychain, notifications, login item, tray state, updates, signing | `src-tauri/src/native.rs`, `src-tauri/native_macos.m`, `src-tauri/src/commands/platform.rs`, `src-tauri/src/tray.rs`, release script | Explicit Tauri commands/events and per-OS adapters; no platform policy in React |

## Rules

- One feature owns its page, workflow hook, components, types, and stories.
- Shared code stays domain-free; it cannot import features.
- Untrusted command results are parsed at the UI/Rust command boundary.
- Read-only daemon IPC stays allowlisted. Mutations use named Tauri commands; no generic write bridge exists.
- Profile draft state is local React state, hydrates once from the Rust-owned public profile, and uses an explicit acknowledgement modal before first publication; publish/withdraw calls use explicit Rust commands. Onboarding scrubber disclosures read detector names from a named Rust command; TypeScript never copies privacy-detector policy.
- No frontend code depends on AGPL server or gate crates; the Tauri shell uses permissive contributor/protocol crates only.
- No page is promoted from placeholder until its read, loading, empty, error, retry, and mutation states are specified.
- Local mutations cross named commands only. Each command accepts the minimum fields needed by its Rust operation.
- Source files enter Rust as selected bytes staged inside the prototype state directory; arbitrary frontend paths are never accepted.
- External browser opening is a named Rust command limited to loopback credential flow, canonical NEAR Cloud credits origin, and canonical Trace Commons public-run URLs.
- Native wallet browser opening is a separate Rust command limited to the exact HTTPS origin returned by the enrolled commons. Wallet lifecycle state never crosses through a generic write bridge.
- `consume_deep_link` accepts only validated enrollment, credential-provider,
  and public-run shapes. Invalid payloads are consumed and rejected before
  React navigation. Credential callbacks carry provider identity only; they
  never carry a code, token, or secret.
- NEAR AI enrollment, admission preparation, and witness review use named commands. Rust supplies fixed status/refusal views; React never maps daemon control labels into user copy.
- Queue outcome disclosure calls read-only `queue_outcome_counts` plus Rust-owned `queue_outcome_line`; it states that entries discarded before queue creation are outside this count.
- Local audit is visibility-only: Rust supplies newest-first fixed labels; React does not authorize, block, or enrich entries.
- Queue grouping uses daemon-minted `project_id`; project-level approval crosses a named command and reports daemon exclusions instead of reimplementing eligibility in React.
- Withdrawal uses account-session-gated Rust IPC and renders distribution reach only when returned by the daemon; a generic success message is not allowed.
- Consent, witness, and behavior controls use named commands with narrow validation; no generic settings-write bridge crosses into React.
- Status-derived queue and community panels render only daemon-provided facts; unavailable routing, budget, and standing values stay unavailable.
- Daemon events cross into React as allowlisted event names only. Event bodies,
  paths, transcript text, and credentials never cross the event bridge.
- Native notification actions have Review and Not now only. Review foregrounds
  the app; the app remains responsible for its normal waiting route.
- Native Swift, WinUI, and GTK remain reference implementations during migration.

## Phase gates

### Phase 1 — shell, profile landing, and queue reads

Accepted when TypeScript build, Storybook build, Rust build, and launch smoke test pass. Source/build gates met; launch smoke reached the debug binary without startup error on 2026-09-15, then was intentionally interrupted. No UI automation or OS packaging proof follows from that smoke.

### Phase 2 — local contributor workflows

Profile publish/withdraw, queue preview/dismiss/approve, local Insights snapshots/episodes/question cards/comparison tasks/specifications, mission drafts, compute settings, account-owned history detail, reviewed public-run create/edit/unpublish, skill learning/evaluation/install/rollback, and daemon pause/resume use named Rust commands. Met for prototype scope. Remaining gate: unified mutation error taxonomy and fixture-backed daemon acceptance. Do not add network calls directly to React.

### Phase 3 — remaining Insights parity

Bounded queue transcript paging, turn indexing, and original-session match counts now use the daemon's anchored preview-body contract. Native folder selection, Git evidence linking, and bounded test-report linking now cross named Tauri commands. Cancellation, reconciliation, and cross-client stale-store handling remain behind their existing feature boundaries. Preserve account-free behavior.

### Phase 4 — contributor workflows

Native-ordered onboarding (source roots, invite/deep-link intake, invite enrollment, NEAR AI enrollment, NEAR wallet ceremony, consent second, optional third-party scan disclosure, project policy, done), tenant-scoped completion state, source-root declarations, project policy, consent editing, local audit visibility, witness trust configuration, behavior guardrails, optional privacy-evidence/token-review controls, private-AI browser credential lifecycle, account-balance display, verified funding destination, configured-tool inventory, routing discovery/configuration/probes, bounded queue transcript/turn inspection, original-session match counts, explicit admission preparation, explicit witness preview review, queue outcome disclosure, private-inference queue offer, community history standing, reviewed public-run create/edit/unpublish, accepted-session skill learning/evaluation/install/rollback, and plan-before-commit harness writes are wired as separate vertical slices. Tray menu/icon, Rust-owned native folder picking, local Git evidence linking, bounded test-report linking, selected macOS notification/login-item flows, digest routing, and quit interception are also source-wired. Windows/Linux adapters, updater ownership, deep certificate review, native file grants, keychain lifecycle proof, packaging/signing, and remaining account flows stay phase-gated.

### macOS parity matrix for current shell

| macOS reference flow | Tauri implementation | Current proof boundary |
| --- | --- | --- |
| Invite paste, issuer preview, and enrollment | `InviteConnectForm`, `enroll_with_invite` | TypeScript compile/build; daemon-backed call is source-wired |
| `tracecommons://enroll` from cold start or running app | `consume_deep_link`, pending app state, `AppShell` polling | Rust parser tests; OS scheme registration still unproved |
| Existing NEAR AI login enrollment | `OnboardingNearAiJoin`, `near_ai_account_enroll` | Named command and fixed UI states; live provider enrollment unproved |
| NEAR wallet capability/start/wait/cancel | `OnboardingWalletConnect`, `native_wallet_flow`, same-origin URL command | Rust lifecycle/URL tests; wallet completion unproved |
| Queue “not offered” reasons | `queue_outcome_counts`, `QueueOutcomeDisclosure` | Read-only parser/build; pre-queue discard reasons remain outside daemon contract |
| Admission preparation | `AdmissionPreparationOverlay`, `prepare_admission_session` | Rust-owned readiness/refusal view; live commons evidence unproved |
| Explicit witness preview review | `WitnessReviewOverlay`, `witness_preview_support`, `witness_preview_request` | Rust-owned support/refusal view; pinned witness availability unproved |
| Certificate/attestation detail | Existing `CertificatePanel` plus history facts | Summary facts only; full certificate review remains Phase 5F |
| Notifications and login item | macOS Objective-C bridge in `native_macos.m`; typed capability/request commands; Review opens `tracecommons://review` | Source/build hook only; permission, relaunch, action, and external-disablement smoke pending |
| Tray/menu/flyout | Daemon-derived counts, project summaries, pause/resume, private-inference stop, weekly rollup, Review/Open/Settings/Quit | Rust compile only; native menu-bar smoke pending |
| Credential/keychain status | Daemon-owned OS-secret lifecycle plus redacted Tauri status (`state`, prefix, expiry, migration) | Source/build only; OS backend, migration, wipe, and crash/restart proof pending |
| Credential/public-run/review deep links | Rust validation plus cold-start/while-running delivery; provider-only credential route; notification Review route | Parser tests only; packaged scheme registration and callback smoke pending |
| Updates | Homebrew-aware installer-owned contract; unmanaged installs remain explicit; no self-replacement | No self-update feed/plugin; ownership decision required before adding updater dependency |
| Signed packaging | Universal Darwin bundle build, ad-hoc signature verification, release config, and sign/notarize/staple/hash script | Developer ID signature, notarization credentials, stapling, Gatekeeper, and clean-machine smoke |
| Certificate/attestation detail and file grants | Existing summary UI and Rust path validation | Full certificate review, bookmarks, portal grants remain Phase 5F |

### Phase 5 — native parity and release

Source-level parity is partially implemented. Full Phase 5 acceptance is not
complete until runtime and release evidence below exists. Remove legacy shells
only after explicit parity acceptance.

## Slice completed in current buildout

The current slice is complete when these facts hold:

- Rust owns native directory selection through `pick_directory`. React never
  executes picker commands or accepts an arbitrary frontend path as trusted.
- Source-root selection accepts only absolute, existing directories. Rust
  canonicalizes them before saving. Git evidence accepts only an absolute,
  existing directory containing `.git` as a directory or worktree file.
- Insights exposes both existing evidence types: local Git commit inspection,
  bounded imported test reports, and explicit unlink. Links remain user-linked
  evidence; they never become success, merge, execution, or causation claims.
- Tauri uses its per-user application-data directory. The prototype no longer
  creates one process-ID temp store per launch.
- Fresh startup opens the config store and compute controller first. The daemon
  starts only after both required source declarations exist; onboarding can
  persist those declarations and start the embedded daemon in the same action.
- Startup without roots still serves account-free Insights and mission drafts,
  settings reads, consent options, and an honest `needs_roots` core status.
- Existing Swift, WinUI, GTK, AGPL server, and gate crates remain untouched by
  this slice. The only shared Rust change is the permissive contributor
  re-export of scrubber names used by the Tauri disclosure.

## Remaining work, phase by phase

### Phase 5A — platform capability contract

Status: source contract implemented. `platform_capabilities` and typed platform
commands exist; Homebrew cask installs report `managed`, other installs report
`unmanaged` until self-update ownership is explicitly selected.

The Rust-owned capability snapshot and sanitized event stream are implemented.
The frontend renders `available`, `unavailable`, `denied`, `not_found`,
`requires_approval`, `managed`, `unmanaged`, and `unknown` explicitly; it does
not infer capability from the operating-system string or package state.

Required command boundaries:

- `platform_capabilities`: OS, package identity, startup state, notification
  permission, update ownership, deep-link support, and tray availability.
- `set_start_at_login`: user-requested enable/disable, with read-back state.
- `notification_permission` and `request_notification_permission`.
- `update_status`, `check_for_update`, and `apply_update`: Homebrew-managed and
  installer-owned `unmanaged` responses are implemented. Self-update and daemon
  quiesce remain intentionally unavailable until update ownership is approved.
- `consume_deep_link`: enrollment, provider-only credential, and public-run
  routes are implemented through the same Rust validation boundary.

The capability snapshot is the compatibility seam. It prevents React from
containing one macOS rule, one Windows rule, and one Linux rule that drift.

### Phase 5B — notifications and startup lifecycle

Status: macOS source adapter and review-safe digest scheduling are implemented;
Windows/Linux adapters and OS smoke evidence remain open.

Migrate native behavior into thin adapters while preserving shared policy:

- macOS: `UNUserNotificationCenter`, digest category/actions, permission status,
  passive digest delivery, Review deep-link routing, and
  `SMAppService.mainApp` login-item state are wired in `native_macos.m`.
- Windows: packaged notification path and unpackaged fallback, plus package
  startup task or the approved per-user portable mechanism.
- Linux: portal notification path and XDG autostart. GNOME has no required
  system-tray surface; KDE status-notifier support is optional enhancement.
- Digest cadence: at most one notification every four hours, silence at zero,
  Review/Not now only, never Approve or Submit from a notification.
- Login-item changes are explicit user actions, read back after mutation, and
  never silently re-enable an external user disablement.

Acceptance evidence still required: permission denied, permission granted, zero
waiting, non-zero waiting, paused watcher, no network, app quit, relaunch, and
external startup disablement on each supported OS. Current local evidence is
limited to Rust compilation and command tests.

### Phase 5C — tray/menu/flyout parity

Status: daemon-derived macOS menu model, weekly rollup, and bounded refresh
cancellation are source-wired. Windows/Linux native surfaces remain open.

The Tauri menu now uses a daemon-derived model:

- count decisions owed, never unread count;
- inert project/session summary lines with project labels, counts, and sizes;
- Review as only forward action;
- Pause submenu with bounded durations and Resume when paused;
- private-inference state routes to Settings and may be stopped from the menu;
- Open, Settings where platform design requires it, and Quit confirmation;
- weekly contributed/held rollup when `history_rollup` returns it;
- no paths, bodies, credentials, or Approve action in tray surfaces.

macOS follows the menu-bar design, Windows follows the tray flyout design,
Linux keeps the window primary and uses portal notifications. Tray updates must
be event-driven or bounded polling with shutdown cancellation; no background
thread may outlive the Tauri app state.

### Phase 5D — credentials, keychain, and account lifecycle

Status: contributor-daemon OS-secret lifecycle and redacted Tauri status are
source-wired. Backend and lifecycle smoke evidence remain open.

The Tauri shell consumes the contributor crate's OS-secret lifecycle with no
secret fields crossing the frontend boundary:

- verify macOS Keychain, Windows Credential Manager, and Linux secret-service
  behavior for account/inference credentials;
- expose presence, key prefix, expiry, and migration/refusal labels only;
- preserve logout/wipe semantics for config, device key, account session,
  approved envelopes, queue, history, and staged updates;
- handle an existing daemon, lock contention, daemon crash, and restart;
- distinguish local account-free Insights from account-owned contribution
  history and withdrawal.

No credential is placed in localStorage, Storybook fixtures, tray labels,
  telemetry, or error text.

### Phase 5E — update ownership and release packaging

Status: Homebrew-aware installer-owned update contract and universal macOS
release path are source-wired; self-update and full release acceptance remain
open.

Keep replacement authority with the installer/deployment system:

- macOS: `tauri.release.conf.json` enables app bundling and hardened runtime;
  `scripts/package-macos-release.sh` injects the release version, builds,
  universal `arm64` + `x86_64`, signs, notarizes with an App Store Connect API
  key, staples, and hashes ZIP/DMG artifacts when release credentials and Tauri
  CLI are present. The final ZIP contains the stapled app. Sparkle remains
  intentionally absent; Homebrew remains Homebrew-owned.
- Windows: MSIX/App Installer feed, package identity, startup extension, and
  daemon quiesce before deployment; unpackaged builds truthfully report
  unmanaged updates.
- Linux: Flatpak update portal for Flatpak builds; source builds do not fetch or
  replace themselves.
- configure signed manifests, artifact hashes, rollback/failure behavior,
  version migration, and clean restart after update.
- debug config keeps `bundle.active` false; release overlay enables bundling
  only for the explicit packaging path. The packaging script has not been run
  in this workspace because no `cargo-tauri`/`tauri` CLI or signing credentials
  are available.

Acceptance evidence includes fresh install, upgrade with queued work, update
refused during upload, update feed unavailable, signature mismatch, rollback,
and restart into the new version.

### Phase 5F — deep native integrations

Status: credential/public-run/review route validation is implemented. Full
certificate/attestation detail and native file-grant lifecycle remain open.

- Certificate/attestation: show the full certificate, signer, measurement
  trust, receipt, expiry, and refusal reason. Do not collapse attestation into
  a generic "verified" badge.
- Deep links: enrollment invites, provider-only credential routes, public-run
  routes, and the notification Review route are wired through
  `consume_deep_link`; malformed or unexpected payloads are rejected before UI
  navigation.
- Native file access: retain Rust validation for every selected path. Add
  security-scoped bookmarks on macOS, Windows picker permission handling, and
  Flatpak portal grants where packaging requires them.
- Git/test evidence: current slice is wired; add cancellation, stale-store
  conflict handling, unlink confirmation policy, and cross-client refresh
  behavior.

### Phase 5G — UI parity and accessibility

Status: platform settings and quit confirmation use existing shadcn primitives
and desktop modal/mobile drawer behavior. Full native visual and accessibility
audit remains open.

Bring parity beyond route availability:

- adopt the “The Turn” mark in window chrome, title bar, menu/tray, and
  notification surfaces, including template behavior on macOS;
- match native spacing, typography, light/dark states, empty/error/loading
  states, and platform navigation conventions;
- keyboard traversal, focus restoration after modal/drawer close, screen-reader
  names, reduced motion, contrast, and minimum hit targets;
- native dialogs on desktop, drawers on narrow/mobile layouts, and no hidden
  destructive action behind an unlabeled control;
- add localization boundary before adding supported languages. Shared Rust
  copy remains the authority for consent, privacy, and limitation sentences.

### Phase 5H — verification and migration gate

Status: local source checks and an unsigned universal Darwin bundle/startup
smoke are available; signed-release, provider, and state-lifecycle evidence are
not complete. The current host has no installed Tauri CLI, signing identity, or
notarization credentials; an ephemeral Tauri CLI 2 produced the bundle.

Run evidence in layers:

1. Source: TypeScript build, Storybook build, Rust check/test, format check,
   license-boundary test, and dependency-license checks when dependencies
   change. Current dependency set adds no Tauri plugin.
2. Tauri: launch, command bridge, clean shutdown, no residual daemon/lock,
   fresh app-data store, migrated app-data store, and account-free routes.
3. OS: signed/package smoke on macOS, Windows, and Linux; picker,
   notifications, login item, tray/flyout, deep link, update, and keychain.
4. Provider/state: separate live enrollment, database, network, update-feed,
   and settlement evidence from local source/build evidence.

Parity acceptance requires a written row for every native capability and every
platform. A passing local macOS build cannot prove Windows packaging, Linux
portals, provider enrollment, update-feed availability, or live settlement.
Legacy Swift, WinUI, and GTK shells remain until those rows are accepted by the
project owner.

## Verification ledger

| Layer | Current evidence | Not proved by it |
| --- | --- | --- |
| Source | `cargo fmt --check`, Rust check/tests, frontend build, Storybook build, `git diff --check` | Native OS behavior, provider state, signed artifact trust |
| Tauri debug | Embedded/attached daemon paths, sanitized event bridge, named commands, shutdown path | Packaged app registration, OS permissions, updater feed |
| macOS source adapter | Universal Darwin Tauri bundle build/startup smoke, Objective-C bridge linkage, release Info.plist, and packaging script | Developer ID signature, notarization, launch-item approval, notification action smoke |
| Provider/state | Existing daemon contracts remain authoritative | Live enrollment, network, database, update feed, settlement, migration rollback |

Required release-only command:

```bash
./scripts/package-macos-release.sh
```

It requires Darwin, a Developer ID Application identity, App Store Connect
notary API-key values, and the Tauri 2 CLI (the script falls back to
`pnpm dlx`). Until it completes, packaging/signing remains unverified.

## Explicit non-goals for this slice

- No PostgreSQL or Docker is needed to launch the local Tauri app. The embedded
  contributor daemon and account-free Insights use local state.
- No generic frontend-to-daemon write bridge. New mutations require a named
  Tauri command with Rust validation.
- No new Tauri plugins or runtime dependencies were added. Native picker uses
  fixed OS picker commands behind Rust validation; unavailable platform tools
  return a fixed error.
- No claim of signed-package, notification, login-item, keychain, updater,
  provider-enrollment, or cross-OS runtime parity follows from local builds.
  Source wiring for selected macOS adapters does not satisfy those runtime
  claims.
- No removal, overwrite, reset, clean, force-push, or replacement of existing
  native shells.
- No Biome gate in this buildout. TypeScript compilation, Vite build,
  Storybook build, Rust checks, and meaningful runtime smoke remain required.

## Current acceptance commands

From repository root:

```bash
pnpm --dir prototypes/tauri-desktop/frontend install
pnpm --dir prototypes/tauri-desktop/frontend build
pnpm --dir prototypes/tauri-desktop/frontend build-storybook
cargo fmt --manifest-path prototypes/tauri-desktop/src-tauri/Cargo.toml -- --check
cargo test --manifest-path prototypes/tauri-desktop/src-tauri/Cargo.toml
cargo run --manifest-path prototypes/tauri-desktop/src-tauri/Cargo.toml
```

The last command is the local desktop start command. It uses Tauri's per-user
application-data directory. It does not create `macos/.build/TraceCommons.app`.

The current feature-parity proof covers source contracts plus local TypeScript,
Storybook, Rust, and command-boundary checks. It does not prove live commons or
NEAR AI enrollment, wallet completion, witness availability, provider delivery,
or signed OS packaging. Those require the Phase 5 evidence rows above.
