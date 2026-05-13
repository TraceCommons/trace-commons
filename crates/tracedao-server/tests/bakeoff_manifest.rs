use std::path::PathBuf;

#[path = "../src/bin/gate_calibrate/bakeoff_manifest.rs"]
mod bakeoff_manifest;
use bakeoff_manifest::{parse_manifest_str, CandidateArch, CandidateLicense};

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
fn parses_qwen3_arch() {
    let raw = r#"
[[candidate]]
id = "qwen3-8b-base"
path = "/srv/q3"
arch = "qwen3"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Qwen3));
    assert!(manifest.warnings().is_empty());
}

#[test]
fn parses_gemma4_arch() {
    let raw = r#"
[[candidate]]
id = "gemma-4-31b"
path = "/srv/g4"
arch = "gemma4"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Gemma4));
    assert!(manifest.warnings().is_empty());
}

#[test]
fn qwen2_alias_warns_and_resolves_to_qwen3() {
    let raw = r#"
[[candidate]]
id = "qwen3-8b-base"
path = "/srv/q3"
arch = "qwen2"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Qwen2));
    let warnings = manifest.warnings();
    assert!(
        warnings.iter().any(|w| w.contains("deprecated arch=qwen2")),
        "warnings: {warnings:?}"
    );
}

#[test]
fn parses_qwen3_5_arch() {
    // A2.3 adds `qwen3_5` as a manifest arch token so the bake-off can
    // schedule Qwen 3.6 27B Dense (the family ships under the
    // `qwen3_5` model_type id) alongside the existing candidates.
    // mistralrs auto-detects the architecture from `config.json` at load
    // time; the manifest field is informational (used for `ctx_for`).
    let raw = r#"
[[candidate]]
id = "qwen3.6-27b-dense"
path = "/srv/q36"
arch = "qwen3_5"
license = "apache-2.0"
"#;
    let m = parse_manifest_str(raw).expect("parses");
    assert!(matches!(m.candidates[0].arch, CandidateArch::Qwen3_5));
    assert!(m.warnings().is_empty());
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
