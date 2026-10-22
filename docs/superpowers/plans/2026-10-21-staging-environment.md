# Internal Staging Environment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A staging macOS app, installed beside the production one, that signs in to NEAR AI staging, contributes to a staging server on its own VM, makes attested Private AI calls against measurement pins signed by our staging key, and receives NEAR AI staging payouts, with nothing shared with production.

**Architecture:** The `near-ai-staging` cargo feature (payout design D9) is the Rust channel; one `channel.rs` module holds every compiled-in host and identity value for both channels. `TC_CHANNEL=staging` drives the macOS bundle from `macos/channels/staging.env`. A signed pin-set format and verifier live in the permissive attestation crate behind the feature; an AGPL operator tool observes and signs. The staging server reuses the pilot's deploy files through a few parameters plus a `deploy/staging-gcp/` directory. A manual-dispatch workflow builds and publishes the staging DMG. The release check is widened by exact lines to allow exactly the staging files.

**Tech Stack:** Rust (contributor, platform, attestation, server, ingest), C ABI (`trace-commons-contributor-ffi`), Swift (`macos/`), bash (build, deploy and CI scripts), GitHub Actions, Caddy, systemd.

**Spec:** `docs/superpowers/specs/2026-10-21-staging-environment-design.md`. Owner decisions S1-S4 and D9 are settled. Section 10's defaults are the current answers; build them so an override is a one-place change. Every implementer reads the spec sections their task cites.

## Dependencies outside this plan

- The payout stack: #1534 (rail contract), #1538 (schema), #1541 (accrual), #1542 (NEAR AI client, `near_ai_api.rs`, the contributor's `near-ai-staging` feature and `scripts/ci/check-release-staging-feature.sh`). Tasks 1, 2 and 7 base on #1542's branch `payout-rail-near-ai-client` until it merges, then rebase on `main`.
- Payout plan Task 7a (`payout-rail-staging-build`): switches the daemon's NEAR AI hosts and wires `near_ai_staging()` into the daemon status. Task 2 here extends it into the channel module. If Task 7a has not started when Task 2 does, Task 2 does Task 7a's host switch itself and Task 7a is closed as included; agree which with the owner first.
- NEAR AI facts still open (spec section 10 item 6): per-model completions hosts on staging, staging's measurements and per-model signers, API-key survival across a redeploy. Nothing in this plan blocks on them; Task 6's `observe` is how the measurements get collected.
- Owner actions (spec section 10): the staging host name and DNS (item 1), the Apple App ID and provisioning profile (item 4), Secret Manager secrets and the `staging` GitHub environment, the staging NEAR AI keys. Tasks 9-11 can merge before these exist; they are needed before the first staging run.

## Global Constraints

Repo and process rules (every task):

- Each task is one PR on its own branch. Create each worktree with `git -C $CODE/trace-commons-server worktree add -b <branch> $CODE/tc-wt-<branch> <base>`. Never work in, or read the working tree of, the shared checkout `$CODE/trace-commons-server`. Never `git stash`; never reset or check out away uncommitted work. In the commands below `CODE` is the directory that holds the shared checkout and `W` is the task's worktree.
- `gh` always takes `--repo TraceCommons/trace-commons`. Open each PR as a draft; never merge, never mark ready. PR body from `.github/pull_request_template.md`; tick only the boxes you can vouch for; last line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- TDD: write the listed tests first, run them, see them fail for the stated reason, then implement.
- Commits: short imperative subject, no prefix, no emoji, last line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Run `cargo fmt --all` before each commit.
- Verification uses the CI forms, and the report pastes the real `test result:` lines. Never claim green from truncated or filtered output. Every Rust task runs, at minimum:
  ```bash
  cargo fmt --all -- --check
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-ingest --bins
  cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
  RUSTFLAGS="-D warnings" cargo test --workspace --no-run
  ```
  plus the task's targeted tests and the full suite of each touched crate, **once with default features and once with `--features near-ai-staging`** (a module behind a feature does not compile in a plain `cargo test -p`; see `CLAUDE.md`, "Known gotchas"). For a touched permissive crate also run `RUSTFLAGS="-D warnings" cargo check -p <crate> --no-default-features`.
