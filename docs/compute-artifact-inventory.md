# Compute worker artifact inventory and integrity checks

Local evidence, 2026-09-04. No release authorization or installed worker test.
Design: [packaging and resource policy](superpowers/specs/2026-09-04-compute-pilot-packaging-policy.md).

## Actual build

Holonear source: `cef95b3692d8b5baacd0b0139b6e286308ba6f23`, clean worktree.
Rust 1.96.0, native arm64, CMake 4.4.3, Xcode SDK reported by the binary: 26.5.
Cargo.lock SHA-256: `37db29fd690e7380c21dc870949e5fda8d9eafad487ba7b21a087d6f6f8517d8`.
The `mlx-sys` CMake fetch selected MLX tag `v0.25.1`, actual checkout
`eaf709b83e559079e212699bfc9dd2f939d25c9a`. Cargo's lock alone does not pin every
CMake download; release provenance must retain those transitive source pins too.

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --release \
  -p holonear-cli --bin holonear --features mlx --locked
```

Build succeeded in 6m46s on the successful attempt. The first sandboxed attempt
failed on Metal compiler cache permissions; a scoped build-permission retry
succeeded. Two upstream MLX C++ header warnings were emitted; this was not a
warnings-free upstream build. No dependencies or upstream source were changed.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `holonear` | 66,609,680 | `d8d9eefc80a520b942b8113757ffb723b1191efc767a929f5f60f9d6aa7094f9` |
| `mlx.metallib` | 88,150,285 | `dec47dccc97538d2758eb27ab09bcb7c5de4660e298521ecee9cae4fb2bca802` |

These identify this local build, not reproducible-build or release pins. The
Mach-O is thin arm64, `LC_BUILD_VERSION` platform macOS, minimum 15.0. Its
signature is linker-generated ad hoc, without a TeamIdentifier or sealed resources.
A Developer ID signature will change the worker hash.

`otool -L` found only system dependencies: IOKit, SystemConfiguration, CoreWLAN,
SecurityFoundation, Foundation, Security, CoreFoundation, Metal, Accelerate,
libSystem, libobjc, libc++ and libiconv. The MLX build uses static `mlx`/`mlxc`,
Metal ON, Accelerate enabled, and Metal JIT OFF. This is link/build evidence,
not an execution proof that a GPU job succeeds.

Reproduce the read-only inventory from Trace Commons:

```sh
bash macos/scripts/inventory-compute-worker.sh \
  /absolute/holonear /absolute/mlx.metallib
