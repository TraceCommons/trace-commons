//! Immutable public repository snapshots and evaluator-only oracles.

use super::{EvaluationFixture, RepositoryEvidence};

pub(super) const REPOSITORY_URL: &str = "https://github.com/TraceCommons/trace-commons";

pub(super) const MARK_COMMIT: &str = "b6722426bb4b83d90425494b664ac468d67943b5";
const MARK_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5";
const WITNESS_COMMIT: &str = "c443326564264b27f5c65cd385b89762e7a81ea1";
const WITNESS_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/c443326564264b27f5c65cd385b89762e7a81ea1";
pub(super) const FLATPAK_COMMIT: &str = "e89a628bf4be8b1e31dd278793b00391e9781f1c";
const FLATPAK_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/e89a628bf4be8b1e31dd278793b00391e9781f1c";
const WINGET_COMMIT: &str = "c5786be30bf4135ca0d5e63e5b8ee04f02acbc04";
const WINGET_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/c5786be30bf4135ca0d5e63e5b8ee04f02acbc04";
const PNG_COMPRESSION_COMMIT: &str = "13f53d4e2c453a0ba15de2ce7d8c6f77eb33a77b";
const PNG_COMPRESSION_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/13f53d4e2c453a0ba15de2ce7d8c6f77eb33a77b";
const UPDATE_FIXTURES_COMMIT: &str = "ddbfd593338541cf67af6cbd50d3af50de492ee2";
const UPDATE_FIXTURES_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/ddbfd593338541cf67af6cbd50d3af50de492ee2";
const RELEASE_STAMP_COMMIT: &str = "74e19601c517f6dd6d735dbeeca97b9a034727c3";
const RELEASE_STAMP_SOURCE: &str =
    "https://github.com/TraceCommons/trace-commons/commit/74e19601c517f6dd6d735dbeeca97b9a034727c3";

pub(in crate::skill_loop::evaluation) const PLAN_TASK_COUNT: usize = 6;

const MARK_LIBRARY: &str = r#"pub fn windows_tiles() -> Vec<BinaryExport> {
    const TILES: [(&str, u32); 3] = [
        ("windows/packaging/Assets/StoreLogo", 50),
        ("windows/packaging/Assets/Square150x150Logo", 150),
        ("windows/packaging/Assets/Square44x44Logo", 44),
    ];
    /// Scale percentages, excluding 100.
    const LADDER: [u32; 4] = [125, 150, 200, 400];

    let mut out = Vec::new();
    for (stem, base) in TILES {
        // Light only. A Start tile is composited on a background the app does
        // not choose, and the manifest sets `BackgroundColor` to transparent,
        // so the tile carries its own light surface rather than following a
        // system appearance it cannot observe.
        out.push(BinaryExport {
            repo_path: leak_path(format!("{stem}.png")),
            bytes: raster::png(Scheme::Light, base),
        });
        for percent in LADDER {
            // Round half up, which is not `div_ceil`: they agree on the .5
            // cases these three bases produce and disagree on everything else,
            // so using ceil here would make the comment above false the moment
            // a fourth tile size appears.
            let size = (base * percent + 50) / 100;
            out.push(BinaryExport {
                repo_path: leak_path(format!("{stem}.scale-{percent}.png")),
                bytes: raster::png(Scheme::Light, size),
            });
        }
    }
    out
}"#;

const MARK_EXPORTER: &str = r#"    if let Some(root) = repo_root {
        for tile in trace_commons_mark::windows_tiles() {
            let path = root.join(tile.repo_path);
            if let Some(parent) = path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    eprintln!("creating {}: {err}", parent.display());
                    return ExitCode::FAILURE;
                }
            }
            if let Err(err) = std::fs::write(&path, &tile.bytes) {
                eprintln!("writing {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
            println!("wrote {}", path.display());
        }
    }"#;

const MARK_DRIFT_CHECK: &str = r#"ASSETS="assets/mark"
TILES="windows/packaging/Assets"

cargo run --quiet -p trace-commons-mark --bin mark-export -- "$ASSETS" --repo-root .

if ! git diff --exit-code -- "$ASSETS" "$TILES"; then
  echo "FATAL: generated assets do not match the generator." >&2"#;

