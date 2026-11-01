# Internal Staging Environment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A staging macOS app, installed beside the production one, that signs in to NEAR AI staging, contributes to a staging server on its own VM, makes attested Private AI calls against measurement pins signed by our staging key, and receives NEAR AI staging payouts, with nothing shared with production.

**Architecture:** The `near-ai-staging` cargo feature (payout design D9) is the Rust channel; one `channel.rs` module holds every compiled-in host and identity value for both channels. `TC_CHANNEL=staging` drives the macOS bundle from `macos/channels/staging.env`. A signed pin-set format and verifier live in the permissive attestation crate behind the feature; an AGPL operator tool observes and signs. The staging server reuses the pilot's deploy files through a few parameters plus a `deploy/staging-gcp/` directory. A manual-dispatch workflow builds and publishes the staging DMG. The release check is widened by exact lines to allow exactly the staging files.

**Tech Stack:** Rust (contributor, platform, attestation, server, ingest), C ABI (`trace-commons-contributor-ffi`), Swift (`macos/`), bash (build, deploy and CI scripts), GitHub Actions, Caddy, systemd.

**Spec:** `docs/superpowers/specs/2026-10-21-staging-environment-design.md`. Owner decisions S1-S4 and D9 are settled. Section 10's defaults are the current answers; build them so an override is a one-place change. Every implementer reads the spec sections their task cites.

## Dependencies outside this plan

- The payout stack: #1534 (rail contract), #1538 (schema), #1541 (accrual), #1542 (NEAR AI client, `near_ai_api.rs`, the contributor's `near-ai-staging` feature and `scripts/ci/check-release-staging-feature.sh`). Tasks 1, 2 and 7 base on #1542's branch `payout-rail-near-ai-client` until it merges, then rebase on `main`.
- Payout plan Task 7a (`payout-rail-staging-build`): switches the daemon's NEAR AI hosts and wires `near_ai_staging()` into the daemon status. Task 2 here extends it into the channel module. If Task 7a has not started when Task 2 does, Task 2 does Task 7a's host switch itself and Task 7a is closed as included; agree which with the owner first.
- NEAR AI facts (spec section 10 item 3). The register set is owner decision S16 (2026-10-22): pin `mrtd`, `rtmr0-2` and `mrconfigid`, and replay RTMR3 from the event log against a signed allowlist instead of pinning it; that set ships only together with the replay (Task 6b). The staging gateway serves attestation and is the configured base; whether staging has per-model completions hosts is unconfirmed. Which log replays the model quote's RTMR3 was settled on 2026-10-22 (Task 6b step 1): the model entry's own `event_log`, with digests recomputed; post-boot compose actions do not extend RTMR3 and are checked from the compose-manager record. Still open: per-model hosts, the compose-manager record's freshness (spec 5.2), a probe across a staging redeploy recording per register whether it changed, and API-key survival across a redeploy. Staging's measurements may change when it redeploys and are not recorded in the repo; Task 6's `observe` is how they get collected.
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
| 3 | `staging-state-dir-channel` | `staging-channel-module` | an override naming the other channel's default state directory is refused (S11; replaced the `channel` file) |
| 4 | `staging-signed-pins-format` | `staging-release-check` | signed pin-set format and verifier in the attestation crate, behind its feature |
| 5 | `staging-pin-source` | `staging-signed-pins-format` + `staging-channel-module` | daemon `PinProvider` reads only the signed set in a staging build; sequence file; status labels |
| 6 | `staging-pins-tool` | `staging-signed-pins-format` | `trace-commons-staging-pins` observe / sign / verify |
| 6b | `staging-rtmr3-replay` | `staging-pins-tool` (+ `staging-pin-source`) | RTMR3 replay and signed allowlist in the tool and the app; payload v2; the S16 register set becomes the default (S16) |
| 7 | `staging-server-guard` | `staging-release-check` | ingest boot guard for NEAR AI host coherence; `/health` `near_ai_staging` |
| 8 | `staging-ffi-channel` | `staging-state-dir-channel` | `tc_build_channel_json()`; Swift state directory, passkey RP and log subsystem from it; dylib/bundle agreement |
| 9 | `staging-bundle-channel` | `staging-ffi-channel` | `macos/channels/*.env`, staging entitlements, build scripts read the channel; URL scheme per channel |
| 10 | `staging-macos-workflow` | `staging-bundle-channel` | `staging-macos.yml`; staging appcast; `assert-no-staging-markers.sh` in the release workflows |
| 11 | `staging-deploy` | `staging-server-guard` | `deploy.sh` parameters; `deploy/staging-gcp/`; Caddy site block; `render-env.sh` |
| 12 | `staging-runbook` | `staging-deploy` | `docs/operator/staging-environment.md`, README index, tester guide |