```

The library in this build lives under
`target/release/build/mlx-sys-099c632f68b81de5/out/build/_deps/mlx-build/mlx/backend/metal/kernels/mlx.metallib`.
That Cargo directory suffix is local, not a stable packaging API. Select it from
the successful build's outputs, never the first match across stale build trees.

## Packaging gap discovered

The built MLX `device.cpp::load_default_library` searches beside the binary for
`mlx.metallib`, then `Resources/mlx.metallib`, then an optional SwiftPM bundle,
then its compiled `METAL_PATH`. The actual compile flags embed a build-directory
absolute path. `mlx-sys` does not make this a runtime environment override.

Therefore the design's `Contents/Resources/Compute/assets/mlx.metallib` is an
integrity staging location, NOT currently a resolved worker runtime location.
Package assembly must first provide an explicit bundle-aware MLX asset lookup
and remove reliance on the build-directory fallback (or revise the layout with
review). Do not scatter duplicate shader files or assume a development machine's
successful load proves a clean installation. Test relocation with build assets
unavailable, and refuse missing/modified shaders before any worker launch.

## Implemented validator

`trace_commons_contributor::compute::artifact::check_integrity` reads only the
fixed manifest/helper paths and explicitly listed resources beneath a bundle.
`ArtifactExpectation.manifest_sha256` pins the exact manifest bytes from independent
reviewed policy, checked before JSON parsing; rewriting the manifest alongside a
worker does not preserve a successful check.
No constructor, consent, environment, FFI or shipping launch behavior changed.

Schema v1 has strict fields: `schema_version`, `source_revision`, `target`,
`backend`, `minimum_macos` (three integer components), `ipc_version`,
`compatibility_id`, `signing_identifier`, `signing_team`, `worker`
(`size_bytes`, `sha256`), and `assets` (`relative_path`, `size_bytes`, `sha256`).
Required resource: exactly spelled `mlx.metallib` must occur in the list.
The parser rejects unknown/duplicate fields, malformed pins, unsupported metadata,
case-colliding paths and traversal. Manifest limit 64 KiB; at most 128 resources;
512 MiB per file and 1 GiB combined. These bounds cover executable resources,
not model weights or a runtime cache allowance.

Checks stream file hashes, inspect the worker header from the same hashed bytes,
and reject symlinks/non-regular files below the canonical
bundle root, inspect bounded thin-arm64 executable load commands and require the
actual minimum OS to match metadata. Unix executable/resource mode checks reject
an inert worker or executable-mode data resource. The expectation is independent
caller-supplied release metadata; no shipping expectation is installed yet.

Success returns counts, not launch authorization. The validator does not verify
OS signatures, prove metadata's backend claim, audit an entire bundle or dynamic
library graph, enforce a filesystem sandbox, or prevent same-user replacement
races. Both `Contents/Helpers` and `Contents/Resources/Compute` are closed
inventories: extra files, symlinks and unnecessary directories at any depth are
refused. Other app directories are outside this check. Framework/native-library inventory
support and trusted signature verification are required before activation.

The `compute-artifact` example is a read-only developer harness. Its signing
labels are compared as metadata only; supplying real-looking labels proves no
signature. Its final required argument is the independently obtained 64-hex-digit
manifest SHA-256; do not derive that trust input from the bundle being checked.
It explicitly prints `signature_verified=false launch_authorized=false`.

## Verification

Artifact tests cover schema/bounds, required resources, independent
compatibility expectations, traversal/case collisions, missing/modified files,
symlinks, executable permissions, post-sign byte changes and Mach-O mismatches
even when the file hash matches. Header fixtures and signature-change bytes are
synthetic, not real signed executables. Four unchanged license-boundary tests,
warnings-denied Clippy and Rust 1.92 standalone library check passed. No dependency
or lockfile changes were needed.

The read-only inventory script ran successfully against the actual release
worker and Metal library. The Rust example also passed against a temporary
integrity-only bundle tree containing those real files: one resource and
154,759,965 total bytes. The manifest deliberately used test signing metadata;
the result reported signature verification and launch authorization both false.
That tree is not a runnable or signed application bundle.

Resource policy is now enforced through native observations (see
`compute-native-observers.md`); the separate Holonear bundle-aware MLX lookup
patch has local relocation evidence. OS signature checks, signed distribution,
test-pool acceptance and installed GPU qualification remain subsequent gates.

## Inert assembly harness

`compute-package` assembles only the compute subtree in a **new** directory:

```sh
cargo run -p trace-commons-contributor --locked --example compute-package -- \
  /absolute/new/Pilot.app /absolute/input/holonear /absolute/input/mlx.metallib \
  FULL_40_CHARACTER_SOURCE_REVISION tc-compute-v1 org.example.worker TESTTEAM01
