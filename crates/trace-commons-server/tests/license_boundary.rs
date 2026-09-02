// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The licence boundary is a dependency-direction invariant, and nothing about
//! the code enforces it.
//!
//! `trace-commons-server`, `-gate-api`, and `-gate-enclave` are
//! AGPL-3.0-or-later. Every other crate is `MIT OR Apache-2.0` so it can be
//! embedded in proprietary software -- the contributor CLI, the desktop apps,
//! and the envelope protocol depend on that.
//!
//! Permissive code may flow into the AGPL crates. The reverse is a licence
//! violation that no compiler will report: adding `trace-commons-gate-api` to
//! the contributor CLI to reuse one trait would quietly make a shipped client
//! copyleft, and nobody would notice until someone read the manifests.
//!
//! So the invariant is checked here instead.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A workspace crate reduced to what the boundary check needs.
#[derive(Debug, Clone)]
struct Crate {
    name: String,
    license: String,
    /// Our own crates this one links in a shipped build: normal and build
    /// dependencies. These are what a licence boundary is actually about.
    deps: BTreeSet<String>,
    /// Our own crates this one pulls in only to run its tests.
    ///
    /// Dev-dependencies do not ship. The published artifact never links them,
    /// so a dev edge across the boundary conveys no combined work and creates
    /// no AGPL obligation. It is tracked separately rather than ignored,
    /// because a new one is still worth a human deciding about.
    dev_deps: BTreeSet<String>,
}

fn is_agpl(license: &str) -> bool {
    license.contains("AGPL")
}

/// Returns one message per violation: a non-AGPL crate that reaches an AGPL
/// crate, with the path that gets it there.
fn boundary_violations(crates: &[Crate]) -> Vec<String> {
    let by_name: BTreeMap<&str, &Crate> = crates.iter().map(|c| (c.name.as_str(), c)).collect();
    let mut violations = Vec::new();

    for start in crates {
        if is_agpl(&start.license) {
            continue;
        }
        // Breadth-first over our own crates only, carrying the path so the
        // failure names the edge to delete rather than just the endpoints.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: Vec<Vec<&str>> = vec![vec![start.name.as_str()]];
        while let Some(path) = queue.pop() {
            let current = *path.last().expect("path is never empty");
            let Some(node) = by_name.get(current) else {
                continue;
            };
            if is_agpl(&node.license) {
                violations.push(format!(
                    "{} is `{}` but reaches AGPL crate {} via {}",
                    start.name,
                    start.license,
                    current,
                    path.join(" -> ")
                ));
                continue;
            }
            for dep in &node.deps {
                if seen.insert(dep.as_str()) {
                    let mut next = path.clone();
                    next.push(dep.as_str());
                    queue.push(next);
                }
            }
        }
    }

    violations.sort();
    violations.dedup();
    violations
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/trace-commons-server.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two levels above the server crate")
        .to_path_buf()
}

/// `cargo metadata --no-deps` for one manifest, reduced to `Crate` values.
///
/// `--no-deps` is deliberate: we only care about edges between our own crates,
/// and resolving the full registry graph would make this test slow enough that
/// someone would delete it.
fn crates_for_manifest(manifest: &Path, our_crates: &BTreeSet<String>) -> Vec<Crate> {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .unwrap_or_else(|error| panic!("cargo metadata for {}: {error}", manifest.display()));
    assert!(
        output.status.success(),
        "cargo metadata for {} failed: {}",
        manifest.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON");

    metadata["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("package has a name");
            let license = package["license"]
                .as_str()
                .unwrap_or_else(|| panic!("{name} declares no license"))
                .to_string();
            let ours = |kind: Option<&str>| -> BTreeSet<String> {
                package["dependencies"]
                    .as_array()
                    .expect("package has dependencies")
                    .iter()
                    // `kind` is absent or null for a normal dependency, and
                    // "dev" or "build" otherwise.
                    .filter(|dep| dep["kind"].as_str() == kind)
                    .filter_map(|dep| dep["name"].as_str())
                    .filter(|dep| our_crates.contains(*dep))
                    .map(str::to_string)
                    .collect()
            };
            let mut deps = ours(None);
            deps.extend(ours(Some("build")));
            Crate {
                name: name.to_string(),
                license,
                deps,
                dev_deps: ours(Some("dev")),
            }
        })
        .collect()
}

/// Every crate in the tree, including the ones excluded from the workspace.
///
/// `trace-commons-contributor-gtk` is in the root manifest's `exclude` list, so
/// it does not appear in the workspace's `cargo metadata` output at all. It is
/// also a shipped client, which makes it exactly the crate a boundary check
/// must not miss. Any future `exclude` entry has to be added here too, which is
/// what the completeness assertion below is for.
fn all_crates() -> Vec<Crate> {
    let root = workspace_root();

    let mut our_crates = BTreeSet::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates/ is readable") {
        let entry = entry.expect("directory entry");
        if entry.path().join("Cargo.toml").is_file() {
            our_crates.insert(
                entry
                    .file_name()
                    .to_str()
                    .expect("crate directory name is UTF-8")
                    .to_string(),
            );
        }
    }

    let mut crates = crates_for_manifest(&root.join("Cargo.toml"), &our_crates);
    let covered: BTreeSet<String> = crates.iter().map(|c| c.name.clone()).collect();
    for missing in our_crates.difference(&covered) {
        crates.extend(crates_for_manifest(
            &root.join("crates").join(missing).join("Cargo.toml"),
            &our_crates,
        ));
    }

    let covered: BTreeSet<String> = crates.iter().map(|c| c.name.clone()).collect();
    assert!(
        our_crates.is_subset(&covered),
        "crates under crates/ that the boundary check never inspected: {:?}",
        our_crates.difference(&covered).collect::<Vec<_>>()
    );

    crates
}