- Every task that touches a build, packaging or deploy file, or a manifest, runs `scripts/ci/check-release-staging-feature.sh check` and `self-test` and pastes both.
- If `Cargo.toml` changes (a feature, never a dependency), run all four `cargo deny` licence checks from `CLAUDE.md`.
- No new dependency. Ed25519 verification uses `ring` 0.17, already in the attestation and contributor crates. If a task seems to need a new crate, stop and report.
- Licence split: the server, gate-api, gate-enclave and ingest crates are AGPL; every new `.rs` file in them starts with `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`. The attestation, protocol, contributor, platform and ffi crates are permissive and never gain an AGPL dependency. `crates/trace-commons-server/tests/license_boundary.rs` stays unchanged.
- Public repo: synthetic values only. Never a real key, org id, measurement from a live staging report, host IP, secret value, personal path or the owner's data in code, fixtures, commits or PR text. Test keys are generated in the test. The staging pin public key enters the repository only in Task 5, as the one real value, and only once the owner has generated the key pair.
- Hash-only logging (`docs/operator/hash-only-logging.md`): labels and `sha256:` hashes only.
- Fail closed: every pin failure pins nothing; every host mismatch refuses to start. Never fall back to the production path, the unsigned document or "verify nothing".
- User-facing words follow `docs/ui-copy-guide.md`; new strings go in the Rust copy tables and are doc-commented `DRAFT, NEEDS APPROVAL`. No new notice (a staging label in the windows) without the owner's approval of both its existence and wording.
- The C ABI header exists twice (`crates/trace-commons-contributor-ffi/include/trace_commons.h`, `macos/Sources/CTraceCommons/include/trace_commons.h`); edit both and `diff` them to identical. Before any `swift test`, run `cargo build -p trace-commons-contributor-ffi`.
- macOS only (S2). No GTK, Windows or Tauri change beyond what a shared crate forces to compile.

## PR Stack

| # | Branch | Base | Scope |
|---|---|---|---|
| 1 | `staging-release-check` | `payout-rail-near-ai-client` | widen the release check for the staging files and manifest lines; self-test cases |
| 2 | `staging-channel-module` | `payout-rail-staging-build` (or `staging-release-check`, see Dependencies) | `channel.rs` with every host and identity value; ratchet test; platform crate takes the access group as a parameter |
| 3 | `staging-state-dir-channel` | `staging-channel-module` | state directory `channel` file; staging refuses a production directory |
| 4 | `staging-signed-pins-format` | `staging-release-check` | signed pin-set format and verifier in the attestation crate, behind its feature |
| 5 | `staging-pin-source` | `staging-signed-pins-format` + `staging-channel-module` | daemon `PinProvider` reads only the signed set in a staging build; sequence file; status labels |
| 6 | `staging-pins-tool` | `staging-signed-pins-format` | `trace-commons-staging-pins` observe / sign / verify |
| 7 | `staging-server-guard` | `staging-release-check` | ingest boot guard for NEAR AI host coherence; `/health` `near_ai_staging` |
| 8 | `staging-ffi-channel` | `staging-state-dir-channel` | `tc_build_channel_json()`; Swift state directory, passkey RP and log subsystem from it; dylib/bundle agreement |
| 9 | `staging-bundle-channel` | `staging-ffi-channel` | `macos/channels/*.env`, staging entitlements, build scripts read the channel; URL scheme per channel |
| 10 | `staging-macos-workflow` | `staging-bundle-channel` | `staging-macos.yml`; staging appcast; `assert-no-staging-markers.sh` in the release workflows |
| 11 | `staging-deploy` | `staging-server-guard` | `deploy.sh` parameters; `deploy/staging-gcp/`; Caddy site block; `render-env.sh` |
| 12 | `staging-runbook` | `staging-deploy` | `docs/operator/staging-environment.md`, README index, tester guide |

Tasks 4, 6 and 7 touch no client code and can be reviewed in parallel with 2-3. Tasks 1, 2 and 7 move to `main` as soon as #1542 merges.