```

The parent must exist. Use canonical absolute input paths: file and parent
symlinks are refused. Existing output directories are never overwritten, including
empty ones. Failed assembly may leave partial output for inspection; it is not
automatically deleted and must not be shipped. Inputs are read only, copied with
bounded streaming hashes, and staged with executable worker/non-executable asset
permissions. The manifest pins the copied bytes, then the existing strict
integrity validator checks the result against the digest of the bytes just generated.
Assembly prints that digest for release review; this producer self-check is not
independent provenance and does not install a trusted release policy. Only macOS 15 arm64 MLX with the single
`mlx.metallib` asset is supported by this first assembly harness.

This is not a complete app bundle or an integration into `make-app-bundle.sh` or
`make-release-dmg.sh`. No keys, network, signing, launch, or production constructor
are involved. Metadata supplied on the command line is a declaration, not source
provenance or signature proof. Success explicitly reports all three of signature
verification, provenance verification, and launch authorization as false.

For a future signed release, sign nested code before generating its final hash
manifest, then seal the outer app. Re-signing the staged helper invalidates this
manifest. Do not copy this subtree into an already sealed app. Trusted signature
requirements and complete nested-code inventory still need implementation before
the release scripts can safely opt into compute. Same-user replacement races are
not prevented by this developer tool.

Assembly checkpoint verification: five harness tests and nine existing artifact
tests passed with warnings denied, as did example Clippy and formatting checks.
The harness also assembled and integrity-checked the previously built worker
(`d97a49703334bf74e5b3985a4a15e442afadc1c29dce33a4dc441062e7e61eb2`)
and Metal library
(`dec47dccc97538d2758eb27ab09bcb7c5de4660e298521ecee9cae4fb2bca802`)
into a fresh temporary tree using test signing labels. No worker was launched.
The source revision declaration was
`0b348bc0b4820b1a53404b0fa59f213b7a3ca130`; assembly does not independently prove
that provenance. Independent review identified destination-parent symlink
handling, which was tightened and covered by regression before this checkpoint.

## Read-only Developer ID verification checkpoint

After assembly and separately authorized signing, an operator can inspect the
helper and outer app without launching or modifying either:

```sh
python3 macos/scripts/verify-compute-signatures.py \
  /absolute/Installed.app org.example.app org.example.worker TEAMID1234
```

Supply identifiers and the team from independently reviewed release policy,
never from the package's manifest. The harness calls the fixed system
`/usr/bin/codesign` verifier for the helper and then the outer app, requiring
strict validity for every architecture and an Apple Developer ID Application
certificate chain with those identifiers and team. Ad-hoc signatures and Apple's
own differently signed system programs do not satisfy that requirement. Each
invocation has a 30-second timeout; missing verification, failure, missing worker,
symlinked paths, or malformed policy refuses. Verifier output is suppressed to
avoid disclosing private paths. It performs no signing, network request of its
own, executable launch, or package modification; system certificate validation
may consult macOS trust services.

This is a developer verification tool, **not** the runtime Security-framework
bridge or a packaged-worker constructor. It does not verify notarization,
hardened-runtime flags, accepted entitlements, model licensing, arbitrary nested
code inventory, source provenance, integrity manifests, or race-free launch.
Success reports `integrity_verified=false provenance_verified=false
launch_authorized=false`. Run the separate integrity check against the final
signed bytes; a pre-sign manifest is invalid after the helper changes. Runtime
policy must ultimately be compiled from reviewed release requirements rather
than accepting user-supplied authority.

Checkpoint verification, 2026-09-05: eight Python tests pass. The real macOS
`csreq` compiler accepts the generated requirement; the real `codesign` verifier
refuses both the unsigned fixture and `/bin/ls` under the worker requirement.
Other tests pin fixed command arguments, independent app/helper identities,
outer-app failure, timeouts and symlink refusal. No positive Developer ID package
was supplied, so mocked success tests establish command wiring only. No signing,
worker launch, deployment or production enablement was performed.

### Signature harness limits

Use canonical absolute paths, including `/private/tmp` instead of macOS's
symlinked `/tmp`. The harness deliberately refuses symlinked ancestors; this
conservative developer-tool rule is not a race-free launch guarantee.

A successful Developer ID requirement check does not establish notarization,
Gatekeeper acceptance, or fresh revocation status. Those are separate release
and runtime gates; this inert tool must not be used as a substitute for them.
