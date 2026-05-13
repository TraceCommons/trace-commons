use std::path::PathBuf;

#[path = "../src/bin/gate_calibrate/bakeoff_manifest.rs"]
mod bakeoff_manifest;
use bakeoff_manifest::{parse_manifest_str, CandidateLicense};

#[test]
fn parses_minimal_two_candidate_manifest() {
    let raw = r#"
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"

[[candidate]]
id = "qwen3-8b-base"
path = "/srv/models/qwen3-8b-base"
arch = "qwen2"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert_eq!(manifest.candidates.len(), 2);
    assert_eq!(manifest.candidates[0].id, "llama-3.1-8b-instruct");
    assert_eq!(
        manifest.candidates[0].path,
        PathBuf::from("/srv/models/llama-3.1-8b-instruct")
    );
    assert_eq!(manifest.candidates[1].license, CandidateLicense::Apache2);
}

#[test]
fn rejects_duplicate_candidate_id() {
    let raw = r#"
[[candidate]]
id = "x"
path = "/a"
arch = "llama"
license = "apache-2.0"

[[candidate]]
id = "x"
path = "/b"
arch = "llama"
license = "apache-2.0"
"#;
    let err = parse_manifest_str(raw).unwrap_err();
    assert!(err.to_string().contains("duplicate candidate id"));
}

#[test]
fn rejects_empty_manifest() {
    let err = parse_manifest_str("").unwrap_err();
    assert!(err
        .to_string()
        .contains("manifest must contain at least one candidate"));
}

#[test]
fn warns_on_non_apache_non_mit_license_for_non_incumbent() {
    let raw = r#"
[[candidate]]
id = "some-other-llama-derivative"
path = "/srv/models/x"
arch = "llama"
license = "llama-community"
"#;
    let manifest = parse_manifest_str(raw).expect("parses but warns");
    let warnings = manifest.warnings();
    assert!(
        warnings.iter().any(|w| w.contains("license")),
        "warnings: {warnings:?}"
    );
}

#[test]
fn no_warning_for_incumbent_llama_community() {
    let raw = r#"
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"
"#;
    let manifest = parse_manifest_str(raw).expect("parses clean");
    assert!(manifest.warnings().is_empty());
}
