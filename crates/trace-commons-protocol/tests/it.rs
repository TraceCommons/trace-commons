//! Every integration test of `trace-commons-protocol`, built as one test binary.
//!
//! Each `tests/<name>.rs` is a module here, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-protocol --test it -- <name>::`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml instead, with the reason
//! beside it. The package sets `autotests = false`, so a file that is in
//! neither place would never be built; `wiring` refuses that.

mod consent_policy_pin;
mod deflate_backend_pin;
mod privacy_filter_near_ai_http;
mod privacy_filter_self_hosted_http;
mod token_distribution;
mod token_distribution_chat;
mod upload_allowance;

mod wiring {
    /// A `tests/*.rs` file that is neither a module of this binary nor
    /// its own `[[test]]` target compiles nowhere and runs nowhere, and
    /// nothing else would say so.
    #[test]
    fn every_test_file_is_built() {
        let root = include_str!("it.rs");
        let manifest = include_str!("../Cargo.toml");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut unbuilt = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            if stem == "it" {
                continue;
            }
            let module = format!("\nmod {stem};\n");
            let target = format!("path = \"tests/{stem}.rs\"");
            if !root.contains(&module) && !manifest.contains(&target) {
                unbuilt.push(stem);
            }
        }
        unbuilt.sort();
        assert!(
            unbuilt.is_empty(),
            "no test target builds these tests/*.rs files: {unbuilt:?}. Add `mod <name>;` to \
             tests/it.rs, or give the file its own [[test]] in Cargo.toml with the reason"
        );
    }
}
