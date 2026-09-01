# Contributing to TraceCommons

## Licensing of your contribution

**You license your contribution under `MIT OR Apache-2.0`, whichever crate you
are contributing to — including the AGPL-licensed server crates.**

This is deliberate and it is not the usual default, so it is worth stating
plainly rather than leaving to inference.

Opening a pull request against this repository means you agree that your
contribution is offered under **both** the MIT license (`LICENSE-MIT`) and the
Apache License 2.0 (`LICENSE-APACHE`), at the recipient's option. You keep the
copyright in your own work. You are not assigning it to anyone.

### Why, when half the repo is AGPL

The usual open-source convention is "inbound = outbound": your contribution is
licensed under whatever the project is licensed under. TraceCommons deviates,
in one direction only.

`trace-commons-server`, `trace-commons-gate-api`, and
`trace-commons-gate-enclave` are distributed under AGPL-3.0-or-later. Anyone who
*receives* those crates gets them under the AGPL and owes the obligations that
come with it — including the section 13 duty to offer source to network users.

Contributions arrive under MIT/Apache instead. MIT and Apache-2.0 code can be
combined into an AGPL work, so this changes nothing about what downstream
recipients get: the server crates ship AGPL either way.

What it does change is that the project retains the ability to relicense the
code it distributes. Without this, the first outside contribution to a crate
would freeze that crate's license permanently, because relicensing would require
tracking down every past contributor. That is a real failure mode and it is
easier to avoid than to fix.

Concretely, this means the copyright holder may distribute your contribution
under other terms, including proprietary ones, without asking you again. If you
are not comfortable with that, do not open a pull request — that is a legitimate
position, and it is better to know before you have written the code.

### What this is not

This is **not** a copyright assignment. You retain the copyright in your
contribution and may use, relicense, or redistribute your own work however you
like, without restriction.

There is **no CLA to sign** and no bot to click through. Opening the pull request
is the whole of it.

### Sign-off

Please sign your commits off:

```bash
git commit -s -m "Your commit message"
```

That adds a `Signed-off-by:` line, certifying the
[Developer Certificate of Origin](https://developercertificate.org/) — in
substance, that you wrote the contribution or otherwise have the right to submit
it under these terms. It is not a copyright transfer.

## The license boundary

Which license a *file* carries depends on which crate it lives in.

- **AGPL-3.0-or-later:** `trace-commons-server`, `trace-commons-gate-api`,
  `trace-commons-gate-enclave`. Every `.rs` file in these carries a copyright +
  SPDX header; new files need one.
- **MIT OR Apache-2.0:** everything else. These crates are meant to be embedded
  in proprietary agent harnesses.

**Permissive crates must never depend on an AGPL crate.** Not even to reuse a
single trait — it would silently make a shipped client copyleft. If you need a
type on both sides, put it in `trace-commons-protocol`.
`crates/trace-commons-server/tests/license_boundary.rs` enforces this; if it
fails, remove the dependency rather than editing the test's expected sets.

See `LICENSE` for the full statement and `AGENTS.md` for the working rules.

## Process

Branch protection on `main` requires:

- All CI checks green.
- A pull request (no direct pushes).
- Linear history (squash or rebase, no merge commits).
- Any review conversations resolved before merge.

Before pushing:

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

CI applies `RUSTFLAGS=-D warnings`, so a plain `cargo check` will not catch what
CI catches.

New dependencies need explicit approval before you add them, and must be
combinable into an AGPL-3.0 work (`cargo deny check licenses`, run under the
default, `near-ai-scorer`, and `local-gpu-models` feature sets). CI runs
`check licenses` and `check sources` under `--all-features`, a superset of
all three, and `check advisories` on the default graph, on every push and
pull request -- so this local run is a pre-flight rather than the only
enforcement.

Commit style: short imperative subjects, no `feat:` / `fix:` prefixes, no
emojis.
