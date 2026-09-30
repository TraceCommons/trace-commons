// Copyright 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `deploy/witness/app-compose.json` is generated from
//! `deploy/witness/docker-compose.yml` by `build-app-compose.sh`, which embeds
//! the compose file verbatim as a JSON string. Two copies of the same text,
//! and nothing in the build kept them equal: between #604 and #681 five
//! commits edited the compose without regenerating the manifest, and
//! `build-app-compose.sh --check` sat red on `main` for days (#760). A check
//! nobody runs is not a check.
//!
//! This asserts the one field that can drift. Every other field the generator
//! writes is a literal in the script, so a divergence there means the script
//! and the manifest were edited apart -- which the byte comparison at the end
//! would also catch, if it could be made without `jq`. The embedded compose is
//! where real drift lands, and it is the field with consequences: the digest
//! of the image to deploy lives in it.
//!
//! WHAT THIS DOES NOT ASSERT, and must not be read as asserting: that either
//! file describes the running CVM. `phala deploy` never reads
//! `app-compose.json` -- it takes the compose file and builds its own manifest
//! -- so the committed manifest is a record of intent, and the only
//! authoritative reading of a deployment is `phala cvms get <cvm-id> --json`.
//! See "The manifest we write is not the manifest that deploys" in
//! `deploy/witness/README.md`.

use std::path::PathBuf;

fn deploy_witness_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/trace-commons-server.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("deploy/witness")
}

#[test]
fn witness_manifest_embeds_the_current_compose_file() {
    let dir = deploy_witness_dir();
    let compose_path = dir.join("docker-compose.yml");
    let manifest_path = dir.join("app-compose.json");

    let compose = std::fs::read_to_string(&compose_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", compose_path.display()));
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", manifest_path.display()));

    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("app-compose.json is not valid JSON");
    let embedded = manifest
        .get("docker_compose_file")
        .and_then(|v| v.as_str())
        .expect("app-compose.json has no string `docker_compose_file`");

    if embedded == compose {
        return;
    }

    // Report the first differing line rather than two multi-kilobyte blobs.
    let mismatch = embedded
        .lines()
        .zip(compose.lines())
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| {
            format!(
                "first difference at line {}:\n  manifest: {a}\n  compose:  {b}",
                i + 1
            )
        })
        .unwrap_or_else(|| {
            format!(
                "one is a prefix of the other: manifest has {} lines, compose has {}",
                embedded.lines().count(),
                compose.lines().count()
            )
        });

    panic!(
        "deploy/witness/app-compose.json does not embed the current \
         docker-compose.yml.\n\n{mismatch}\n\n\
         Fix: run deploy/witness/build-app-compose.sh and commit the result.\n\n\
         This is a bookkeeping failure between two files in this repository. \
         Regenerating CANNOT move a deployed measurement -- phala deploy never \
         reads app-compose.json. Whether the deployment itself is current is a \
         separate question, answered only by `phala cvms get <cvm-id> --json`."
    );
}

/// The generator writes `manifest_version`, `name` and `runner` as literals,
/// and the deploy documentation quotes them. A silent change to any of the
/// three would make the committed manifest describe a different application.
#[test]
fn witness_manifest_keeps_its_identifying_fields() {
    let manifest_text = std::fs::read_to_string(deploy_witness_dir().join("app-compose.json"))
        .expect("reading app-compose.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("app-compose.json is not valid JSON");

    assert_eq!(manifest["manifest_version"], serde_json::json!(2));
    assert_eq!(manifest["name"], serde_json::json!("trace-commons-witness"));
    assert_eq!(manifest["runner"], serde_json::json!("docker-compose"));
}