const MARK_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "crates/trace-commons-mark/src/lib.rs",
        contents: MARK_LIBRARY,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-mark/src/bin/mark-export.rs",
        contents: MARK_EXPORTER,
    },
    RepositoryEvidence {
        path: "scripts/mark/check-drift.sh",
        contents: MARK_DRIFT_CHECK,
    },
];
const MARK_TILES: &[&str] = &[
    "windows/packaging/Assets/StoreLogo.png",
    "windows/packaging/Assets/StoreLogo.scale-125.png",
    "windows/packaging/Assets/StoreLogo.scale-150.png",
    "windows/packaging/Assets/StoreLogo.scale-200.png",
    "windows/packaging/Assets/StoreLogo.scale-400.png",
    "windows/packaging/Assets/Square150x150Logo.png",
    "windows/packaging/Assets/Square150x150Logo.scale-125.png",
    "windows/packaging/Assets/Square150x150Logo.scale-150.png",
    "windows/packaging/Assets/Square150x150Logo.scale-200.png",
    "windows/packaging/Assets/Square150x150Logo.scale-400.png",
    "windows/packaging/Assets/Square44x44Logo.png",
    "windows/packaging/Assets/Square44x44Logo.scale-125.png",
    "windows/packaging/Assets/Square44x44Logo.scale-150.png",
    "windows/packaging/Assets/Square44x44Logo.scale-200.png",
    "windows/packaging/Assets/Square44x44Logo.scale-400.png",
];

const MARK_REGENERATION: &[&[&str]] = &[&[
    "cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root .",
    "cargo run --quiet -p trace-commons-mark --bin mark-export -- assets/mark --repo-root .",
]];
const MARK_VERIFICATION: &[&[&str]] = &[&[
    "scripts/mark/check-drift.sh",
    "bash scripts/mark/check-drift.sh",
]];

const WITNESS_COMPOSE: &str = r#"    image: ghcr.io/tracecommons/trace-commons-witness@sha256:f1d4c00266656f0227292efe7239595d6ad0bd7b9083c750d610c0e11b2689bc"#;

const WITNESS_GENERATOR: &str = r#"set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compose="${here}/docker-compose.yml"
manifest="${here}/app-compose.json"

check_only=false
if [[ "${1:-}" == "--check" ]]; then
  check_only=true
  manifest="$(mktemp)"
  trap 'rm -f "${manifest}"' EXIT
elif [[ $# -gt 0 ]]; then
  echo "usage: $(basename "$0") [--check]" >&2
  exit 2
fi

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }"#;

const WITNESS_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "deploy/witness/docker-compose.yml",
        contents: WITNESS_COMPOSE,
    },
    RepositoryEvidence {
        path: "deploy/witness/build-app-compose.sh",
        contents: WITNESS_GENERATOR,
    },
];
const WITNESS_REGENERATION: &[&[&str]] = &[&[
    "deploy/witness/build-app-compose.sh",
    "bash deploy/witness/build-app-compose.sh",
]];
const WITNESS_VERIFICATION: &[&[&str]] = &[&[
    "deploy/witness/build-app-compose.sh --check",
    "bash deploy/witness/build-app-compose.sh --check",
]];

pub(super) const FLATPAK_DEPENDENCY: &str = r#"ironwire_proxy = { git = "https://github.com/nearai/ironwire", rev = "4f58f5a1bb16b13a8952fdf4c86b238d662b4069" }"#;

const FLATPAK_MANIFEST: &str = r#"      # Generated, not written by hand: run
      #   pip install aiohttp tomlkit
      #   python3 flatpak-cargo-generator.py \
      #     crates/trace-commons-contributor-gtk/Cargo.lock \
      #     -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
      # (flatpak-cargo-generator.py comes from
      # https://github.com/flatpak/flatpak-builder-tools, and needs network
      # access -- it downloads nothing itself, but resolves checksums for
      # everything in the lockfile). Absent that file, the build-commands
      # above will fail at the network-sandboxed `cargo build` step, which
      # is the correct, honest failure rather than a silent full-network
      # build that defeats the point of a reproducible Flatpak.
      - cargo-sources.json"#;

const FLATPAK_CHECK: &str = r#"#[test]
fn the_flatpak_vendor_set_matches_the_gtk_lockfile() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock =
        std::fs::read_to_string(root.join("crates/trace-commons-contributor-gtk/Cargo.lock"))
            .expect("GTK Cargo.lock is readable");
    let sources = std::fs::read_to_string(
        root.join("crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json"),
    )
    .expect("cargo-sources.json is readable");
