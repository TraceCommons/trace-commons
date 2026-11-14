// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Unsafe-code boundary across every first-party Rust crate in the repository.
//!
//! Like `license_boundary.rs`, this check inspects the whole Rust tree
//! (root workspace, excluded `crates/trace-commons-contributor-gtk`, and the
//! separate `tauri-desktop/src-tauri` workspace) so a new crate cannot omit
//! `forbid(unsafe_code)` by accident and an audited crate cannot silently
//! weaken its lint posture.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Tier 1: zero `unsafe` anywhere in source or tests.
const FORBID_CRATES: &[&str] = &[
    "trace-commons-attestation",
    "trace-commons-build-info",
    "trace-commons-client-server-tests",
    "trace-commons-contributor-gtk",
    "trace-commons-contributor-sources",
    "trace-commons-gate-api",
    "trace-commons-gate-enclave",
    "trace-commons-mark",
];

/// Tier 2: zero `unsafe` in shipped targets (`#![cfg_attr(not(test), forbid(unsafe_code))]`
/// or `#![forbid(unsafe_code)]` on every `lib`/`bin` root), with `unsafe` permitted
/// only on mutex-serialized `std::env::{set_var, remove_var}` unit tests.
const TEST_ONLY_ENV_CRATES: &[&str] = &[
    "trace-commons-ingest",
    "trace-commons-operator-client",
    "trace-commons-protocol",
    "trace-commons-server",
];

/// Tier 3: crates with audited FFI, OS handle/ACL/keychain, or volatile-wipe `unsafe`.
const AUDITED_FFI_PLATFORM_CRATES: &[&str] = &[
    "trace-commons-contributor",
    "trace-commons-contributor-ffi",
    "trace-commons-contributor-insights",
    "trace-commons-contributor-platform",
    "trace-commons-tauri-desktop",
];

#[derive(Debug, Clone)]
struct TargetInfo {
    name: String,
    kinds: Vec<String>,
    src_path: PathBuf,
}

#[derive(Debug, Clone)]
struct CratePolicyInput {
    name: String,
    manifest_text: String,
    targets: Vec<(String, Vec<String>, String)>,
    rs_files: BTreeMap<String, String>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two levels above the server crate")
        .to_path_buf()
}

fn collect_rs_files(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if path.is_dir() {
            if matches!(name.as_ref(), "target" | "vendor" | "node_modules" | ".git") {
                continue;
            }
            collect_rs_files(&path, root, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            out.insert(rel, contents);
        }
    }
}

fn packages_for_manifest(
    manifest: &Path,
    our_crates: &BTreeSet<String>,
) -> Vec<(String, PathBuf, Vec<TargetInfo>)> {
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
        .filter_map(|pkg| {
            let name = pkg["name"].as_str().expect("package has a name");
            if !our_crates.contains(name) && name != "trace-commons-tauri-desktop" {
                return None;
            }
            let manifest_path = PathBuf::from(
                pkg["manifest_path"]
                    .as_str()
                    .expect("package has manifest_path"),
            );
            let targets = pkg["targets"]
                .as_array()
                .expect("package has targets")
                .iter()
                .map(|t| TargetInfo {
                    name: t["name"].as_str().expect("target name").to_string(),
                    kinds: t["kind"]
                        .as_array()
                        .expect("target kind")
                        .iter()
                        .filter_map(|k| k.as_str().map(str::to_string))
                        .collect(),
                    src_path: PathBuf::from(t["src_path"].as_str().expect("target src_path")),
                })
                .collect();
            Some((name.to_string(), manifest_path, targets))
        })
        .collect()
}

