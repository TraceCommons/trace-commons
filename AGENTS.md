# AGENTS.md — trace-commons

Rules every coding agent working in this repository needs, whatever tool it
runs in. This file stands on its own for licensing, which comes first because
it is the one rule here whose violation a compiler will never report.

`CLAUDE.md` holds the full repo guidance: build and test commands, the CI
jobs, conventions, and known gotchas. Read it too. Where a topic appears in
both files, the full text is in one and the other points to it.

## Licensing: the split is load-bearing

This repository is **split-licensed**. Which license applies depends on which
crate the code lives in.

### AGPL-3.0-or-later

Copyright (C) 2026 K&Z Partners LLC.

- `crates/trace-commons-server`
- `crates/trace-commons-gate-api`
- `crates/trace-commons-gate-enclave`
- `crates/trace-commons-ingest`

Every `.rs` file in these four crates begins with:

```rust
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
```

**Any new `.rs` file you add to these crates needs that header.** It goes at
line 1, above any inner attribute (`#![...]`) — a comment before an inner
attribute is legal Rust.

### MIT OR Apache-2.0

- `crates/trace-commons-protocol`
- `crates/trace-commons-contributor`
- `crates/trace-commons-contributor-sources`
- `crates/trace-commons-contributor-insights`
- `crates/trace-commons-contributor-platform`
- `crates/trace-commons-contributor-ffi`
- `crates/trace-commons-contributor-gtk`
- `crates/trace-commons-operator-client`
- `crates/trace-commons-mark`
- `crates/trace-commons-build-info`
- `crates/trace-commons-attestation`
- `crates/trace-commons-client-server-tests` (unpublished, tests only)
- `tauri-desktop/src-tauri` (`trace-commons-tauri-desktop`, a separate Cargo
  workspace)

These stay permissive deliberately. The contributor CLI, the desktop apps, and
the envelope protocol are meant to be embedded in proprietary agent harnesses;
Ironclaw consumes `trace-commons-protocol` directly. Do **not** add AGPL headers
to files in these crates.

### The one rule

**Permissive code may flow into the AGPL crates. Never the reverse.**

Do not add `trace-commons-server`, `trace-commons-gate-api`,
`trace-commons-gate-enclave`, or `trace-commons-ingest` to the dependencies of
any permissive crate — not even to reuse a single trait or type. Doing so
silently makes a shipped client copyleft, and nothing about the build will tell
you.

If you need a type on both sides, put it in `trace-commons-protocol` (or another
permissive crate) and let the AGPL side depend on it. That is the direction the
seam is designed for.

`crates/trace-commons-server/tests/license_boundary.rs` enforces this. If it
fails, the fix is to remove the dependency — **not** to edit the expected sets
in the test. Those sets are the specification; changing them to match your diff
defeats the check entirely.

Test-only `dev-dependencies` across the boundary are permitted, because they
never reach a published artifact. The existing set is pinned in that same test
file (`trace-commons-client-server-tests` takes one on `trace-commons-server`
under `cfg(not(windows))`). Keep that edge out of the contributor itself, so
client-only tests never compile the server. Adding a new one requires updating
that list on purpose.

### Inbound contributions are MIT/Apache, outbound is AGPL

Contributions arrive under `MIT OR Apache-2.0` — including contributions to the
four AGPL crates. Recipients of those crates still get them under AGPL; only
the inbound direction is permissive, so the project keeps the ability to
relicense what it distributes.

This is stated in `CONTRIBUTING.md`, in the PR template checklist, in `LICENSE`,
and in the README. If you touch any of those, keep them consistent — a
contributor who reads only one of them must not come away with a different
answer.

An AGPL header on a file is about how that file is *distributed*. It says
nothing about the inbound terms, and adding one does not change them.

### Why the gate crates are AGPL

`crates/trace-commons-gate-api/README.md` describes the gate traits as a seam
where a proprietary scoring backend substitutes. Putting that seam under AGPL is
deliberate: it is available to the copyright holder, who can license itself, and
closed to third parties.