"#;

const FLATPAK_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "crates/trace-commons-contributor/Cargo.toml",
        contents: FLATPAK_DEPENDENCY,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml",
        contents: FLATPAK_MANIFEST,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor/tests/release_pipeline.rs",
        contents: FLATPAK_CHECK,
    },
];

const FLATPAK_REGENERATION: &[&[&str]] = &[
    &[
        "cargo metadata --format-version 1",
        "cargo update -p ironwire_proxy",
    ],
    &[
        "cargo metadata --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --format-version 1",
        "cargo update --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml -p ironwire_proxy",
    ],
    &[
        "python3 flatpak-cargo-generator.py crates/trace-commons-contributor-gtk/Cargo.lock -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json",
        "python flatpak-cargo-generator.py crates/trace-commons-contributor-gtk/Cargo.lock -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json",
    ],
];
const FLATPAK_VERIFICATION: &[&[&str]] = &[&[
    "cargo test -p trace-commons-contributor --test release_pipeline",
    "cargo test --manifest-path crates/trace-commons-contributor/Cargo.toml --test release_pipeline",
]];

const WINGET_SETUP: &str = r#"VERSION="${1:?usage: generate-manifests.sh <version> [asset-url]   e.g. 0.1.1}"
TAG="contributor-v$VERSION"
REPO="TraceCommons/trace-commons-server"
ASSET="trace-commons-contributor-x86_64-pc-windows-msvc.zip"
URL="${2:-https://github.com/$REPO/releases/download/$TAG/$ASSET}"

OUT="manifests/t/TraceCommons/Contributor/$VERSION"
MANIFEST_VERSION="1.12.0""#;

const WINGET_LOCALE: &str = r#"cat > "$OUT/TraceCommons.Contributor.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$MANIFEST_VERSION.schema.json
PackageIdentifier: TraceCommons.Contributor
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: Iqlusion Inc
PublisherUrl: https://tracecommons.ai
PublisherSupportUrl: https://github.com/$REPO/issues
PackageName: Trace Commons Contributor
PackageUrl: https://tracecommons.ai/install/
License: MIT OR Apache-2.0
"#;

const WINGET_NEXT_STEPS: &str = r#"Next, to submit:
  1. Fork and clone https://github.com/microsoft/winget-pkgs
  2. Copy $OUT into the fork at the same path
  3. Validate:  winget validate --manifest <path>   (on Windows)
     and ideally test: winget install --manifest <path>
  4. Open a pull request against microsoft/winget-pkgs"#;

const WINGET_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "scripts/winget/generate-manifests.sh",
        contents: WINGET_SETUP,
    },
    RepositoryEvidence {
        path: "scripts/winget/generate-manifests.sh",
        contents: WINGET_LOCALE,
    },
    RepositoryEvidence {
        path: "scripts/winget/generate-manifests.sh",
        contents: WINGET_NEXT_STEPS,
    },
];
const WINGET_REGENERATION: &[&[&str]] = &[&[
    "scripts/winget/generate-manifests.sh 0.12.2",
    "bash scripts/winget/generate-manifests.sh 0.12.2",
]];
const WINGET_VERIFICATION: &[&[&str]] =
    &[&["winget validate --manifest manifests/t/TraceCommons/Contributor/0.12.2"]];

const WINGET_OUTPUTS: &[&str] = &[
    "manifests/t/TraceCommons/Contributor/0.12.2/TraceCommons.Contributor.installer.yaml",
    "manifests/t/TraceCommons/Contributor/0.12.2/TraceCommons.Contributor.locale.en-US.yaml",
    "manifests/t/TraceCommons/Contributor/0.12.2/TraceCommons.Contributor.yaml",
];

