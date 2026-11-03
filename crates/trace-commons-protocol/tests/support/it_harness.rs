//! Shared by every `tests/it.rs` that builds a crate's integration tests as
//! one binary. Each `it.rs` declares it first, with `#[macro_use]`, so
//! `libtest_name!` is in scope in every module after it:
//!
//! ```ignore
//! #[macro_use]
//! #[path = "../../trace-commons-protocol/tests/support/it_harness.rs"]
//! mod it_harness;
//! ```

/// `name` as libtest knows it in this binary: tests in a module of a combined
/// binary are named `<module>::<name>`, and a test that re-runs its own binary
/// with `--exact` and the bare name runs nothing and still exits 0. Expands
/// `module_path!()` where it is called.
#[allow(unused_macros)]
macro_rules! libtest_name {
    ($name:expr) => {{
        let name: &str = $name;
        match module_path!().split_once("::") {
            Some((_, module)) => format!("{module}::{name}"),
            None => name.to_owned(),
        }
    }};
}

/// With `autotests = false`, a `tests/*.rs` file that is neither a `mod` of
/// `tests/it.rs` nor the `path` of a `[[test]]` table compiles nowhere and
/// runs nowhere, and nothing else would say so. A file that is both runs
/// twice, once inside the shared process it was kept out of.
#[test]
fn every_test_file_is_built_once() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests = root.join("tests");
    let it = std::fs::read_to_string(tests.join("it.rs")).unwrap();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();

    let modules: Vec<&str> = it
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let mut targets = Vec::new();
    let mut in_test_table = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_test_table = line == "[[test]]";
        } else if in_test_table {
            let path = line.strip_prefix("path").map(str::trim_start);
            if let Some(path) = path.and_then(|rest| rest.strip_prefix('=')) {
                let path = path.trim().trim_matches('"');
                let stem = path
                    .strip_prefix("tests/")
                    .and_then(|p| p.strip_suffix(".rs"));
                targets.push(stem.unwrap_or(path).to_owned());
            }
        }
    }

    let mut unbuilt = Vec::new();
    let mut twice = Vec::new();
    for entry in std::fs::read_dir(&tests).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let in_it = modules.contains(&stem.as_str());
        let own = targets.contains(&stem);
        if stem == "it" {
            assert!(
                own,
                "Cargo.toml has no [[test]] with path = \"tests/it.rs\""
            );
        } else if !in_it && !own {
            unbuilt.push(stem);
        } else if in_it && own {
            twice.push(stem);
        }
    }
    unbuilt.sort();
    twice.sort();
    assert!(
        unbuilt.is_empty(),
        "no test target builds these tests/*.rs files: {unbuilt:?}. Add `mod <name>;` to \
         tests/it.rs, or give the file its own [[test]] in Cargo.toml with the reason"
    );
    assert!(
        twice.is_empty(),
        "these tests/*.rs files are both a module of tests/it.rs and their own [[test]]: \
         {twice:?}. Keep one"
    );
}