There is no CLA, but the inbound MIT/Apache term above is what preserves this:
without it, a single outside contribution to either gate crate would end the
ability to grant a proprietary exception on it permanently. Do not accept a
contribution to these crates offered under other terms without raising it.

### AGPL section 13

Section 13 obliges anyone running a modified version as a network service to
offer users the Corresponding Source. `trace-commons-ingest` does this at
`GET /v1/source`.

That route is **unauthenticated, carries no tenant context, and sits outside
every fail-closed gate**, by design. Do not put it behind auth, do not add a
tenant predicate to it, and do not let a middleware change sweep it up. A
credential requirement defeats the section it exists to satisfy. See
`docs/operator/agpl-source-offer.md`.

`TRACE_COMMONS_SOURCE_URL` is a constant, not an environment variable. That is
intentional — section 13 binds the operator of a *modified* build, who is
already editing the source. Do not turn it into config.

### Dependency licenses

New dependencies must be combinable into an AGPL-3.0 work. `deny.toml` holds the
allow list; a GPL-2.0-only, SSPL, or proprietary dependency is a hard conflict.

```bash
cargo deny check licenses
cargo deny --features near-ai-scorer check licenses
cargo deny --features local-gpu-models check licenses
cargo deny --all-features check licenses
```

Run all four: the feature sets pull in different trees, and some crates
resolve only under `--all-features`, so that run is not redundant (`CLAUDE.md`
names the trees). `--features` is a global flag and goes before `check`. CI
runs `check licenses`, `check sources` and `check advisories` under
`--all-features`, so this is a pre-flight rather than the only enforcement.
Note this is separate from — and does not replace — the repo's standing rule
that new dependencies need explicit human approval before you add them.

### Checklist before claiming a change is done

- New `.rs` files in the four AGPL crates carry the header.
- No permissive crate gained a dependency on an AGPL crate.
- `cargo test -p trace-commons-server --test it -- 'license_boundary::'` passes, without
  editing the test's expected sets.
- `cargo deny check licenses` passes under all four invocations above, if you
  touched dependencies. CI runs the same check, plus advisories and sources,
  on every push.

## Client priority

Native SwiftUI (`macos/`) is the main client on macOS and the parity target
for the other shells (#1173 D1): no new Tauri work on macOS. GTK, Windows and
Tauri match its terminology, states, flows and affordances. `CLAUDE.md` ("What
this repo is") has the full statement.

## Product text

When adding or changing user-facing UI text, follow
[docs/ui-copy-guide.md](docs/ui-copy-guide.md).

**No new notices without explicit approval.** Do not add a banner, card,
one-time message, interruption, notification, or a daemon status channel that
feeds one, unless the owner has explicitly approved both the notice and its
wording. A spec or design principle calling for one is not approval: ask.
See rule 9 of the copy guide.

## Building and verifying

`CLAUDE.md` has the commands and the reasons. The rules for every agent:

- **CI applies `RUSTFLAGS=-D warnings`**, so plain `cargo check` does not
  catch what CI catches. Use the `RUSTFLAGS` form before claiming green.
- **Executing a multi-task plan.** Write each task's tests, but run the tests
  and CI checks once, after the last task. Per task, run at most a plain
  `cargo check` on the touched crates. See `CLAUDE.md` ("Executing a
  multi-task plan") for the details and the final run's command list.
- **Shared compile cache.** Set `rustc-wrapper = "sccache"` under `[build]`
  in your user-level `~/.cargo/config.toml`, never the repository's. For
  agent and worktree builds, also set `CARGO_INCREMENTAL=0` and
  `RUSTFLAGS="-D warnings"` for every cargo command, clippy included, in your
  user-level environment, and raise sccache's size limit to about 50 GiB.
  See `CLAUDE.md` ("Shared compile cache").

## Everything else

See `CLAUDE.md`: the CI jobs and which are required, hash-only logging,
fail-closed defaults, tenant scoping, RLS, the gate-contract conventions, and
the repo's known gotchas.
