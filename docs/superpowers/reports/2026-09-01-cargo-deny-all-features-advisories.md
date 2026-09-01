# cargo deny --all-features check advisories audit

Date: 2026-09-01
Command: `cargo deny --all-features check advisories`
cargo-deny version: 0.20.2
Exit code: 1

Investigation only. No dependency or config changes proposed in this report.

## Why this graph

The default-feature graph that `cargo-deny-default`
(`.github/workflows/cargo-deny.yml`) checks in CI cannot see an advisory in
a crate that only `near-ai-scorer` or `local-gpu-models` pulls in -- those
are optional dependencies, invisible to `cargo metadata` unless their
feature is enabled. `--all-features` unions every feature's tree into one
graph, which is the only way to see what a `local-gpu-models` or
`near-ai-scorer` build actually ships.

## Findings

Seven RUSTSEC advisories, all rooted in the mistralrs/candle/fastembed tree
(behind `local-gpu-models`) or the google-cloud-kms tree (behind
`gcp-kms`), none of them present in the default-feature graph.

| ID | Crate | Issue | Fix |
|---|---|---|---|
| RUSTSEC-2026-0204 | crossbeam-epoch 0.9.18 | `fmt::Pointer`/`fmt::Display` dereferences an invalid pointer (e.g. `Atomic::null`) | Upgrade to >=0.9.20 |
| RUSTSEC-2025-0057 | fxhash 0.2.1 | Unmaintained | No safe upgrade -- rustc-hash is the suggested replacement |
| RUSTSEC-2026-0118 | hickory-proto 0.25.2 | NSEC3 closest-encloser proof validation loops unbounded on a cross-zone SOA; OOM in release builds | No safe upgrade in 0.25.x -- the affected type moved to `hickory-net` 0.26.1 |
| RUSTSEC-2026-0119 | hickory-proto 0.25.2 | O(n^2) name-compression linear scan during message encoding; CPU-exhaustion DoS | Upgrade to >=0.26.1 |
| RUSTSEC-2026-0253 | lru 0.18.0 | `LruCache::pop()` is not panic-safe -- a panicking key `Drop` during `pop()` leaves dangling list pointers (use-after-free / double-free) | Upgrade to >=0.18.2 |
| RUSTSEC-2024-0436 | paste 1.0.15 | Unmaintained | No safe upgrade -- pastey is the suggested fork |
| RUSTSEC-2025-0134 | rustls-pemfile 2.2.0 | Unmaintained | No safe upgrade -- migrate to `rustls-pki-types`'s `PemObject` |

## Dependency paths

- crossbeam-epoch, fxhash, paste: via `crossbeam-deque`/`rayon-core` and
  `bm25`/`gemm`, under `mistralrs` -> `trace-commons-gate-enclave` ->
  `trace-commons-server` (`local-gpu-models`).
- hickory-proto (both advisories): via `hickory-resolver` ->
  `reqwest 0.13.3` -> `dcap-qvl` / `mistralrs` / `mistralrs-mcp` ->
  `trace-commons-server`.
- lru: direct dependency of `trace-commons-gate-enclave`
  (`local-gpu-models`).
- rustls-pemfile: via `tonic` -> `google-cloud-gax` /
  `google-cloud-googleapis` -> `google-cloud-kms` (`gcp-kms`).

## Status

Not cleared, not ignored, not triaged for severity or reachability. This
report exists so the gap is visible and assignable rather than silently
absent: the PR that wires `cargo-deny` into CI deliberately does not add
an `--all-features check advisories` job while these seven are
outstanding -- a permanently red required check is worse than the
coverage gap it would close. The default-graph advisories check (bundled
into `cargo-deny-default`) stays as the only advisories gate for now.

## Recommendation

Triage each of the seven for actual reachability in a `local-gpu-models`
or `gcp-kms` build -- an advisory two dependency hops from an optional
build-time-only tool is a different risk than one on the request-handling
path -- then either fix (four have safe upgrades: crossbeam-epoch,
hickory-proto x2, lru) or add explicit `deny.toml` `ignore` entries for
the three that do not (fxhash, paste, rustls-pemfile), before adding an
`--all-features check advisories` job to CI.