## Review Focus

1. **No production path ever reaches staging, or the reverse.** Tests: `a_staging_build_names_no_production_host` and `a_default_build_names_no_staging_host` (task 2), `a_staging_ingest_refuses_a_production_near_ai_host` (task 7), `a_staging_store_refuses_a_production_directory` (task 3).
2. **Pins fail closed on every path.** Bad signature, unknown key, wrong channel or gateway host, expired, future-issued, rollback, same sequence with other bytes, one bad set: each pins nothing. Tests in tasks 4 and 5.
3. **Production carries nothing of the staging verifier.** `a_default_build_has_no_staging_pin_key` (task 5) and the marker scan (task 10).
4. **The release check still fails when it should.** Every new allowance has a self-test case proving one more step past it is refused (task 1).

---

### Task 1: Widen the release check for staging (`staging-release-check`, base `payout-rail-near-ai-client`)

Spec: section 6.2.

**Files:**
- Modify: `scripts/ci/check-release-staging-feature.sh` (add `STAGING_FILES`, the trigger check, the new exact manifest lines, `crates/trace-commons-attestation/Cargo.toml` and `crates/trace-commons-protocol/Cargo.toml` in `SHIPPED_MANIFESTS`, and self-test cases)
- Create (placeholders that later tasks fill, so the check has real files to scan): `.github/workflows/staging-macos.yml` with `on: workflow_dispatch` and a single job that echoes and exits non-zero ("not built yet"); `deploy/staging-gcp/cloudbuild.yaml` and `macos/channels/staging.env` with the feature named and nothing else

- [ ] **Step 1: Write the failing self-test cases** (each seeds one violation into the scratch copy and must be refused):
  - a staging file that no longer names the feature
  - a staging file that names a production destination (the production appcast URL, the production update bucket, the pilot artifact bucket, the Homebrew tap)
  - `push:` added to `staging-macos.yml`
  - `workflow_call:` added to `staging-macos.yml`
  - a `release-*.yml` that references `staging-macos.yml` or `macos/channels/staging.env`
  - the contributor's forward widened by one more crate
  - the attestation crate's definition forwarding anywhere
  - `required-features = ["near-ai-staging"]` on a bin other than `trace-commons-staging-pins`
- [ ] **Step 2: Run** `scripts/ci/check-release-staging-feature.sh self-test`; expect the new cases to report "was NOT refused".
- [ ] **Step 3: Implement.** Exact-line allowances only; no patterns that admit a family of lines.
- [ ] **Step 4: Verify** — `check` passes on the tree, `self-test` prints "refused as expected" for every case (paste both); `actionlint` if available on the placeholder workflow.
- [ ] **Step 5: Commit** — `Let the release check allow exactly the staging build files`.

**Done when:** the check passes, every case is refused, and no release workflow, deploy file or shipped manifest can enable the feature except by the exact lines listed in the spec.

---

### Task 2: One channel module (`staging-channel-module`)

Spec: sections 4.1, 4.2; Verified, first two tables.