Tasks 4, 6 and 7 touch no client code and can be reviewed in parallel with 2-3. Tasks 1, 2 and 7 move to `main` as soon as #1542 merges.

## Review Focus

1. **No production path ever reaches staging, or the reverse.** Tests: `a_staging_build_names_no_production_host` and `a_default_build_names_no_staging_host` (task 2), `a_staging_ingest_refuses_a_production_near_ai_host` (task 7), `an_override_naming_the_other_channel_default_is_refused` (config.rs) and `an_override_to_the_other_channel_directory_is_refused_before_it_is_opened` (`channel_hosts`), plus `tc_resolve_state_dir`'s `an_override_naming_the_other_channel_directory_is_refused_without_a_path` and the macOS app's `testAnOverrideNamingTheOtherChannelDirectoryIsRefused` (task 8).
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
  - a release workflow tag filter that matches the staging tag prefix `staging-`
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
#[test] fn a_default_build_names_no_staging_host()      // cfg(not(feature)): same scan, and channel.rs's default values; the deny list holds only SHA-256 digests of staging hosts
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

Spec: section 4.4. The first version wrote a `channel` file into every state
directory (#1547); owner decision S11 replaced it with the override check
below.

**Files:**
- Modify: `crates/trace-commons-contributor/src/channel.rs` (`OTHER_STATE_DIR_NAME` for both channels)
- Modify: `crates/trace-commons-contributor/src/config.rs` (`ConfigStore::resolve` refuses an override that resolves to the other channel's default state directory or inside it, as `state-dir-other-channel`)
- Tests: `config.rs` tests, `tests/channel_hosts.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn an_override_naming_the_other_channel_default_is_refused()
#[test] fn an_override_inside_the_other_channel_default_is_refused()
#[test] fn an_override_under_a_missing_child_of_the_other_channel_default_is_refused()
#[test] fn an_override_symlinked_to_the_other_channel_default_is_refused() // cfg(unix)
#[test] fn a_sibling_sharing_the_other_channel_name_prefix_is_allowed()
#[test] fn the_other_channel_default_dirs_carry_the_other_channel_name()
#[test] fn an_override_to_the_other_channel_directory_is_refused_before_it_is_opened() // channel_hosts.rs, cfg(unix)
```

- [ ] **Steps 2-4** as in task 2, in both builds.
- [ ] **Step 5: Commit** — `Replace the state directory channel marker with an override check`.

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
- Modify: `crates/trace-commons-contributor/src/channel.rs` (staging only: `STAGING_PIN_URL`, `STAGING_PIN_PUBLIC_KEY`; the real 32-byte key once the owner has generated it, until then a deliberately malformed 31-byte value that `ring` rejects outright, so every document is refused with an error)
- Modify: `crates/trace-commons-contributor/src/routing/proof_attestor.rs` (`PinProvider`: under the feature, the signed document is the only source; the ingest document and the env override are not compiled; the sequence file `near-ai-pins-sequence` in the state directory; refresh timing unchanged)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (status reports the pin refusal label)
- Tests: `proof_attestor.rs` tests with a mock pin server
- Modify: `scripts/ci/staging-pin-public-keys.txt` (the staging public key, in the same commit that compiles it in) and `scripts/ci/assert-no-staging-markers.sh` (extend `sources_agree` so the pin URL constant must contain `URL_MARKER` and the compiled-in key must be listed in the keys file, as it already does for the format constants)
- Modify: `.github/workflows/staging-macos.yml` (an inverted marker scan that requires the staging bundle's dylib and CLI to contain the format string and the key: the positive control for the release scan, which until then passes on an absent signal)

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
#[test] fn the_pin_gateway_host_is_the_signed_documents_host() // staging: host(NEAR_AI_ATTESTATION_GATEWAY) == "cloud-stg-api.near.ai"
```

Acceptance: the host the app compares a document's `gateway_host` with is
the bare host of `channel::NEAR_AI_ATTESTATION_GATEWAY`, never the full URL,
and a staging-build test pins it to the same literal the re-sign tool's
`the_payload_names_the_staging_gateway_host` test pins
(`cloud-stg-api.near.ai`). The tool writes the host of the server's
compiled-in staging base; nothing else checks that the two agree.

- [ ] **Steps 2-4** as in task 2; also `cargo test -p trace-commons-contributor --features near-ai-staging routing::proof_attestor`.
- [ ] **Step 5: Commit** — `Take NEAR AI pins only from the signed staging set in a staging build`.

As merged (#1564), the app checks the signed sets' registers only. It does not replay RTMR3; that, and refusing a v1 document once the replay is in, is Task 6b (S16). Until Task 6b lands, a staging pin set must keep production's register form (`mrtd`, `rtmr0-3`).

---

### Task 6: The re-sign tool (`staging-pins-tool`, base `staging-signed-pins-format`)

Spec: sections 5.4, 5.9.

Requirements from the 2026-10-22 probe and owner decision S16 (spec 5.2, 5.4):

- The register set is S16: `mrtd`, `rtmr0`, `rtmr1`, `rtmr2` and `mrconfigid`, with RTMR3 replayed and checked against an allowlist. In this task (#1563), `DEFAULT_REGISTERS` stays production's `mrtd` and `rtmr0-3`. It changes to the S16 set only in the same PR that lands the replay (Task 6b), never before: `mrtd`, `rtmr0-2` and `mrconfigid` without the replay must never ship, because with staging's v1 MRCONFIGID nothing in that set covers the key provider or any compose action after boot. Point `DEFAULT_REGISTERS`'s doc comment at S16 instead of "section 10, item 6".
- `observe` emits sets only from `model_attestations[].intel_quote` for the requested model. The gateway quote and the compose-manager quote may be verified but are never emitted as a pin set. #1563's `pin_observe::observe_model` already reads only `model_entry_quotes(&report, model)`; the test below pins that.
- `observe` re-verifies the quote on every run, because measurements may change when NEAR AI staging redeploys, and never copies `info.tcb_info` into a set.
- The live report is the gateway's `model_attestations` shape, not a per-model host's single report; the server-side parsing is separate work (branch `staging-gateway-quotes`).

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
#[test] fn observe_emits_sets_only_from_the_requested_models_entry_quotes() // a report whose gateway and compose-manager quotes verify and carry other registers: none of their registers appears in the output
```

- [ ] **Steps 2-4** as in task 2, with `-p trace-commons-server --features near-ai-staging --bin trace-commons-staging-pins`.
- [ ] **Step 5: Commit** — `Add an operator tool to observe and sign staging NEAR AI pins`.

---

### Task 6b: Replay RTMR3 and check it against an allowlist (`staging-rtmr3-replay`, base `staging-pins-tool`)

Spec: S16, sections 5.2, 5.4, 5.5, 5.6, 5.9.

Owner decision S16: the re-sign tool and the app both replay the RTMR3 event log, require the replay to equal the verified model entry quote's RTMR3, and check app id, key provider and every post-boot compose action against an allowlist carried in the signed payload. Step 1 found that compose actions do not extend RTMR3, so the design (spec 5.2) has two halves: (a) replay the model entry's own `event_log` with recomputed digests, require equality with the verified model quote's RTMR3, and check `app-id`, `compose-hash` (equal to MRCONFIGID's v1 compose hash), `mr-kms`, `os-image-hash` and `key-provider` against the allowlist, `instance-id` any value, and an unknown, extra, missing or out-of-order event refused; (b) verify the entry's `compose_manager_attestation` quote against Intel collateral, require its registers to equal the model quote's and our nonce at `report_data[32..64]`, recompute `actions_hash` (SHA-256 of the actions as sorted-keys compact JSON) and require it at `report_data[0..32]`, and check every action (kind, container, image reference) against the allowlist; an action not allowed refuses. (Superseded by owner decision S17: the actions have a different shape per kind, and the allowlist lists compose-manager images by digest and compose files by path, SHA-256 and commit; spec 5.2.) This task is where `DEFAULT_REGISTERS` becomes the S16 set (`mrtd`, `rtmr0-2`, `mrconfigid`); the register change and the replay land in one PR, in both the tool and the app, or not at all.

**Files:**
- Modify: `crates/trace-commons-attestation/src/` (permissive, behind `near-ai-staging`): an RTMR3 replay over a dstack event log, the event digest recomputed as SHA-384(event_type LE32 || ":" || name || ":" || payload); the compose-manager check (quote registers equal to the model quote's, nonce, recomputed `actions_hash`, actions against the allowlist); the allowlist type and check; payload format `trace-commons-near-ai-pins/v2` with `allowlist` (`rtmr3` and `compose_actions`), its own domain-separation prefix, and v1 refused by a replay-aware verifier
- Modify: `crates/trace-commons-server/src/near_attestation/pin_observe.rs` and `src/bin/trace-commons-staging-pins.rs` (`observe` replays each model entry's log, verifies and checks its compose-manager record, and prints the decoded events and compose actions for review, never the instance-id value; `sign` reads the allowlist file and embeds it; `DEFAULT_REGISTERS` becomes the S16 set)
- Create: `deploy/staging-gcp/near-ai-rtmr3-allowlist.json` (the reviewed allowlist; every change is a PR)
- Modify: `crates/trace-commons-contributor/src/routing/proof_attestor.rs` (after a set matches, replay the entry's log and check its compose-manager record, both from the same report and nonce as the quote, and check the allowlist; a key is earned only if the set, the replay, the RTMR3 allowlist and the compose-manager checks all pass; status labels for each refusal)
- Modify: `docs/operator/staging-pin-signing.md` (the allowlist review step)
- Tests: in the attestation crate, `crates/trace-commons-server/tests/staging_pins_tool.rs`, and `proof_attestor.rs`

- [x] **Step 1: Settle which log replays the model quote's RTMR3.** Done 2026-10-22, from a live capture with a fresh nonce kept out of the repo; recorded in the spec (Verified on 2026-10-22: the RTMR3 replay). Findings: the model entry's own top-level `event_log` (30 events) replays to the model quote's RTMR3, and so do the compose-manager record's copy of it and the gateway's log against the gateway quote. Its ten RTMR3 events, in order: `system-preparing`, `app-id`, `compose-hash`, `instance-id`, `boot-mr-done`, `mr-kms`, `os-image-hash`, `key-provider`, `storage-fs`, `system-ready`, all `event_type` `0x08000001`. NEAR AI ships them with an empty `digest`, so each is recomputed as SHA-384(event_type LE32 || ":" || name || ":" || payload bytes) before the replay. Post-boot compose actions do not extend RTMR3: they are the compose-manager record's `actions` list, whose quote has the model quote's registers, our nonce at `report_data[32..64]`, and at `report_data[0..32]` the SHA-256 of the actions as sorted-keys compact JSON. That capture's compose-manager quote was not itself verified against Intel collateral; this task's code verifies it. Open: whether a compose action can happen between the compose-manager quote and the model's use of its key (spec 5.2); the app fetches both from one report with one nonce.
- [x] **Step 2: Write the failing tests** (every test below exists and runs under `--features near-ai-staging`)

```rust
// attestation crate
#[test] fn replay_matching_the_quote_is_accepted()
#[test] fn replay_mismatch_is_refused()                       // one event altered, dropped or reordered
#[test] fn the_event_digest_is_recomputed_from_type_name_and_payload() // an empty recorded digest replays; known-answer vector
#[test] fn an_event_whose_recorded_digest_differs_from_its_payload_is_refused()
#[test] fn an_unknown_event_is_refused()                      // name not in the allowlist
#[test] fn an_extra_missing_or_out_of_order_event_is_refused()
#[test] fn a_different_mr_kms_or_os_image_hash_is_refused()
#[test] fn the_compose_manager_quote_is_bound_to_the_model_quote_and_nonce() // registers equal, nonce at [32..64]
#[test] fn a_compose_manager_quote_from_another_td_is_refused()
#[test] fn a_compose_manager_quote_that_does_not_verify_is_refused()
#[test] fn actions_hash_is_recomputed_as_sorted_compact_json() // and must sit at report_data[0..32]; default or unsorted serialisation does not match
#[test] fn an_actions_list_not_matching_report_data_is_refused()
#[test] fn an_allowlisted_compose_action_is_accepted()
#[test] fn a_compose_action_with_a_disallowed_image_is_refused()
#[test] fn a_compose_action_of_an_unlisted_kind_or_container_is_refused()
#[test] fn a_different_app_id_is_refused()
#[test] fn a_different_key_provider_is_refused()              // the staging KMS only
#[test] fn the_instance_id_is_accepted_with_any_value()
#[test] fn a_compose_hash_event_other_than_mrconfigids_is_refused()
#[test] fn a_v1_document_is_refused_by_a_replay_aware_verifier()
// re-sign tool
#[test] fn observe_refuses_a_run_whose_replay_does_not_match()
#[test] fn observe_refuses_a_run_whose_compose_manager_check_fails()
#[test] fn observe_prints_the_compose_actions_for_review()
#[test] fn observe_never_emits_a_gateway_or_compose_manager_quote_as_a_pin_set()
#[test] fn observe_never_prints_the_instance_id_value()
#[test] fn the_default_registers_are_the_s16_set()
#[test] fn sign_embeds_the_allowlist_file()
// app
#[test] fn a_staging_build_earns_a_key_only_when_set_replay_and_allowlist_pass()
#[test] fn a_staging_build_refuses_an_unknown_event()
#[test] fn a_staging_build_refuses_a_replay_mismatch()
#[test] fn a_staging_build_accepts_an_allowlisted_compose_action()
#[test] fn a_staging_build_refuses_a_disallowed_compose_image()
#[test] fn a_staging_build_refuses_a_compose_manager_quote_from_another_td()
#[test] fn a_staging_build_takes_the_log_and_compose_record_from_the_quotes_report()
#[test] fn a_staging_build_never_takes_a_gateway_quote_for_the_model()
#[test] fn status_names_the_replay_and_allowlist_refusals()
```

- [x] **Steps 3-5** as in task 2, for each touched crate with and without `--features near-ai-staging`; `--no-default-features` for the attestation crate; the release check and the marker scan (the v2 format string is a new staging marker).
- [x] **Step 6: Commit** — `Replay RTMR3 against a signed allowlist in staging pins` (landed as several commits on `staging-rtmr3-replay`).

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

### Task 7b: Open sign-up, and the ingest under `near-ai-staging` in CI (`staging-allowlist-admission`, base `staging-server-guard`)

Spec: S12 (supersedes S5), section 3.12. The branch name predates S12.

- No staging admission code and no staging setting: a staging ingest provisions and binds a new NEAR AI login exactly as a default build does, with no invite and no list.
- A staging build refuses the NEAR wallet ceremony by name (`staging_wallet_provisioning_closed`) before reading any configuration, and both capabilities endpoints report `ready: false` for it, so NEAR AI is the only way to make a staging account.
- A staging build does not require the admission ledger (`near_provisioning_admission_ready`) for NEAR AI sign-in, compiled out under the feature (owner decision 2026-10-22, "Skip ledger on staging"). A default build still requires it.
- A staging build also admits uploads from those accounts without the ledger: a `nearai-` tenant whose provisioned device resolves skips the ledger while it is unconfigured (`admission::staging_admits_without_ledger`, compiled to `false` in every other build). Tests: `a_staging_build_admits_the_first_upload_without_the_admission_ledger`, its default-build twin `a_default_build_refuses_the_first_upload_without_the_admission_ledger`, and `a_production_build_never_admits_without_the_ledger`.
- CI checks, lints and runs the whole ingest lib under `near-ai-staging` (`trace-commons-ingest tests, whole bin, near-ai-staging, against PostgreSQL`, not a required check), and requires the two tests below to have run.

Tests first: `open_sign_up::a_new_near_ai_login_gets_an_account_with_no_invite` (no `cfg`; runs in both builds, red under the feature while S5's list was in place), `a_staging_build_never_offers_wallet_provisioning`, `open_sign_up::a_staging_build_signs_up_without_the_admission_ledger` and its default-build twin `a_default_build_refuses_sign_up_without_the_admission_ledger`.

### Task 8: The app reads its channel from the dylib (`staging-ffi-channel`)

Spec: sections 4.1 (last paragraph), 4.3.

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` (`tc_build_channel_json()`: channel, state directory name, state directory env name, passkey RP ID, URL scheme; no secret, no key)
- Modify: both `trace_commons.h` copies
- Modify: `macos/Sources/TCShellCore/StateDirectory.swift` (default directory and env name from the FFI), `macos/Sources/TCShellCore/NativePasskey.swift` (RP ID from the FFI), `macos/Sources/TraceCommonsApp/WindowBreadcrumb.swift` (subsystem from the bundle id), app startup (compare Info.plist `TCChannel` with the dylib's; refuse to start the daemon on a mismatch)
- Tests: FFI tests; `macos/Tests/TCShellCoreTests/StateDirectoryTests.swift`, `NativePasskeyTests.swift`, a new `ChannelAgreementTests.swift`
- Note (spec 4.4): the app opens its directory with `ConfigStore::open`, never `ConfigStore::resolve`, so the other-channel override refusal reaches it only through `tc_resolve_state_dir`. `StateDirectory` sends every directory taken from the override variable through it and refuses to start on `state-dir-other-channel`. Tests: FFI `an_override_naming_the_other_channel_directory_is_refused_without_a_path`; Swift `StateDirectoryTests` (the check is consulted for the override and only for it) and `TCBridgeTests/StateDirectoryBridgeTests` (the real dylib refuses the other channel's directory).

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
- Modify: the IronWire home, pointer and control port per spec 4.2 and S10 (`channel.rs`, `daemon/private_inference.rs`, `daemon/ironwire_pointer.rs`, `daemon/settings.rs`); tests `a_staging_build_uses_its_own_ironwire_home`, `a_staging_build_ignores_the_production_pointer`, and a production pin that its home, pointer and port are unchanged
- Modify: `scripts/check-aasa.sh` callers in CI only if a test needs the staging app id (the script already takes it)
- Tests: `scripts/ci/` test for `info-plist.sh` under each channel; `deep_link.rs` tests

- [ ] **Step 1: Write the failing tests** — `info-plist.sh` with `TC_CHANNEL=production` produces today's plist exactly (diff against a fixture taken before the change); with `staging` it has the staging bundle id, name, scheme and no production feed; `deep_link`: `a_staging_build_refuses_the_production_scheme`, `a_default_build_refuses_the_staging_scheme`.
- [ ] **Steps 2-4** — the release check and self-test (the build scripts must still not name the feature); a local `TC_CHANNEL=staging macos/scripts/make-app-bundle.sh` ad-hoc build that launches and reports `channel: staging` (describe the run; no screen captures).
- [ ] **Step 5: Commit** — `Build the macOS bundle for a channel from one file`.

---

### Task 9b: The in-window staging label (`staging-window-label`, base `staging-bundle-channel`)

Spec: S7, section 4.8.

- One short persistent label in the main window when the channel is staging; nothing in a production build.
- The string lives in the Rust copy tables, follows docs/ui-copy-guide.md, and is marked for owner approval in the PR.

Tests first: `a_staging_channel_shows_the_label`, `a_production_channel_shows_none`, and the copy-table checks.

### Task 10: The staging workflow (`staging-macos-workflow`)

Spec: sections 6.1, 5.7.

**Files:**
- Modify: `.github/workflows/staging-macos.yml` (the real job: `workflow_dispatch` only, no GitHub environment (spec S14), build with `TC_CHANNEL=staging`, `swift test`, sign, notarize, staple, channel agreement, staging appcast, upload to the staging bucket, GitHub pre-release `staging-<date>-<sha>`; `actions/checkout@v6`, `actions/cache@v5`, ends with `./.github/actions/trim-cargo-cache` if it caches `target`)
- Modify: `scripts/updates/generate-appcast.sh` (output path and key secret name as arguments; production defaults unchanged)
- Create: `scripts/ci/assert-no-staging-markers.sh` (fails if a binary contains the staging pin key bytes, the pin URL path or the `trace-commons-near-ai-pins/v1` format string; staging host names are not markers, since production holds only their digests) and its self-test
- Modify: `.github/workflows/release-apps.yml` (run the marker scan on the dylib and CLI after the macOS build)
- Tests: the marker script's self-test (a seeded binary with the key bytes is refused)

- [ ] **Step 1: Write the failing self-test**, run it.
- [ ] **Step 2-3: Implement.**
- [ ] **Step 4: Verify** — release check and self-test; marker self-test; `actionlint`; one dispatch of the workflow from main by the owner once its prerequisites exist (spec 6.1; not a precondition for merging).
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

**NEAR AI sign-in (S12, spec 3.12).** `staging.env.template` leaves NEAR AI login provisioning open: there is no allow-list setting and no invite to configure, and it must not require tenant access grants for sign-in (`TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS` is not read by the sign-in path; keep the pilot template's value). What `near_ai_login_readiness_named` requires on a staging build, each of which the template or its secrets must supply:
- `TRACE_COMMONS_NEAR_PROVISIONING_ENABLED=true` (the literal `true`).
- The account registry database (the PostgreSQL mirror).
- `TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER` and `TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY` (secrets).
- `TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL` (https) and `TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE`.
- Leave the admission ledger off (`TRACE_COMMONS_ADMISSION_ENABLED=false` or unset) and set no `TRACE_COMMONS_WITNESS_*` bypass settings: a staging build creates NEAR AI accounts without the ledger (S12), and the ledger is what would require the witness. A default build still refuses sign-in without the ledger (`near_provisioning_admission_ready`).

The wallet settings (`_WITNESS_JSON`, `_PUBLIC_ORIGIN`) are not needed: a staging build refuses the wallet ceremony.

**Notes from Task 7's boot guard (#1548):**
- `staging.env.template` must always set `TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL`, even with no scorer configured. Unset, it resolves to the production gateway, and a staging ingest refuses to boot.
- The guard accepts only the exact staging API host. The staging gateway serves attestation and is the configured base; whether NEAR AI staging also has per-model completions hosts is unconfirmed (spec open item 3). If it does, a staging scorer configured on one is refused at boot. Once that is confirmed, either let the staging build accept those hosts, or add their digests to the default build's refusal so a production ingest cannot name them.
- `TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL` is not checked. The self-hosted filter is not built for staging (spec open item 4); if it is, the staging build should refuse any NEAR AI host other than staging there too.

---

### Task 12: Runbook and tester guide (`staging-runbook`)

Spec: sections 3, 5.4, 7.

**Files:**
- Create: `docs/operator/staging-environment.md` (provisioning order, Secret Manager names and IAM, DNS, first deploy, the pin re-sign procedure, the payout steps by reference to the payout design section 2, rollback, what to check after a NEAR AI staging redeploy, and that sign-up is open to any staging NEAR AI account (S12): there is no list to keep, and stopping the ingest service is how to shut staging to everyone)
- Create: `docs/staging-testers.md` (the spec's section 7, in plain words, following `docs/ui-copy-guide.md`; no invite step: a tester makes a staging NEAR AI account and signs in)
- Modify: `docs/operator/README.md` (index), `README.md` if its operator checklist lists environments

**Progress (2026-10-22):** drafted as `docs/operator/staging-tester-guide.md` (the tester guide, in the operator directory so the index lists it) and `docs/operator/staging-runbook.md`, both listed in `docs/operator/README.md`. Done: tester install, checksum check, staging account, what works and what does not, removal; operator build, publish check, update feed, pin re-sign pointer, shutdown. Remaining: the "Server bring-up (pending Task 11)" section (Task 6b filled "Serving the first pin set"); the report channel (open item 6, written as TBD); a final walk of every command once Task 11 lands.

- [x] **Step 1 (partly):** The build, publish and feed commands were checked against `staging-macos.yml`; the server commands wait for Task 11.
- [ ] **Step 2: Commit** — `Add the staging environment runbook and tester guide`. The first part is committed; the rest follows Task 11.

**Done when:** an operator can bring up the VM, deploy, sign pins and publish a build from the runbook alone, and a tester can install and sign in from the guide alone.
