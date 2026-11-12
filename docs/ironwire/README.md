# IronWire

IronWire is the loopback inference proxy the contributor daemon runs in-process
for Private AI. Its library crates live in this repository as
`crates/ironwire_*`, and this directory holds their design documents.

## Where it came from

The crates and these documents were imported on 2026-11-10 as a single
squashed snapshot of [nearai/ironwire](https://github.com/nearai/ironwire) at
`b234896acd44e1c2216f98d2de82361b9a0a5796`, the revision the contributor
pinned at the time. This repository is now their source of truth; changes land
here as ordinary pull requests, and nearai/ironwire is no longer pinned or
synced. Per-line history before the import is in that repository.

Not imported: the `ironwire` command-line binary, its macOS app, packaging,
release scripts and CI. Links to those point at the upstream revision above.

## License

The ironwire_* crates are licensed MIT OR Apache-2.0, copyright NEAR AI. Each
crate directory carries both license texts (`LICENSE-MIT`, `LICENSE-APACHE`)
from the upstream repository.

## Documents

- [AGENTS.md](AGENTS.md) -- working rules for IronWire code (code comments
  cite its numbered rules).
- [DESIGN.md](DESIGN.md), [TRUST.md](TRUST.md), [PROTOCOL.md](PROTOCOL.md),
  [TRANSLATION.md](TRANSLATION.md), [PRIVACY.md](PRIVACY.md),
  [EMBEDDING.md](EMBEDDING.md), [UPDATES.md](UPDATES.md),
  [PACKAGING.md](PACKAGING.md), [SCALE.md](SCALE.md),
  [CRITIQUE.md](CRITIQUE.md), [ROADMAP.md](ROADMAP.md).
