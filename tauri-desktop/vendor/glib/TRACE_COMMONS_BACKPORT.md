# Maintained GLib 0.18 security backport

This is the published `glib` 0.18.5 crate from crates.io, with the original
MIT `LICENSE`, `COPYRIGHT`, README and source retained. Its upstream source
revision is `42b9caf98e03ded086362d9653ca58fe94dc8658` (gtk-rs-core tag 0.18.5,
`glib/`). The published archive SHA-256 is
`233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`.
Cargo's local cache markers `.cargo-ok` and `.cargo_vcs_info.json` are omitted;
this file preserves the latter's upstream revision.

Tauri's Linux GTK 3 / WebKit dependency family requires GLib 0.18, whose
upstream branch has no patched release. This copy is maintained only until
Tauri can use a supported binding family containing the upstream fix.

## Patch

`src/variant_iter.rs::VariantStrIter::impl_get` backports the two-line fix
from [gtk-rs-core PR #1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343):
make the out-pointer local mutable and pass `&mut p` to the variadic
`g_variant_get_child` C function. Passing `&p` while C writes through it
violates Rust aliasing rules and can turn the following string read into a
null-pointer dereference in optimized builds. This fixes
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).

The package version is `0.18.5+tc.1` to identify this reviewed backport;
`Cargo.toml.orig` remains the original published manifest for provenance.
The only additional upstream source edit is a crate-scoped allowance for
`unused_parens` and `mismatched_lifetime_syntaxes` in `src/lib.rs`. The registry
crate previously had these old style warnings capped by Cargo; path
dependencies expose them under the application's required `-D warnings`.
This keeps the backport small without weakening application or Clippy lint
settings. No API or other upstream implementation has changed. The additional
`tests/rustsec_2024_0429.rs` checks forward, backward and mixed iteration,
including `nth`, `nth_back` and `last`, with optimized code.

From the repository root:

```sh
python3 scripts/ci/test-glib-backport.py
```

The helper creates a temporary standalone harness, preserving `RUSTFLAGS="-D
warnings"` and `CARGO_INCREMENTAL=0`, without adding a workspace or generated
files under this vendor. `--offline` reuses dependencies already in the Cargo
cache. `--source /path/to/unpatched/glib` runs the same regression against a
reference copy when reproducing the failure.

Keep the Tauri workspace's `[patch.crates-io]` entry, lockfile source, and
scanner provenance verification together. Re-run the optimized regression
when changing this copy. Remove the vendor, patch entry and matching scanner
exception together once Tauri's Linux graph uses an upstream fixed release.
Do not broaden the exception to registry GLib 0.18 or other advisory IDs.