**Files:**
- Create: `crates/trace-commons-contributor/src/channel.rs` (for both channels: `CHANNEL`, `STATE_DIR_NAME`, `STATE_DIR_ENV`, `KEYCHAIN_ACCESS_GROUP`, keychain service prefix, `NATIVE_ACCOUNT_ORIGIN`, `PASSKEY_RP_ID`, `PASSKEY_ORIGIN`, `URL_SCHEME`, NEAR AI API origin, inference base, dashboard origin, attestation gateway; the production deny list of staging hosts; staging values use a `STAGING_HOST` constant whose value is the spec's open item 1, kept in this one place)
- Modify: every site in the spec's tables — `daemon/nearai_credential/api.rs:31`, `managed/route.rs:36`, `daemon/nearai_credential/funding.rs:24`, `external_url.rs:57`, `envelope.rs:113`, `routing/proof_attestor.rs:98`, `skill_loop/evaluation/transport.rs:19`, `daemon/native_identity.rs:86,229,377`, `config.rs:900` and the env variable name — to read `channel.rs`; pass the NEAR AI base to IronWire explicitly everywhere so `NEARAI_DEFAULT_BASE_URL` is never used
- Modify: `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs:19` callers in the contributor pass the channel's base; the protocol default stays production (the server sets its own, task 7)
- Modify: `crates/trace-commons-contributor-platform/src/os_secret_store.rs` (`ACCESS_GROUP` and service names become constructor parameters; the entitlements parity test at `:517-547` moves to the contributor and gains a staging twin against `macos/entitlements-staging.plist`, created here as a copy with the staging group and associated domain)
- Tests: in `channel.rs` and a new `crates/trace-commons-contributor/tests/channel_hosts.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_staging_build_names_no_production_host()   // cfg(feature): scans src/ of contributor, ffi, platform for host literals outside channel.rs and #[cfg(test)]
#[test] fn a_default_build_names_no_staging_host()      // cfg(not(feature)): same scan, and channel.rs's default values
#[test] fn every_near_ai_host_in_a_build_is_one_host()  // API origin, inference base, gateway share a host per channel
#[test] fn a_production_build_refuses_a_staging_ingest_origin()
#[test] fn the_staging_entitlements_grant_the_staging_access_group()
#[test] fn ironwire_is_never_built_on_its_default_base()
```

- [ ] **Step 2: Run** with and without `--features near-ai-staging`; expect the scans to list today's literals.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify** — global checks; `cargo test -p trace-commons-contributor` and `-p trace-commons-contributor-platform`, default and `--features near-ai-staging`, against a baseline taken before Step 1; the release check and self-test.
- [ ] **Step 5: Commit** — `Hold every compiled-in host and identity value in one channel module`.

**Done when:** no host or identity literal remains outside `channel.rs`, and the scans fail on a reintroduced one.

---

### Task 3: A state directory belongs to one channel (`staging-state-dir-channel`)

Spec: section 4.4.

**Files:**
- Modify: `crates/trace-commons-contributor/src/config.rs` (`ConfigStore::open` writes `channel` into a new directory; refuses the other channel's; a directory with `contributor.json` and no `channel` file is production)
- Tests: `config.rs` tests

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_new_directory_records_its_channel()
#[test] fn a_staging_store_refuses_a_production_directory()       // cfg(feature)
#[test] fn a_staging_store_refuses_an_unmarked_enrolled_directory() // cfg(feature)
#[test] fn a_production_store_refuses_a_staging_directory()       // cfg(not(feature))
#[test] fn a_production_store_adopts_an_unmarked_directory()      // existing installs keep working
```

- [ ] **Steps 2-4** as in task 2, plus the Windows `adopt_state_dir` tests still pass.
- [ ] **Step 5: Commit** — `Mark each state directory with its build channel`.

---

### Task 4: Signed pin-set format and verifier (`staging-signed-pins-format`, base `staging-release-check`)

Spec: sections 5.2, 5.5 (steps 2-3), 5.6, 5.8.

**Files:**
- Create: `crates/trace-commons-attestation/src/staging_pins.rs` (MIT OR Apache-2.0; `SignedPinSet`, `PinPayload`, `verify(document, public_key, now, gateway_host) -> Result<VerifiedPins, PinRefusal>` with one `PinRefusal` label per row of the spec's failure table; the domain-separation prefix as a constant)
- Modify: `crates/trace-commons-attestation/Cargo.toml` (`near-ai-staging = []`), `src/lib.rs` (`#[cfg(feature = "near-ai-staging")] pub mod staging_pins;`)
- Tests: in the module, with a key pair generated in the test by `ring`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_well_signed_current_set_verifies()
#[test] fn a_signature_by_another_key_is_refused()
#[test] fn an_unknown_key_id_is_refused()
#[test] fn the_signature_covers_the_domain_prefix()   // a signature over the bare payload is refused
#[test] fn a_wrong_channel_format_or_gateway_host_is_refused()
#[test] fn an_expired_or_future_set_is_refused()       // 5-minute skew allowance at both edges
#[test] fn a_lifetime_over_fourteen_days_is_refused()
#[test] fn one_unparseable_set_refuses_the_document()
#[test] fn an_empty_set_list_is_refused()
```

- [ ] **Step 2: Run** `cargo test -p trace-commons-attestation --features near-ai-staging`; expect compile failure, then failures.
- [ ] **Step 3: Implement.** Sequence checking is not here (it needs state); task 5 does it.
- [ ] **Step 4: Verify** — `cargo test -p trace-commons-attestation` default (count unchanged from baseline) and `--features near-ai-staging` (new tests run); `--no-default-features` check; the four `cargo deny` runs; the release check.
- [ ] **Step 5: Commit** — `Add a signed NEAR AI pin-set format for staging builds`.

---

### Task 5: The daemon reads only signed pins in a staging build (`staging-pin-source`)

Spec: sections 5.1, 5.3, 5.5, 5.6, 5.7.

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml` (the feature forwards to `trace-commons-attestation/near-ai-staging`, the exact line task 1 allows)
- Modify: `crates/trace-commons-contributor/src/channel.rs` (staging only: `STAGING_PIN_URL`, `STAGING_PIN_PUBLIC_KEY` as 32 bytes; the real key once the owner has generated it, until then a value no private key matches, so every document is refused)
- Modify: `crates/trace-commons-contributor/src/routing/proof_attestor.rs` (`PinProvider`: under the feature, the signed document is the only source; the ingest document and the env override are not compiled; the sequence file `near-ai-pins-sequence` in the state directory; refresh timing unchanged)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (status reports the pin refusal label)
- Tests: `proof_attestor.rs` tests with a mock pin server

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_staging_build_pins_a_signed_set()
#[test] fn a_staging_build_ignores_the_ingest_document()
#[test] fn a_staging_build_ignores_the_measurement_env_override()
#[test] fn a_lower_sequence_is_refused_after_a_higher_one()
#[test] fn the_same_sequence_with_other_bytes_is_refused()
#[test] fn a_failed_fetch_keeps_the_last_set_only_until_pin_ttl_and_expiry()
#[test] fn every_refusal_earns_no_key()             // no Attestation::Keys under any refusal
#[test] fn status_names_the_pin_refusal()
#[test] fn a_default_build_has_no_staging_pin_key() // cfg(not(feature)): channel exposes no staging key or URL
#[test] fn a_default_build_still_uses_the_ingest_document()
```

- [ ] **Steps 2-4** as in task 2; also `cargo test -p trace-commons-contributor --features near-ai-staging routing::proof_attestor`.
- [ ] **Step 5: Commit** — `Take NEAR AI pins only from the signed staging set in a staging build`.

---

### Task 6: The re-sign tool (`staging-pins-tool`, base `staging-signed-pins-format`)

Spec: sections 5.4, 5.9.

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace-commons-staging-pins.rs` (AGPL header; `observe`, `sign`, `verify`; keys from stdin only; `sign` refuses a sequence not above the served file's and a TTL above 14 days; `sign` prints the `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=` line for the first set)
- Modify: `crates/trace-commons-server/Cargo.toml` (`[[bin]]` with `required-features = ["near-ai-staging"]`; the server's feature forwards to the attestation crate; both exact lines task 1 allows)
- Tests: in the bin, plus `crates/trace-commons-server/tests/staging_pins_tool.rs` against a mock report server and a fixture quote that fails verification

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn observe_refuses_a_set_from_a_quote_that_does_not_verify()
#[test] fn observe_refuses_a_report_not_bound_to_its_nonce()
#[test] fn sign_then_verify_round_trips()
#[test] fn sign_refuses_a_sequence_not_above_the_served_one()
#[test] fn sign_refuses_a_ttl_over_fourteen_days()
#[test] fn sign_never_reads_a_key_from_argv()
#[test] fn nothing_logs_a_key_or_a_measurement()
```

- [ ] **Steps 2-4** as in task 2, with `-p trace-commons-server --features near-ai-staging --bin trace-commons-staging-pins`.
- [ ] **Step 5: Commit** — `Add an operator tool to observe and sign staging NEAR AI pins`.

---

### Task 7: Server boot guard and readiness (`staging-server-guard`, base `staging-release-check`)

Spec: sections 3.9, 3.11.

**Files:**
- Modify: `crates/trace-commons-server/src/near_ai_api.rs` (`near_ai_host_coherence(settings) -> Result<(), &'static str>`: under the feature every NEAR AI host is the staging host; without it none is)
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (call it at boot with the scorer, attestation and privacy-filter bases, configured or defaulted; refuse with `missing_control:near_ai_staging_host_coherence` naming the setting; `health_handler` reports `near_ai_staging`)
- Tests: `near_ai_api.rs` and ingest `tests.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_staging_ingest_refuses_a_production_near_ai_host()        // cfg(feature), each of the three settings
#[test] fn a_staging_ingest_refuses_an_unset_attestation_base()        // the default is production
#[test] fn a_default_ingest_refuses_a_staging_near_ai_host()
#[test] fn the_refusal_names_the_setting_never_the_value()
#[tokio::test] async fn health_reports_near_ai_staging()
```

- [ ] **Steps 2-4** as in task 2 for the server and ingest; the ingest bin tests run whole against PostgreSQL per `CLAUDE.md`.
- [ ] **Step 5: Commit** — `Refuse to boot a staging ingest that names a production NEAR AI host`.

---

### Task 8: The app reads its channel from the dylib (`staging-ffi-channel`)

Spec: sections 4.1 (last paragraph), 4.3.

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` (`tc_build_channel_json()`: channel, state directory name, state directory env name, passkey RP ID, URL scheme; no secret, no key)
- Modify: both `trace_commons.h` copies
- Modify: `macos/Sources/TCShellCore/StateDirectory.swift` (default directory and env name from the FFI), `macos/Sources/TCShellCore/NativePasskey.swift` (RP ID from the FFI), `macos/Sources/TraceCommonsApp/WindowBreadcrumb.swift` (subsystem from the bundle id), app startup (compare Info.plist `TCChannel` with the dylib's; refuse to start the daemon on a mismatch)
- Tests: FFI tests; `macos/Tests/TCShellCoreTests/StateDirectoryTests.swift`, `NativePasskeyTests.swift`, a new `ChannelAgreementTests.swift`

- [ ] **Step 1: Write the failing tests** — Rust: `the_channel_json_matches_the_channel_module`, `the_channel_json_carries_no_key`. Swift: `testDefaultDirectoryComesFromTheDylib`, `testPasskeyRpComesFromTheDylib`, `testAChannelMismatchRefusesToStart`, `testAMissingPlistChannelRefusesToStart`.
- [ ] **Step 2: Run** — `cargo build -p trace-commons-contributor-ffi && swift test --package-path macos --filter 'StateDirectoryTests|NativePasskeyTests|ChannelAgreementTests'`.
- [ ] **Steps 3-4** — full `swift test --package-path macos` against baseline after rebuilding the dylib; headers `diff` identical.
- [ ] **Step 5: Commit** — `Read the build channel from the dylib in the macOS app`.

---

### Task 9: Bundle channel files (`staging-bundle-channel`)

Spec: sections 4.1, 4.2, 4.5, 4.7.

**Files:**
- Create: `macos/channels/production.env` (today's values, so a production build is byte-identical in its Info.plist), fill `macos/channels/staging.env` (bundle id, display name, icon name, entitlements file, profile path variable, feed URL, URL scheme, the opaque cargo-features value)
- Modify: `macos/scripts/make-app-bundle.sh`, `info-plist.sh`, `make-release-dmg.sh` (read `TC_CHANNEL`, default `production`; pass the features value through; check the bundled `near-ai --version` channel against the plist)
- Modify: `crates/trace-commons-contributor/src/deep_link.rs` (accept only `channel::URL_SCHEME`)
- Modify: `scripts/check-aasa.sh` callers in CI only if a test needs the staging app id (the script already takes it)
- Tests: `scripts/ci/` test for `info-plist.sh` under each channel; `deep_link.rs` tests

- [ ] **Step 1: Write the failing tests** — `info-plist.sh` with `TC_CHANNEL=production` produces today's plist exactly (diff against a fixture taken before the change); with `staging` it has the staging bundle id, name, scheme and no production feed; `deep_link`: `a_staging_build_refuses_the_production_scheme`, `a_default_build_refuses_the_staging_scheme`.
- [ ] **Steps 2-4** — the release check and self-test (the build scripts must still not name the feature); a local `TC_CHANNEL=staging macos/scripts/make-app-bundle.sh` ad-hoc build that launches and reports `channel: staging` (describe the run; no screen captures).
- [ ] **Step 5: Commit** — `Build the macOS bundle for a channel from one file`.

---

### Task 10: The staging workflow (`staging-macos-workflow`)

Spec: sections 6.1, 5.7.

**Files:**
- Modify: `.github/workflows/staging-macos.yml` (the real job: `workflow_dispatch` only, `environment: staging`, build with `TC_CHANNEL=staging`, `swift test`, sign, notarize, staple, channel agreement, staging appcast, upload to the staging bucket, GitHub pre-release `staging-<date>-<sha>`; `actions/checkout@v6`, `actions/cache@v5`, ends with `./.github/actions/trim-cargo-cache` if it caches `target`)
- Modify: `scripts/updates/generate-appcast.sh` (output path and key secret name as arguments; production defaults unchanged)
- Create: `scripts/ci/assert-no-staging-markers.sh` (fails if a binary contains the staging pin key bytes or a staging host) and its self-test
- Modify: `.github/workflows/release-apps.yml` (run the marker scan on the dylib and CLI after the macOS build)
- Tests: the marker script's self-test (a seeded binary with the key bytes is refused)

- [ ] **Step 1: Write the failing self-test**, run it.
- [ ] **Step 2-3: Implement.**
- [ ] **Step 4: Verify** — release check and self-test; marker self-test; `actionlint`; one dispatch of the workflow on the PR branch by the owner once the `staging` environment exists (not a precondition for opening the PR).
- [ ] **Step 5: Commit** — `Add a manual workflow that publishes the staging macOS app`.

---

### Task 11: Deploy the staging server (`staging-deploy`, base `staging-server-guard`)

Spec: sections 3.1-3.8, 3.10, 3.12.

**Files:**
- Modify: `deploy/pilot-gcp/deploy.sh` (`TC_DB_MODE`, `TC_DEPLOY_LABEL`, `TC_EXPECT_STAGING` with pilot defaults; the `/health` channel check)
- Modify: `deploy/pilot-gcp/Caddyfile.template` (the apex site block behind a variable that renders to nothing for the pilot)
- Create: `deploy/staging-gcp/cloudbuild.yaml` (real build; staging bucket only), `staging.env.template`, `postgresql.conf.d`/drop-in for the ingest unit, `apple-app-site-association`, `render-env.sh`, `README.md`
- Tests: a shell test that renders the pilot templates before and after and diffs them identical; `caddy adapt` on both renders where Caddy is available; the release check

- [ ] **Step 1: Write the failing test** — the pilot render diff (must stay empty) and a staging render that must contain no pilot bucket, key ring or host.
- [ ] **Steps 2-4.** No `gcloud` command runs in this task.
- [ ] **Step 5: Commit** — `Parameterise the pilot deploy for a staging server`.

---

### Task 12: Runbook and tester guide (`staging-runbook`)

Spec: sections 3, 5.4, 7.

**Files:**
- Create: `docs/operator/staging-environment.md` (provisioning order, Secret Manager names and IAM, DNS, first deploy, the pin re-sign procedure, the payout steps by reference to the payout design section 2, rollback, what to check after a NEAR AI staging redeploy)
- Create: `docs/staging-testers.md` (the spec's section 7, in plain words, following `docs/ui-copy-guide.md`)
- Modify: `docs/operator/README.md` (index), `README.md` if its operator checklist lists environments

- [ ] **Step 1:** Walk the runbook against the merged tasks; every command names a real script or flag (grep for each).
- [ ] **Step 2: Commit** — `Add the staging environment runbook and tester guide`.

**Done when:** an operator can bring up the VM, deploy, sign pins and publish a build from the runbook alone, and a tester can install and sign in from the guide alone.
