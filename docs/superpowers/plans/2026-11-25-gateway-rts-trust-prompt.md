# Gateway Side of the Red Sandbox Trust Prompt: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the upstream `rts` trust prompt and signed attestation policy feed in Gateway, and stop a NEAR AI production redeploy from blocking a Gateway release.

**Architecture:** Gateway runs `rts` in-process. `red_team::run` calls `rts::launch::run` inside the Red Sandbox terminal window, so the prompt, the feed and the trust store all arrive with a dependency bump. The feed only works when its two public keys are compiled in (`option_env!("RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX")`), so the bump also pins the keys at build time. Once an unlisted production build prompts instead of failing, the release gate verifies production in two passes: blocking without the pin, and non-blocking with the pin to report drift.

**Tech Stack:** Rust (`trace-commons-contributor`), GitHub Actions (`release-apps.yml`), the GTK and Tauri workspaces' lockfiles, and the flatpak vendor list.

**Spec:** upstream `nearai/nearai-red-sandbox` `docs/specs/2026-11-25-user-trusted-compose-design.md`, "Gateway" section (PR #69). The upstream signed-feed design is `docs/specs/2026-11-24-signed-attestation-policy-feed-design.md` (PR #66, merged).

## Prerequisite

- nearai/nearai-red-sandbox#69 is merged, **including the fix that moves the trust store to `~/.local/share/nearai-red-sandbox/trusted-composes.json`**. Call its merge commit `R` (40 hex). Every task uses `R`. Do not start Task 1 before #69 merges: a rev on the PR branch could be rewritten.

## Owner decisions this plan needs

- **D1: how a Gateway user forgets a trusted build.** The upstream prompt says "until you run `rts trust forget <prefix>`". Gateway users have no `rts` on their PATH: the runtime lives under `<config>/red-team/runtime`. The options:
  - **(a) Recommended:** add `near-ai red-team trust list|forget`, which calls `rts::attest::trusted::{load, forget}` directly. Add an upstream `LaunchSpec` field naming the forget command, so Gateway's prompt says `near-ai red-team trust forget <prefix>`. This changes approved prompt copy, so it needs your approval of the exact line.
  - **(b)** Accept it for v1: the store file can be edited by hand, and support can explain.

  Task 3 is built only under (a).
- **D2 (decided in this plan; say if you disagree): compile the feed keys in through a repository `.cargo/config.toml` `[env]` entry.** The alternative was setting them only in release-workflow env. With the config entry, every build (dev, CI, release, both shells) gets the same feed behaviour, and a test pins the keys. Rotating a key then needs a commit here as well as an `rts` release. The CLAUDE.md rule against adding `RUSTFLAGS`/`CARGO_INCREMENTAL`/`rustc-wrapper` to that file does not cover public constants.

## Global Constraints

- `rts` is pinned to exactly one revision `R` everywhere. A git-dependency rev bump must touch all four:
  - root `Cargo.lock`
  - `crates/trace-commons-contributor-gtk/Cargo.lock`
  - `tauri-desktop/Cargo.lock`
  - `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json`: commit, `dest` dir, the `cp` commands, and the inline cargo config rev.

  `git grep` for the old revs (`a5bd279f3ca3`, and the stale `d10b305` still in `cargo-sources.json`) must return nothing afterwards.
- The `nearai/ironwire` rev stays `53518343781063b3c39bad9c4876488800ada89e`, the one `rts` at `R` resolves (checked: unchanged upstream since `a5bd279`). The `Cargo.toml` comment requires the two to match.
- No new crates. `rts`'s `Cargo.toml`/`Cargo.lock` did not change between `a5bd279` and #69's head; re-check at `R`. The `rts` version stays 0.1.10, so the `RTS_RELEASE` / `RTS_RELEASE_WINDOWS` runtime archive pins do not change.
- Feed public keys, comma-separated, CI first, as in upstream's `RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX`:
  `7b528575c138cd711ed863eb59a7a95d78d79d222a8a962ddb25196ccc14db5c,39982511b3d64da04fbb22cbec7e6040ae5c844ec750a428e6528f221ffd4d17`
- Staging builds keep `ComposePolicy::MrConfigOnly` (`red_team::attest_options`). They never prompt.
- No new user-facing text except under D1(a), with approved wording (`docs/ui-copy-guide.md` rule 9).
- Run everything once after the last task (repo CLAUDE.md, "Executing a multi-task plan"). Per task, run only `cargo check -p trace-commons-contributor --all-targets`.

## Review Focus

- **A release build without the keys:** a missing or empty `[env]` would ship a Gateway that never fetches the feed and never prompts. Pinned by Task 1's `the_attestation_policy_feed_keys_are_compiled_in`, which runs on the macOS and Windows test jobs.
- **A Windows Gateway session whose stdin is not a console:** the prompt is gated on stdin and stderr both being terminals, so a Windows Terminal launch through `near-ai-gateway-helper.exe` that is not interactive would silently refuse as today. Checked manually in Final Verification. No automated test can exercise a real console.
- **A release run on a day NEAR has redeployed production:** the gate must still publish, with a visible drift warning. Pinned by Task 2's `release_pipeline` assertions.
- **The flatpak vendor list drifting:** pinned by the `git grep` check in Task 1 and by `release_pipeline::cargo_sources_json_*`.

---

### Task 1: Bump `rts` to `R` and compile in the feed keys

**Files:**
- Modify: `crates/trace-commons-contributor/Cargo.toml:162` (the `rts` rev)
- Modify: `Cargo.lock`, `crates/trace-commons-contributor-gtk/Cargo.lock`, `tauri-desktop/Cargo.lock`
- Modify: `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json` (regenerated)
- Create: `.cargo/config.toml` with only `[env]`
- Test: `crates/trace-commons-contributor/src/managed/red_team_tests.rs`

**Interfaces:**
- Consumes: `rts::attest::feed::FEED_KEYS: &[[u8; 32]]` at `R`.
- Produces: a Gateway build whose `rts` fetches and verifies the signed feed.

- [ ] **Step 1: Write the failing test** (under the same `cfg(any(all(target_os = "macos", target_arch = "aarch64"), windows))` as the `rts` dependency):

```rust
#[test]
fn the_attestation_policy_feed_keys_are_compiled_in() {
    let hex: Vec<String> = rts::attest::feed::FEED_KEYS.iter().map(hex::encode).collect();
    assert_eq!(hex, [
        "7b528575c138cd711ed863eb59a7a95d78d79d222a8a962ddb25196ccc14db5c",
        "39982511b3d64da04fbb22cbec7e6040ae5c844ec750a428e6528f221ffd4d17",
    ]);
}
```

- [ ] **Step 2: Confirm it fails.** At `a5bd279`, `rts::attest::feed` does not exist, so the result is a compile error. After the rev bump and before the `[env]`, the assertion fails because `FEED_KEYS` is empty. Record both.
- [ ] **Step 3: Bump.**
  - Set `rev = "R"` in `Cargo.toml`, then `cargo update -p rts` at the root.
  - Run `cargo check` in `crates/trace-commons-contributor-gtk` and in `tauri-desktop/src-tauri`. Each lockfile diff must change only the `rts` and `dcap-qvl` source lines, with 0 version changes and 0 dropped crates.
  - Regenerate `cargo-sources.json` with the generator command in `flatpak/ai.tracecommons.Contributor.yml`. It needs network, `aiohttp` and `tomlkit`; ask before installing either, per the dependency policy.
- [ ] **Step 4: Add `.cargo/config.toml`:**

```toml
# Ed25519 keys that sign NEAR AI Red SandBox's attestation policy feed (CI key,
# then the offline recovery key), compiled into `rts` with option_env!. Without
# them `rts` never fetches the feed and never offers its trust prompt. Public
# values; they match nearai/nearai-red-sandbox's README and its
# RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX variable.
[env]
RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX = "<the two keys from Global Constraints>"
```

- [ ] **Step 5: Run** the new test plus `cargo test -p trace-commons-contributor --test it -- 'release_pipeline::'`. Check that `git grep -n -e a5bd279f3ca3 -e d10b305` prints nothing.
- [ ] **Step 6: Commit** `git commit -m "Bump rts to the trust prompt and compile in the policy feed keys"`.

### Task 2: Split the release gate's production check

**Files:**
- Modify: `.github/workflows/release-apps.yml`, job `release-attestation`
- Modify: `crates/trace-commons-contributor/tests/release_pipeline.rs`, `live_attestation_is_required_before_app_publication`

**Interfaces:**
- Consumes: the job's existing checkout of `rts` at the locked rev in `$RUNNER_TEMP/release-red-sandbox`.
- Produces: three steps:
  1. **Blocking.** Production with `RTS_LIVE_COMPOSE_POLICY=mr-config-only`, then staging with `mr-config-only`.
  2. **Non-blocking** (`continue-on-error: true`). Production with `RTS_LIVE_COMPOSE_POLICY=pinned` and `RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX` set to the Global Constraints keys, so pinned and feed-endorsed both count. It is named `Report production builds the policy does not list yet`.
  3. A step that runs only when (2) failed and emits `::warning::` naming the run, so drift is visible on the run page.

- [ ] **Step 1: Write the failing test.** Extend `live_attestation_is_required_before_app_publication`:
  - The blocking production command contains `RTS_LIVE_NEAR_ENVIRONMENT=production RTS_LIVE_COMPOSE_POLICY=mr-config-only \`.
  - The step containing `RTS_LIVE_COMPOSE_POLICY=pinned` has `continue-on-error: true`, sets `RTS_ATTESTATION_POLICY_PUBLIC_KEYS_HEX`, and is the only step that does.
  - No `RTS_LIVE_COMPOSE_POLICY=pinned` appears in a step without `continue-on-error: true`.
  - `cargo test --locked --test live_attest -- --ignored --test-threads=1` appears exactly 3 times.
  - The existing `publish` / `windows-app` / `linux-flatpak` `needs` assertions are unchanged.
- [ ] **Step 2: Confirm it fails.**
- [ ] **Step 3: Edit the workflow.** Comment the reason in the file: an unlisted production build now prompts in Gateway instead of failing, so pin drift is reported, not blocking. A broken verifier still blocks through the `mr-config-only` pass.
- [ ] **Step 4: Run** `release_pipeline::` and check that `actionlint` is clean.
- [ ] **Step 5: Commit** `git commit -m "Report production pin drift without blocking a Gateway release"`.

### Task 3 (only under D1(a)): `near-ai red-team trust list|forget`

Write this task in full once D1 is answered: the upstream `LaunchSpec` field and its approved wording decide its interface. It is scoped to:
- a new `red-team trust` subcommand in `crates/trace-commons-contributor/src/cli.rs`, which calls `rts::attest::trusted::{load, forget}` with the user's home, using the same `rts::install::home_in` launch uses;
- output through `rts::attest::trusted::list_lines` and `rts::attest::prompt::store_warning_line`, which are already sanitized upstream;
- the upstream field set in `red_team::spec_for`.

### Final verification (once, after the last task)

- [ ] The repo CLAUDE.md "Local development" commands for the contributor crate: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`, clippy with the CI allow-list, and `cargo fmt --all -- --check`.
- [ ] The GTK workspace: `cargo check --locked` and its tests via `--manifest-path`. The Tauri workspace: `cargo check --locked`.
- [ ] All four `cargo deny` runs from CLAUDE.md, including `--all-features`.
- [ ] `scripts/ci/msrv-floor.sh`, or the `builds at the declared MSRV floor` CI job. `rts` declares 1.89.
- [ ] Manual, Apple silicon Mac, production channel build:
  - Launch a Red Team session.
  - In the Red Sandbox window, confirm the feed line reads as loaded (`[rts] Attestation policy: ...` naming the feed, not "compiled-in only").
  - Run `test -t 0` in the same terminal type to confirm stdin is a TTY.
- [ ] Manual, Windows, production channel MSIX: the same feed-line check through the `near-ai-gateway-helper.exe` launch.
- [ ] The first time NEAR redeploys production to a build the feed holds: confirm the prompt appears, that `y` launches, and that the next launch prints `[rts] NEAR AI gateway build <short>: you trusted it on <date>`. Record the result on the PR. This cannot be staged ahead of a real redeploy.