fn all_crate_inputs() -> Vec<CratePolicyInput> {
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

    let mut raw = packages_for_manifest(&root.join("Cargo.toml"), &our_crates);
    let covered: BTreeSet<String> = raw.iter().map(|(n, _, _)| n.clone()).collect();
    for missing in our_crates.difference(&covered) {
        raw.extend(packages_for_manifest(
            &root.join("crates").join(missing).join("Cargo.toml"),
            &our_crates,
        ));
    }

    let covered: BTreeSet<String> = raw.iter().map(|(n, _, _)| n.clone()).collect();
    assert!(
        our_crates.is_subset(&covered),
        "crates under crates/ that unsafe_boundary never inspected: {:?}",
        our_crates.difference(&covered).collect::<Vec<_>>()
    );

    raw.extend(packages_for_manifest(
        &root.join("tauri-desktop/src-tauri/Cargo.toml"),
        &our_crates,
    ));

    raw.into_iter()
        .map(|(name, manifest_path, targets)| {
            let crate_dir = manifest_path.parent().expect("manifest parent");
            let manifest_text = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", manifest_path.display()));
            let mut rs_files = BTreeMap::new();
            collect_rs_files(crate_dir, crate_dir, &mut rs_files);
            let target_entries = targets
                .into_iter()
                .map(|t| {
                    let src_contents = std::fs::read_to_string(&t.src_path)
                        .unwrap_or_else(|e| panic!("reading {}: {e}", t.src_path.display()));
                    (t.name, t.kinds, src_contents)
                })
                .collect();
            CratePolicyInput {
                name,
                manifest_text,
                targets: target_entries,
                rs_files,
            }
        })
        .collect()
}

/// Extract key/value assignments inside a `[section]` header of a Cargo.toml
/// without pulling in a TOML parser dependency.
fn section_entries<'a>(manifest: &'a str, section: &str) -> BTreeMap<&'a str, &'a str> {
    let header = format!("[{section}]");
    let mut in_section = false;
    let mut out = BTreeMap::new();
    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_section = line == header;
            continue;
        }
        if in_section && let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim(), v.trim().trim_matches('"'));
        }
    }
    out
}

/// The code part of a line that is an attribute (`#[...]` or `#![...]`),
/// with any trailing `//` comment removed. String literals that merely
/// mention an attribute (such as the messages in this file) never start a
/// line with `#`, so they are not mistaken for one.
fn attribute_code(line: &str) -> Option<&str> {
    let code = line.split("//").next().unwrap_or("").trim();
    (code.starts_with("#[") || code.starts_with("#![")).then_some(code)
}

fn overrides_unsafe_code(code: &str) -> bool {
    (code.contains("allow(") || code.contains("expect(")) && code.contains("unsafe_code")
}

fn contains_unsafe_code_override(source: &str) -> bool {
    source
        .lines()
        .filter_map(attribute_code)
        .any(overrides_unsafe_code)
}

fn contains_inner_allow_unsafe_code(source: &str) -> bool {
    source
        .lines()
        .filter_map(attribute_code)
        .any(|code| code.starts_with("#![") && overrides_unsafe_code(code))
}

fn has_shipped_forbid_attr(source: &str) -> bool {
    source.lines().any(|line| {
        let code = line.split("//").next().unwrap_or("").trim();
        code == "#![forbid(unsafe_code)]" || code == "#![cfg_attr(not(test), forbid(unsafe_code))]"
    })
}