#[test]
fn no_permissive_crate_depends_on_an_agpl_crate() {
    let crates = all_crates();
    let violations = boundary_violations(&crates);
    assert!(
        violations.is_empty(),
        "licence boundary violated. A crate published as MIT OR Apache-2.0 \
         cannot depend on an AGPL crate -- either drop the dependency or move \
         the crate across the boundary deliberately:\n  {}",
        violations.join("\n  ")
    );
}

/// The split is only meaningful if both sides are actually populated. If a
/// future refactor renames the gate crates or flattens them into the server,
/// this fails and someone re-reads LICENSE rather than discovering later that
/// the check had been vacuously passing.
#[test]
fn both_sides_of_the_boundary_are_populated() {
    let crates = all_crates();

    let mut agpl: Vec<&str> = crates
        .iter()
        .filter(|c| is_agpl(&c.license))
        .map(|c| c.name.as_str())
        .collect();
    agpl.sort();

    assert_eq!(
        agpl,
        [
            "trace-commons-gate-api",
            "trace-commons-gate-enclave",
            "trace-commons-server",
        ],
        "the set of AGPL crates changed; update LICENSE and this test together"
    );

    for permissive in [
        "trace-commons-protocol",
        "trace-commons-attestation",
        "trace-commons-contributor",
        "trace-commons-contributor-ffi",
        "trace-commons-contributor-gtk",
        "trace-commons-operator-client",
        "trace-commons-mark",
        "trace-commons-build-info",
    ] {
        let found = crates
            .iter()
            .find(|c| c.name == permissive)
            .unwrap_or_else(|| panic!("{permissive} is missing from the crate list"));
        assert_eq!(
            found.license, "MIT OR Apache-2.0",
            "{permissive} is a shipped client and must stay permissive"
        );
    }
}

/// The check above passes today. This one proves it would fail if the boundary
/// were actually crossed, so a green run means something.
#[test]
fn a_permissive_crate_reaching_an_agpl_crate_is_caught() {
    let permissive = |name: &str, deps: &[&str]| Crate {
        name: name.to_string(),
        license: "MIT OR Apache-2.0".to_string(),
        deps: deps.iter().map(|d| d.to_string()).collect(),
        dev_deps: BTreeSet::new(),
    };

    // Direct edge.
    let direct = vec![
        permissive("client", &["gate-api"]),
        Crate {
            name: "gate-api".to_string(),
            license: "AGPL-3.0-or-later".to_string(),
            deps: BTreeSet::new(),
            dev_deps: BTreeSet::new(),
        },
    ];
    let found = boundary_violations(&direct);
    assert_eq!(found.len(), 1, "expected one violation, got {found:?}");
    assert!(found[0].contains("client -> gate-api"), "{found:?}");

    // Transitive edge, which is the one a reviewer would miss.
    let transitive = vec![
        permissive("client", &["shared"]),
        permissive("shared", &["gate-api"]),
        Crate {
            name: "gate-api".to_string(),
            license: "AGPL-3.0-or-later".to_string(),
            deps: BTreeSet::new(),
            dev_deps: BTreeSet::new(),
        },
    ];
    let found = boundary_violations(&transitive);
    assert_eq!(found.len(), 2, "expected two violations, got {found:?}");
    assert!(
        found
            .iter()
            .any(|v| v.contains("client -> shared -> gate-api")),
        "the transitive path must be reported: {found:?}"
    );

    // And the legal direction stays legal.
    let allowed = vec![
        Crate {
            name: "server".to_string(),
            license: "AGPL-3.0-or-later".to_string(),
            deps: ["protocol".to_string()].into_iter().collect(),
            dev_deps: BTreeSet::new(),
        },
        permissive("protocol", &[]),
    ];
    assert!(
        boundary_violations(&allowed).is_empty(),
        "permissive into AGPL is permitted and must not be flagged"
    );
}

/// Dev-dependency edges that cross the boundary.
///
/// These are allowed -- they never reach a shipped artifact -- but pinning the
/// exact set means a new one has to be added here on purpose, by someone who
/// has read why the previous ones were acceptable. `trace-commons-contributor`
/// takes this one under `cfg(not(windows))` for three cross-check tests in
/// `account_auth.rs` and the `e2e_enroll_and_submit` integration test; its own
/// shipped code never touches the server crate.
#[test]
fn dev_dependency_edges_across_the_boundary_are_the_known_ones() {
    let crates = all_crates();
    let agpl: BTreeSet<&str> = crates
        .iter()
        .filter(|c| is_agpl(&c.license))
        .map(|c| c.name.as_str())
        .collect();

    let mut crossing: Vec<String> = crates
        .iter()
        .filter(|c| !is_agpl(&c.license))
        .flat_map(|c| {
            c.dev_deps
                .iter()
                .filter(|dep| agpl.contains(dep.as_str()))
                .map(move |dep| format!("{} (dev) -> {dep}", c.name))
        })
        .collect();
    crossing.sort();

    assert_eq!(
        crossing,
        ["trace-commons-contributor (dev) -> trace-commons-server"],
        "a dev-dependency edge across the licence boundary changed. This does \
         not violate the licence -- dev-dependencies do not ship -- but confirm \
         the crate's shipped code still never links the AGPL crate, then update \
         this list."
    );
}
