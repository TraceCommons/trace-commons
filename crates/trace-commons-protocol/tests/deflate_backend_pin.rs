//! Pins the deflate backend behind `lef1`'s `deflate_ratio_milli` to one
//! version across every workspace that builds a daemon.
//!
//! The ratio is part of the feature definition a published estimate table is
//! fitted against, and `deflated_sizes_are_pinned_for_fixed_inputs` guards it,
//! but only under this workspace's `Cargo.lock`: the GTK shell and the Tauri
//! app are separate workspaces with their own lockfiles, and neither runs
//! this crate's tests. A different `flate2` or `miniz_oxide` there, or a C or
//! `zlib-rs` backend switched on by feature unification, could give the
//! daemon different ratios with no job failing. So this checks the three
//! lockfiles agree on the backend, from the root workspace where it runs.
//!
//! If it fails after a dependency update, move all three lockfiles to the same
//! `flate2` and `miniz_oxide` (`cargo update -p flate2 --precise <v>` with each
//! `--manifest-path`), then rerun `deflated_sizes_are_pinned_for_fixed_inputs`.
//! Kristi's #1285 re-review, item 1.

use std::collections::BTreeSet;
use std::path::Path;

/// The lockfile of every workspace that builds the contributor daemon,
/// relative to the repository root.
const LOCKFILES: [&str; 3] = [
    "Cargo.lock",
    "crates/trace-commons-contributor-gtk/Cargo.lock",
    "tauri-desktop/Cargo.lock",
];

/// Backends `flate2` can be built over other than `miniz_oxide`. None may be
/// a dependency of `flate2` in any lockfile.
const OTHER_BACKENDS: [&str; 4] = ["zlib-rs", "libz-sys", "libz-ng-sys", "libz-rs-sys"];

/// One `[[package]]` entry: name, version, checksum, dependency names.
struct Package {
    name: String,
    version: String,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

fn quoted(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim().strip_prefix('=')?.trim();
    Some(rest.trim_matches('"').to_string())
}

fn packages(lock: &str) -> Vec<Package> {
    lock.split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            let mut name = None;
            let mut version = None;
            let mut checksum = None;
            let mut dependencies = Vec::new();
            let mut in_dependencies = false;
            for line in block.lines() {
                let line = line.trim();
                if in_dependencies {
                    if line == "]" {
                        in_dependencies = false;
                    } else if let Some(dep) = line.strip_prefix('"') {
                        // "name", or "name version" when two versions resolve.
                        let dep = dep.trim_end_matches(',').trim_end_matches('"');
                        let dep_name = dep.split(' ').next().unwrap_or(dep);
                        dependencies.push(dep_name.to_string());
                    }
                    continue;
                }
                if line.starts_with("dependencies") {
                    in_dependencies = true;
                } else if let Some(v) = quoted(line, "name") {
                    name = Some(v);
                } else if let Some(v) = quoted(line, "version") {
                    version = Some(v);
                } else if let Some(v) = quoted(line, "checksum") {
                    checksum = Some(v);
                }
            }
            Some(Package {
                name: name?,
                version: version?,
                checksum,
                dependencies,
            })
        })
        .collect()
}

/// `(name, version, checksum)` of `flate2` and `miniz_oxide` in one lockfile,
/// refusing a second resolved version of either or another backend.
fn deflate_backend(path: &str, lock: &str) -> BTreeSet<(String, String, Option<String>)> {
    let packages = packages(lock);
    let mut found = BTreeSet::new();
    for name in ["flate2", "miniz_oxide"] {
        let versions: Vec<&Package> = packages.iter().filter(|p| p.name == name).collect();
        assert_eq!(
            versions.len(),
            1,
            "{path} resolves {} versions of {name}",
            versions.len()
        );
        let p = versions[0];
        found.insert((p.name.clone(), p.version.clone(), p.checksum.clone()));
    }
    let flate2 = packages
        .iter()
        .find(|p| p.name == "flate2")
        .expect("flate2");
    for backend in OTHER_BACKENDS {
        assert!(
            !flate2.dependencies.iter().any(|d| d == backend),
            "{path}: flate2 depends on {backend}"
        );
    }
    found
}

#[test]
fn every_daemon_workspace_resolves_the_same_deflate_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let backends: Vec<_> = LOCKFILES
        .iter()
        .map(|path| {
            let lock = std::fs::read_to_string(root.join(path))
                .unwrap_or_else(|e| panic!("reading {path}: {e}"));
            (*path, deflate_backend(path, &lock))
        })
        .collect();
    for (path, backend) in &backends[1..] {
        assert_eq!(
            backend, &backends[0].1,
            "{path} resolves a different deflate backend from {}",
            backends[0].0
        );
    }
}

#[test]
fn a_second_backend_or_version_is_refused() {
    let lock = r#"
[[package]]
name = "flate2"
version = "1.1.10"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aa"
dependencies = [
 "crc32fast",
 "miniz_oxide 0.9.1",
 "zlib-rs",
]

[[package]]
name = "miniz_oxide"
version = "0.9.1"
checksum = "bb"
"#;
    let result = std::panic::catch_unwind(|| deflate_backend("fixture", lock));
    assert!(result.is_err(), "zlib-rs under flate2 is refused");

    let two_versions = format!(
        "{lock}\n[[package]]\nname = \"miniz_oxide\"\nversion = \"0.8.9\"\nchecksum = \"cc\"\n"
    )
    .replace(" \"zlib-rs\",\n", "");
    let result = std::panic::catch_unwind(|| deflate_backend("fixture", &two_versions));
    assert!(result.is_err(), "two miniz_oxide versions are refused");
}