fn policy_violations(crates: &[CratePolicyInput]) -> Vec<String> {
    let forbid_set: BTreeSet<&str> = FORBID_CRATES.iter().copied().collect();
    let test_only_set: BTreeSet<&str> = TEST_ONLY_ENV_CRATES.iter().copied().collect();
    let audited_set: BTreeSet<&str> = AUDITED_FFI_PLATFORM_CRATES.iter().copied().collect();

    let mut violations = Vec::new();

    for krate in crates {
        let name = krate.name.as_str();
        let rust_lints = section_entries(&krate.manifest_text, "lints.rust");
        let clippy_lints = section_entries(&krate.manifest_text, "lints.clippy");

        // Every custom-build script (`build.rs`) across every tier must forbid unsafe_code.
        for (target_name, kinds, src) in &krate.targets {
            if kinds.iter().any(|k| k == "custom-build") {
                if !src
                    .lines()
                    .any(|l| l.split("//").next().unwrap_or("").trim() == "#![forbid(unsafe_code)]")
                {
                    violations.push(format!(
                        "{name} custom-build target `{target_name}` must declare `#![forbid(unsafe_code)]`"
                    ));
                }
                if contains_unsafe_code_override(src) {
                    violations.push(format!(
                        "{name} custom-build target `{target_name}` must not allow or expect `unsafe_code`"
                    ));
                }
            }
        }

        if forbid_set.contains(name) {
            if rust_lints.get("unsafe_code") != Some(&"forbid") {
                violations.push(format!(
                    "{name} is in FORBID_CRATES and must set `[lints.rust] unsafe_code = \"forbid\"`"
                ));
            }
            for (rel_path, src) in &krate.rs_files {
                if contains_unsafe_code_override(src) {
                    violations.push(format!(
                        "{name} ({rel_path}) is in FORBID_CRATES and must not contain allow/expect(unsafe_code)"
                    ));
                }
            }
        } else if test_only_set.contains(name) {
            if rust_lints.get("unsafe_code") != Some(&"deny") {
                violations.push(format!(
                    "{name} is in TEST_ONLY_ENV_CRATES and must set `[lints.rust] unsafe_code = \"deny\"`"
                ));
            }
            if rust_lints.get("unsafe_op_in_unsafe_fn") != Some(&"forbid") {
                violations.push(format!(
                    "{name} is in TEST_ONLY_ENV_CRATES and must set `[lints.rust] unsafe_op_in_unsafe_fn = \"forbid\"`"
                ));
            }
            if clippy_lints.get("undocumented_unsafe_blocks") != Some(&"deny") {
                violations.push(format!(
                    "{name} is in TEST_ONLY_ENV_CRATES and must set `[lints.clippy] undocumented_unsafe_blocks = \"deny\"`"
                ));
            }
            for (target_name, kinds, src) in &krate.targets {
                let is_shipped = kinds
                    .iter()
                    .any(|k| matches!(k.as_str(), "lib" | "rlib" | "cdylib" | "staticlib" | "bin"));
                if is_shipped && !has_shipped_forbid_attr(src) {
                    violations.push(format!(
                        "{name} shipped target `{target_name}` must declare `#![forbid(unsafe_code)]` or `#![cfg_attr(not(test), forbid(unsafe_code))]`"
                    ));
                }
            }
            for (rel_path, src) in &krate.rs_files {
                if contains_inner_allow_unsafe_code(src) {
                    violations.push(format!(
                        "{name} ({rel_path}) must not use inner `#![allow(unsafe_code)]` in TEST_ONLY_ENV_CRATES"
                    ));
                }
                if rel_path.starts_with("tests/") && contains_unsafe_code_override(src) {
                    violations.push(format!(
                        "{name} ({rel_path}) integration tests must not allow `unsafe_code`"
                    ));
                }
            }
        } else if audited_set.contains(name) {
            if rust_lints.get("unsafe_code") != Some(&"deny") {
                violations.push(format!(
                    "{name} is in AUDITED_FFI_PLATFORM_CRATES and must set `[lints.rust] unsafe_code = \"deny\"`"
                ));
            }
            if rust_lints.get("unsafe_op_in_unsafe_fn") != Some(&"forbid") {
                violations.push(format!(
                    "{name} is in AUDITED_FFI_PLATFORM_CRATES and must set `[lints.rust] unsafe_op_in_unsafe_fn = \"forbid\"`"
                ));
            }
            // Dedicated C ABI wrapper crate `trace-commons-contributor-ffi` enforces safety
            // docs at every `pub unsafe extern "C" fn` export; all other Tier 3 crates also
            // deny undocumented inner `unsafe` blocks via Clippy.
            if name != "trace-commons-contributor-ffi"
                && clippy_lints.get("undocumented_unsafe_blocks") != Some(&"deny")
            {
                violations.push(format!(
                    "{name} is in AUDITED_FFI_PLATFORM_CRATES and must set `[lints.clippy] undocumented_unsafe_blocks = \"deny\"`"
                ));
            }
        } else {
            violations.push(format!(
                "{name} is unclassified in unsafe_boundary.rs; default new crates to `[lints.rust] unsafe_code = \"forbid\"` and add to FORBID_CRATES"
            ));
        }
    }

    let seen: BTreeSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    for expected in FORBID_CRATES
        .iter()
        .chain(TEST_ONLY_ENV_CRATES.iter())
        .chain(AUDITED_FFI_PLATFORM_CRATES.iter())
    {
        if !seen.contains(expected) {
            violations.push(format!(
                "pinned crate `{expected}` in unsafe_boundary.rs was not found in the workspace tree"
            ));
        }
    }

    violations
}

