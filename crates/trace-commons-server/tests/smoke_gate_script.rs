// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Keeps `scripts/operator/smoke-gate.sh` honest against the drill handlers.
//!
//! The script is the operator's pre-promotion gate and nothing executes it in
//! CI, so it rotted silently: it asserted a `success` field no drill response
//! has ever carried, and it POSTed body-less requests to handlers whose
//! `Json<T>` extractor is required. These checks are text-level -- they do not
//! run the script or the server -- but they tie the script's field names and
//! request bodies to the structs in `trace-commons-ingest.rs`, so the same
//! drift fails a build instead of a promotion.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/trace-commons-server.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// `object-primary-read` -> `ObjectPrimaryRead`.
fn drill_camel_case(drill: &str) -> String {
    drill
        .split('-')
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// The `REQUIRED_DRILLS=( ... )` array, in script order.
fn required_drills(script: &str) -> Vec<String> {
    let start = script
        .find("REQUIRED_DRILLS=(")
        .expect("smoke-gate.sh declares REQUIRED_DRILLS");
    let body = &script[start + "REQUIRED_DRILLS=(".len()..];
    let end = body.find(')').expect("REQUIRED_DRILLS array is closed");
    body[..end].split_whitespace().map(str::to_string).collect()
}

/// The body of `struct <name> { ... }`, without the braces.
fn struct_body<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("struct {name} {{\n");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("trace-commons-ingest.rs defines struct {name}"))
        + needle.len();
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("struct {name} is closed"));
    &rest[..end]
}

/// Field names in a struct body that carry no `#[serde(default...)]`, i.e.
/// the ones a request body must supply.
fn required_field_names(body: &str) -> Vec<String> {
    let mut required = Vec::new();
    let mut defaulted = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[serde(") {
            if trimmed.contains("default") {
                defaulted = true;
            }
            continue;
        }
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once(':') {
            if !defaulted {
                required.push(name.trim().to_string());
            }
        }
        defaulted = false;
    }
    required
}

/// The `<drill>)` arm of the script's `drill_body` case, if it has one.
fn drill_body_arm<'a>(script: &'a str, drill: &str) -> Option<&'a str> {
    let start = script.find("drill_body() {")?;
    let case = &script[start..];
    let end = case.find("\n  esac").expect("drill_body case is closed");
    let case = &case[..end];
    let arm_start = case.find(&format!("\n    {drill})\n"))?;
    let arm = &case[arm_start..];
    let arm_end = arm.find(";;").expect("case arm is terminated");
    Some(&arm[..arm_end])
}

#[test]
fn smoke_gate_drills_are_routed_and_report_ready() {
    let root = repo_root();
    let script = read(&root.join("scripts/operator/smoke-gate.sh"));
    let source = read(&root.join("crates/trace-commons-server/src/bin/trace-commons-ingest.rs"));

    let drills = required_drills(&script);
    assert_eq!(
        drills.len(),
        15,
        "REQUIRED_DRILLS changed size; update the drill docs alongside it"
    );

    for drill in &drills {
        let route = format!("\"/v1/admin/{drill}-drill\"");
        assert!(
            source.contains(&route),
            "smoke-gate.sh calls {drill}-drill but no such route is registered"
        );

        // The script's readiness check is `jq -r '.ready // false'`. Every
        // drill response must therefore actually carry `ready`.
        let response = format!("Trace{}DrillResponse", drill_camel_case(drill));
        let body = struct_body(&source, &response);
        assert!(
            body.lines().any(|line| line.trim() == "ready: bool,"),
            "{response} has no `ready: bool`; smoke-gate.sh reads `.ready`"
        );
    }

    assert!(
        script.contains("jq -r '.ready // false'"),
        "smoke-gate.sh no longer reads the drill readiness field"
    );
    assert!(
        !script.contains(".success"),
        "smoke-gate.sh reads `.success`; no drill response carries that field"
    );
}

#[test]
fn smoke_gate_sends_every_required_drill_request_field() {
    let root = repo_root();
    let script = read(&root.join("scripts/operator/smoke-gate.sh"));
    let source = read(&root.join("crates/trace-commons-server/src/bin/trace-commons-ingest.rs"));

    for drill in required_drills(&script) {
        let request = format!("Trace{}DrillRequest", drill_camel_case(&drill));
        let required = required_field_names(struct_body(&source, &request));
        let arm = drill_body_arm(&script, &drill);

        if required.is_empty() {
            // No required field: the default `{}` body is enough.
            continue;
        }

        let arm = arm.unwrap_or_else(|| {
            panic!(
                "{request} requires {required:?} but smoke-gate.sh sends the default `{{}}` body \
                 for {drill}"
            )
        });
        for field in required {
            assert!(
                arm.contains(&format!("{field}:")),
                "smoke-gate.sh does not send required field `{field}` to {drill}-drill"
            );
        }
    }
}
