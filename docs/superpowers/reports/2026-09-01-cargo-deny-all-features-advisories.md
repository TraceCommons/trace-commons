# cargo deny --all-features check advisories audit

Date: 2026-09-01
Command: `cargo deny --all-features check advisories`
cargo-deny version: 0.20.2
Exit code: 1

Investigation only. No dependency or config changes proposed in this report.

## Why this graph

An optional dependency is invisible to `cargo metadata` unless its feature
is enabled, so a graph that omits a feature cannot see an advisory in a
crate only that feature pulls in. `--all-features` unions every feature's
tree into one graph: 2,272 crates, a superset of every named graph (the
largest single one, `local-gpu-models-cuda`, is 2,086).

This matters because production does not build the default graph.
`cloudbuild.yaml` builds ingest with
`--features gcs-client,gcp-kms,near-ai-scorer`, and `deploy/pilot-gcp`
adds `near-attestation-collateral`.

## Per-graph results, measured 2026-09-01

| Graph | licences | sources | advisories |
|---|---|---|---|
| default | pass | pass | pass |
| `near-ai-scorer` | pass | pass | **fail** |
| `local-gpu-models` | pass | pass | **fail** |
| `local-gpu-models-cuda` | pass | pass | **fail** |
| `gcs-client` | pass | pass | pass |
| `gcp-kms` | pass | pass | **fail** |
| `near-attestation-collateral` | pass | pass | **fail** |
| `--all-features` | pass | pass | **fail** |

Licences and sources pass everywhere, which is why CI runs them once under
`--all-features`. Advisories now run there too -- see Status below.

## Findings

Seven RUSTSEC advisories, none present in the default-feature graph.
Attributed to the graph that actually pulls each one in, measured per
feature rather than inferred from the union:

- `gcp-kms` -- RUSTSEC-2025-0134 (rustls-pemfile), and nothing else.
- `near-attestation-collateral` -- RUSTSEC-2026-0118 and -0119
  (hickory-proto), RUSTSEC-2026-0204 (crossbeam-epoch).
- `near-ai-scorer`, `local-gpu-models`, `local-gpu-models-cuda` --
  RUSTSEC-2024-0436 (paste), RUSTSEC-2025-0057 (fxhash),
  RUSTSEC-2026-0204 (crossbeam-epoch), RUSTSEC-2026-0253 (lru).
- `gcs-client` -- none; its advisories check passes.

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

Resolved 2026-09-02. Advisories now run under `--all-features` alongside
licences and sources, so the graph production ships is gated rather than
only the default graph.

Two of the seven were fixed rather than carried:

- **RUSTSEC-2026-0204** (crossbeam-epoch) -- bumped 0.9.18 -> 0.9.20.
- **RUSTSEC-2026-0253** (lru) -- bumped 0.18.0 -> 0.18.3. This one was an
  unsound use-after-free in a **direct** dependency of
  `trace-commons-gate-enclave`, not a transitive leaf.

The lockfile diff is those two packages and nothing else.

The remaining five are `deny.toml` `ignore` entries, each carrying the
reason no upgrade reaches it from this tree. Two corrections to the
Recommendation this section replaces:

- **hickory-proto x2 do not have a reachable safe upgrade.** The fix is in
  0.26.1, but hickory-proto is pinned by `hickory-resolver` 0.25.x, which
  `reqwest 0.13.3` requires by semver. Reaching 0.26 needs a reqwest
  release, not a lockfile bump. The earlier claim that four had safe
  upgrades counted these two; only two did.
- **RUSTSEC-2026-0118 is the one to watch.** It is a genuine remote-input
  vulnerability -- an unbounded loop, OOM in release builds, on a crafted
  NSEC3 cross-zone response -- and it is reached when the ingest host
  resolves NEAR AI endpoints, so a hostile or hijacked DNS response is the
  trigger. It is carried because nothing in this tree can fix it today,
  not because it was judged low-risk. Re-check it the moment reqwest moves
  to hickory-resolver 0.26.

The other three (fxhash, paste, rustls-pemfile) are unmaintained-crate
advisories with no vulnerability and no upgrade path we control.