#[test]
fn every_first_party_crate_enforces_its_unsafe_code_tier() {
    let crates = all_crate_inputs();
    let violations = policy_violations(&crates);
    assert!(
        violations.is_empty(),
        "unsafe-code boundary violated:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn tiers_are_disjoint_and_cover_all_seventeen_crates() {
    let mut all = BTreeSet::new();
    for name in FORBID_CRATES
        .iter()
        .chain(TEST_ONLY_ENV_CRATES.iter())
        .chain(AUDITED_FFI_PLATFORM_CRATES.iter())
    {
        assert!(
            all.insert(*name),
            "crate `{name}` appears in more than one unsafe_boundary tier"
        );
    }
    assert_eq!(all.len(), 17, "expected 17 first-party Rust crates");
}

#[test]
fn unclassified_or_weakened_crates_are_caught() {
    let baseline = all_crate_inputs();
    assert!(policy_violations(&baseline).is_empty());

    // 1. A newly added crate that omits classification fails closed.
    let mut with_unclassified = baseline.clone();
    with_unclassified.push(CratePolicyInput {
        name: "trace-commons-new-unclassified".to_string(),
        manifest_text: "[package]\nname = \"trace-commons-new-unclassified\"\n".to_string(),
        targets: vec![(
            "trace_commons_new_unclassified".to_string(),
            vec!["lib".to_string()],
            "pub fn ok() {}\n".to_string(),
        )],
        rs_files: BTreeMap::new(),
    });
    let found = policy_violations(&with_unclassified);
    assert!(
        found.iter().any(|v| v.contains("unclassified")),
        "expected unclassified crate violation, got {found:?}"
    );

    // 2. Weakening a FORBID_CRATES member from `forbid` to `deny` or sneaking in `allow(unsafe_code)` fails.
    let mut weakened_tier1 = baseline.clone();
    let mark = weakened_tier1
        .iter_mut()
        .find(|c| c.name == "trace-commons-mark")
        .expect("trace-commons-mark present");
    mark.manifest_text = mark
        .manifest_text
        .replace("unsafe_code = \"forbid\"", "unsafe_code = \"deny\"");
    mark.rs_files.insert(
        "src/sneaky.rs".to_string(),
        "#[allow(unsafe_code)]\npub fn bad() {}\n".to_string(),
    );
    let found = policy_violations(&weakened_tier1);
    assert_eq!(found.len(), 2, "expected 2 violations, got {found:?}");

    // 3. Dropping `#![cfg_attr(not(test), forbid(unsafe_code))]` on a Tier 2 shipped target fails.
    let mut weakened_tier2 = baseline.clone();
    let proto = weakened_tier2
        .iter_mut()
        .find(|c| c.name == "trace-commons-protocol")
        .expect("trace-commons-protocol present");
    proto.targets[0].2 = "pub fn missing_forbid() {}\n".to_string();
    let found = policy_violations(&weakened_tier2);
    assert!(
        found
            .iter()
            .any(|v| v.contains("shipped target") && v.contains("forbid(unsafe_code)")),
        "expected Tier 2 shipped-target violation, got {found:?}"
    );

    // 4. Removing `#![forbid(unsafe_code)]` from a build.rs fails.
    let mut weakened_build_rs = baseline;
    let build_info = weakened_build_rs
        .iter_mut()
        .find(|c| c.name == "trace-commons-build-info")
        .expect("trace-commons-build-info present");
    for (_, kinds, src) in &mut build_info.targets {
        if kinds.iter().any(|k| k == "custom-build") {
            *src = "fn main() {}\n".to_string();
        }
    }
    let found = policy_violations(&weakened_build_rs);
    assert!(
        found.iter().any(|v| v.contains("custom-build")),
        "expected custom-build violation, got {found:?}"
    );
}