const PNG_ENCODER: &str = r#"/// Encode straight RGBA pixels as a PNG.
///
/// Rows use the `Up` filter, which subtracts the row above. The mark is wide
/// bands of one colour, so almost every row becomes zeroes and the run-length
/// compressor collapses it. Without filtering the same image is stored
/// essentially verbatim: the 150px tile was 90kB before this and is a fraction
/// of that after, which matters because a scale-400 variant of it would
/// otherwise be well over a megabyte of committed binary.
pub fn encode_png(pixels: &[u8], size: u32) -> Vec<u8> {
    assert_eq!(
        pixels.len(),
        (size as usize) * (size as usize) * 4,
        "pixel buffer does not match {size}x{size} RGBA"
    );

    let raw = filter_up(pixels, size);
    let mut z = vec![0x78, 0x01];
    z.extend_from_slice(&deflate_rle(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}"#;

const PNG_FOREIGN_DECODER: &str = r#"#!/usr/bin/env python3
"""Decode the generated PNGs with an implementation we did not write.

crates/trace-commons-mark rasterizes the mark and encodes the PNG itself,
including a small hand-rolled deflate. Testing that encoder with the encoder's
own assumptions proves very little: a stream can be self-consistent and still be
something no other decoder accepts.

So this decodes with Python's standard-library `zlib` -- a different
implementation of the same specification -- and compares the result to the
raw pixel buffer the renderer produced. Both files are written by
`cargo run -p trace-commons-mark --example emit-verify`.

Standard library only, deliberately. Pillow would be the obvious tool and is not
available on every runner; `zlib` and `struct` are always there, which means
this check can run anywhere the drift check runs.

Usage: verify-png.py <dir-containing-mark-N.png-and-mark-N.rgba>
""""#;

const PNG_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "crates/trace-commons-mark/src/raster.rs",
        contents: PNG_ENCODER,
    },
    RepositoryEvidence {
        path: "scripts/mark/verify-png.py",
        contents: PNG_FOREIGN_DECODER,
    },
    RepositoryEvidence {
        path: "scripts/mark/check-drift.sh",
        contents: MARK_DRIFT_CHECK,
    },
];

const PNG_OUTPUTS: &[&str] = &[
    "windows/packaging/Assets/StoreLogo.png",
    "windows/packaging/Assets/Square150x150Logo.png",
    "windows/packaging/Assets/Square44x44Logo.png",
];
const PNG_REGENERATION: &[&[&str]] = &[&[
    "cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root .",
    "cargo run --quiet -p trace-commons-mark --bin mark-export -- assets/mark --repo-root .",
]];
const PNG_VERIFICATION: &[&[&str]] = &[
    &[
        "mkdir -p target/mark-verify && cargo run -p trace-commons-mark --example emit-verify -- target/mark-verify",
    ],
    &[
        "python3 scripts/mark/verify-png.py target/mark-verify",
        "python scripts/mark/verify-png.py target/mark-verify",
    ],
];

const UPDATE_FIXTURE_GENERATOR: &str = r#"write_manifest() {
  version="$1"
  out="$2"
  # Every CLI slug points at the same artifact so the fixtures exercise the
  # client on whatever host the test runs on.
  platforms=""
  for slug in windows-x86_64-cli linux-x86_64-cli macos-aarch64-cli macos-x86_64-cli; do
    entry="$(printf '"%s":{"url":"https://example.invalid/%s","sha256":"%s","size":%s}' \
               "$slug" "$slug" "$SHA" "$SIZE")"
    if [ -n "$platforms" ]; then platforms="$platforms,$entry"; else platforms="$entry"; fi
  done
  printf '{"schema_version":"trace_commons.update_manifest.v1","version":"%s","published_at":"2026-08-17T00:00:00Z","platforms":{%s}}' \
    "$version" "$platforms" | jq -S . > "$out"
}"#;

const UPDATE_FIXTURE_CONTRACT: &str = r#"The verify-before-swap logic for automatic updates exists twice: once in Rust
and once in Swift for the macOS app. These fixtures are the mitigation for
that duplication. Both suites read these exact bytes, so a check dropped in
either implementation fails a test rather than shipping.

Regenerate with `./regenerate.sh`. It is deterministic: keys come from fixed
seeds and manifests carry a fixed `published_at`, so re-running changes
nothing unless a fixture genuinely changed."#;

const UPDATE_FIXTURE_TEST: &str = r#"#[test]
fn the_good_manifest_verifies_and_names_every_cli_platform() {
    let manifest = good_manifest();
    assert_eq!(manifest.version, "9.9.9");
    for slug in [
        "windows-x86_64-cli",
        "linux-x86_64-cli",
        "macos-aarch64-cli",
        "macos-x86_64-cli",
    ] {
        assert!(
            manifest.platforms.contains_key(slug),
            "good fixture is missing {slug}"
        );
    }
}"#;

