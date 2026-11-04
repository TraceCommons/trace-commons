//! The commons' name stays out of everything a user sees.
//!
//! Owner rule, 2026-11-02: "The name Trace Commons should currently not
//! appear anywhere in the app ... not in any user visible place." The app is
//! Near.ai; a sentence about where traces go says "the server that receives
//! your traces" or similar. Internal identifiers (crate names, bundle ids,
//! keychain services, `TraceCommons` type names) are not covered: they have
//! no space, and this scan looks for the spoken form.
//!
//! This scans the production source of every crate whose strings reach a
//! user: the copy tables the shells decode, CLI help, daemon messages, and
//! what the daemon writes into other tools' config files. Comment lines are
//! skipped. Rust string continuations (`\` at line end) are joined first, so
//! a name split across two lines of one literal is still found.
//!
//! The one allowed file is `brand.rs`, which defines `commons_name!` and
//! `COMMONS_NAME` and has no consumers. Using either anywhere else fails,
//! because `concat!(commons_name!())` would put the name back without the
//! literal ever appearing in source.

use std::path::{Path, PathBuf};

/// Crates whose `src/` holds text a user can see.
const CRATES: &[&str] = &[
    "trace-commons-contributor",
    "trace-commons-contributor-sources",
    "trace-commons-contributor-insights",
    "trace-commons-contributor-platform",
    "trace-commons-contributor-ffi",
];

/// The definition of the name. Kept, with no consumers, for when the owner
/// brings it back.
const ALLOWED: &[&str] = &["trace-commons-contributor/src/brand.rs"];

/// Single lines that may keep the name, each with why.
const ALLOWED_LINES: &[(&str, &str)] = &[
    // An evaluation fixture quoting `scripts/winget/generate-manifests.sh`
    // verbatim, as a skill-learning test input. It is never displayed; the
    // winget package name itself is a packaging question listed in the PR
    // for the owner.
    (
        "trace-commons-contributor/src/skill_loop/evaluation/fixtures/catalog.rs",
        "PackageName: Trace Commons Contributor",
    ),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// Drops comment lines and joins `\`-continued string lines, then reports
/// each finding as `(line, text)`.
fn findings(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut joined = String::new();
    let mut start = 0;
    for (index, line) in source.lines().enumerate() {
        if joined.is_empty() {
            start = index + 1;
            joined.push_str(line);
        } else {
            // A continued string literal drops the next line's indentation.
            joined.push_str(line.trim_start());
        }
        if joined.ends_with('\\') {
            joined.pop();
            continue;
        }
        let logical = std::mem::take(&mut joined);
        if logical.trim_start().starts_with("//") {
            continue;
        }
        let collapsed = logical.split_whitespace().collect::<Vec<_>>().join(" ");
        let names_it = collapsed.to_lowercase().contains("trace commons");
        let uses_it = collapsed.contains("commons_name!") || collapsed.contains("COMMONS_NAME");
        // A test asserting a page does NOT carry the name is this guard's
        // own job, done closer to the page: `assert!(!x.contains(...COMMONS_NAME))`.
        let refuses_it = collapsed.contains("assert!(!")
            && collapsed.contains(".contains(crate::brand::COMMONS_NAME)");
        if names_it || (uses_it && !refuses_it) {
            found.push((start, collapsed));
        }
    }
    found
}

#[test]
fn the_scanner_finds_the_name_in_every_form_it_claims_to() {
    assert_eq!(
        findings("let s = \"Connected to Trace Commons.\";").len(),
        1
    );
    assert_eq!(findings("let s = \"TRACE COMMONS\";").len(), 1);
    assert_eq!(
        findings("let s = \"split Trace \\\n    Commons here\";").len(),
        1
    );
    assert_eq!(findings("concat!(\"a \", crate::commons_name!())").len(), 1);
    assert_eq!(findings("let n = brand::COMMONS_NAME;").len(), 1);
    assert!(
        findings("assert!(!page.contains(crate::brand::COMMONS_NAME), \"{page}\");").is_empty()
    );
    assert_eq!(
        findings("assert!(page.contains(crate::brand::COMMONS_NAME));").len(),
        1
    );
    assert!(findings("// Trace Commons in a comment").is_empty());
    assert!(findings("/// Trace Commons in a doc comment").is_empty());
    assert!(findings("use trace_commons_contributor::TraceCommons;").is_empty());
}

#[test]
fn no_user_visible_source_names_the_commons() {
    let root = crates_dir();
    let mut files = Vec::new();
    for krate in CRATES {
        rust_files(&root.join(krate).join("src"), &mut files);
    }
    assert!(
        files.len() > 100,
        "scanned only {} files; the crate layout moved",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .expect("under crates/")
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        let source = std::fs::read_to_string(file).expect("read source");
        for (line, text) in findings(&source) {
            if ALLOWED_LINES.contains(&(rel.as_str(), text.as_str())) {
                continue;
            }
            offenders.push(format!("{rel}:{line}: {text}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "The commons' name must not appear in anything a user sees (owner rule \
         2026-11-02). Say \"this app\", \"the server that receives your traces\" or \
         \"NEAR AI\" instead:\n{}",
        offenders.join("\n")
    );
}
