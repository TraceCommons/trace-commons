<!-- Short imperative subject, no feat:/fix: prefix, no emojis. -->

## What this changes



## Verification

<!-- Paste actual command output. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
- [ ] `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server`
- [ ] `cargo clippy` with the repo allow-list

## Licensing

- [ ] I license this contribution under **MIT OR Apache-2.0**, per
      [CONTRIBUTING.md](../CONTRIBUTING.md) — including for the AGPL-licensed
      server crates, which are distributed under AGPL-3.0-or-later regardless.
- [ ] Any new `.rs` file in `trace-commons-server`, `trace-commons-gate-api`, or
      `trace-commons-gate-enclave` carries the copyright + SPDX header.
- [ ] No permissive crate gained a dependency on an AGPL crate.