const UPDATE_FIXTURE_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "tests/fixtures/update-conformance/regenerate.sh",
        contents: UPDATE_FIXTURE_GENERATOR,
    },
    RepositoryEvidence {
        path: "tests/fixtures/update-conformance/README.md",
        contents: UPDATE_FIXTURE_CONTRACT,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor/tests/update_conformance.rs",
        contents: UPDATE_FIXTURE_TEST,
    },
];

const UPDATE_FIXTURE_OUTPUTS: &[&str] = &[
    "tests/fixtures/update-conformance/good/latest.json",
    "tests/fixtures/update-conformance/good/latest.json.sig",
    "tests/fixtures/update-conformance/bad-signature/latest.json",
    "tests/fixtures/update-conformance/bad-signature/latest.json.sig",
    "tests/fixtures/update-conformance/downgrade/latest.json",
    "tests/fixtures/update-conformance/downgrade/latest.json.sig",
];
const UPDATE_FIXTURE_REGENERATION: &[&[&str]] = &[&[
    "tests/fixtures/update-conformance/regenerate.sh",
    "bash tests/fixtures/update-conformance/regenerate.sh",
]];
const UPDATE_FIXTURE_VERIFICATION: &[&[&str]] = &[&[
    "cargo test -p trace-commons-contributor --test update_conformance",
    "cargo test --manifest-path crates/trace-commons-contributor/Cargo.toml --test update_conformance",
]];

const RELEASE_CORE_PACKAGE: &str = r#"[package]
name = "trace-commons-contributor"
version = "0.12.0""#;
const RELEASE_FFI_PACKAGE: &str = r#"[package]
name = "trace-commons-contributor-ffi"
version = "0.12.0""#;
const RELEASE_GTK_PACKAGE: &str = r#"[package]
name = "trace-commons-contributor-gtk"
version = "0.12.0""#;

const RELEASE_METAINFO: &str = r#"<releases>
    <release version="0.12.0" date="2026-09-08">"#;

const RELEASE_VERSION_GATE: &str = r#"crate_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' \
  crates/trace-commons-contributor-gtk/Cargo.toml | head -1)"
meta_version="$(sed -n 's/.*<release version="\([^"]*\)".*/\1/p' \
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.metainfo.xml \
  | head -1)"
[ "$crate_version" = "$meta_version" ]"#;

const RELEASE_STAMP_EVIDENCE: &[RepositoryEvidence] = &[
    RepositoryEvidence {
        path: "crates/trace-commons-contributor/Cargo.toml",
        contents: RELEASE_CORE_PACKAGE,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor-ffi/Cargo.toml",
        contents: RELEASE_FFI_PACKAGE,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor-gtk/Cargo.toml",
        contents: RELEASE_GTK_PACKAGE,
    },
    RepositoryEvidence {
        path: "crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.metainfo.xml",
        contents: RELEASE_METAINFO,
    },
    RepositoryEvidence {
        path: ".github/workflows/ci.yml",
        contents: RELEASE_VERSION_GATE,
    },
];

const RELEASE_STAMP_OUTPUTS: &[&str] = &[
    "Cargo.lock",
    "crates/trace-commons-contributor-gtk/Cargo.lock",
];
const RELEASE_STAMP_REGENERATION: &[&[&str]] = &[
    &["cargo metadata --format-version 1"],
    &[
        "cargo metadata --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --format-version 1",
    ],
];
const RELEASE_STAMP_VERIFICATION: &[&[&str]] = &[
    &["cargo test -p trace-commons-contributor --test release_pipeline"],
    &[
        "appstreamcli validate --no-net --pedantic crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.metainfo.xml",
    ],
];

pub(in crate::skill_loop::evaluation) const FIXTURES: &[EvaluationFixture] = &[
    EvaluationFixture {
        id: "mark-msix-scale-ladder",
        cluster: "mark-assets",
        source_url: MARK_SOURCE,
        snapshot_commit: MARK_COMMIT,
        task: "Change the Windows package tile background while keeping all three base assets and their four display-scale variants consistent.",
        evidence: MARK_EVIDENCE,
        required_edit_paths: &["crates/trace-commons-mark/src/lib.rs"],
        generated_output_paths: MARK_TILES,
        regeneration_steps: MARK_REGENERATION,
        verification_steps: MARK_VERIFICATION,
    },
    EvaluationFixture {
        id: "witness-compose-image-pin",
        cluster: "witness-compose",
        source_url: WITNESS_SOURCE,
        snapshot_commit: WITNESS_COMMIT,
        task: "Replace the witness image digest and keep the measured app-compose deployment document current.",
        evidence: WITNESS_EVIDENCE,
        required_edit_paths: &["deploy/witness/docker-compose.yml"],
        generated_output_paths: &["deploy/witness/app-compose.json"],
        regeneration_steps: WITNESS_REGENERATION,
        verification_steps: WITNESS_VERIFICATION,
    },
    EvaluationFixture {
        id: "flatpak-ironwire-pin",
        cluster: "flatpak-sources",
        source_url: FLATPAK_SOURCE,
        snapshot_commit: FLATPAK_COMMIT,
        task: "Advance the pinned IronWire proxy dependency to a reviewed revision and keep both lockfiles and the offline Flatpak vendor source set synchronized.",
        evidence: FLATPAK_EVIDENCE,
        required_edit_paths: &["crates/trace-commons-contributor/Cargo.toml"],
        generated_output_paths: &[
            "Cargo.lock",
            "crates/trace-commons-contributor-gtk/Cargo.lock",
            "crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json",
        ],
        regeneration_steps: FLATPAK_REGENERATION,
        verification_steps: FLATPAK_VERIFICATION,
    },
    EvaluationFixture {
        id: "winget-publisher",
        cluster: "winget-manifests",
        source_url: WINGET_SOURCE,
        snapshot_commit: WINGET_COMMIT,
        task: "For release 0.12.2, change the publisher shown in the generated WinGet manifests without allowing the installer, version, and locale documents to drift apart.",
        evidence: WINGET_EVIDENCE,
        required_edit_paths: &["scripts/winget/generate-manifests.sh"],
        generated_output_paths: WINGET_OUTPUTS,
        regeneration_steps: WINGET_REGENERATION,
        verification_steps: WINGET_VERIFICATION,
    },
    EvaluationFixture {
        id: "png-encoder-foreign-decoder",
        cluster: "png-encoding",
        source_url: PNG_COMPRESSION_SOURCE,
        snapshot_commit: PNG_COMPRESSION_COMMIT,
        task: "Reduce the generated Windows tile PNG size by changing the shared encoder, regenerate all committed base tiles, and verify the bytes with an independent decoder.",
        evidence: PNG_EVIDENCE,
        required_edit_paths: &["crates/trace-commons-mark/src/raster.rs"],
        generated_output_paths: PNG_OUTPUTS,
        regeneration_steps: PNG_REGENERATION,
        verification_steps: PNG_VERIFICATION,
    },
    EvaluationFixture {
        id: "update-conformance-platform",
        cluster: "update-conformance",
        source_url: UPDATE_FIXTURES_SOURCE,
        snapshot_commit: UPDATE_FIXTURES_COMMIT,
        task: "Change the artifact URL origin in the deterministic update-conformance generator and regenerate every signed manifest fixture without hand-editing its signatures.",
        evidence: UPDATE_FIXTURE_EVIDENCE,
        required_edit_paths: &["tests/fixtures/update-conformance/regenerate.sh"],
        generated_output_paths: UPDATE_FIXTURE_OUTPUTS,
        regeneration_steps: UPDATE_FIXTURE_REGENERATION,
        verification_steps: UPDATE_FIXTURE_VERIFICATION,
    },
    EvaluationFixture {
        id: "contributor-release-version-stamp",
        cluster: "release-versioning",
        source_url: RELEASE_STAMP_SOURCE,
        snapshot_commit: RELEASE_STAMP_COMMIT,
        task: "Prepare contributor release 0.13.0 by updating all three contributor package versions and the newest Flatpak release entry, then regenerate both Rust lockfiles.",
        evidence: RELEASE_STAMP_EVIDENCE,
        required_edit_paths: &[
            "crates/trace-commons-contributor/Cargo.toml",
            "crates/trace-commons-contributor-ffi/Cargo.toml",
            "crates/trace-commons-contributor-gtk/Cargo.toml",
            "crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.metainfo.xml",
        ],
        generated_output_paths: RELEASE_STAMP_OUTPUTS,
        regeneration_steps: RELEASE_STAMP_REGENERATION,
        verification_steps: RELEASE_STAMP_VERIFICATION,
    },
];
