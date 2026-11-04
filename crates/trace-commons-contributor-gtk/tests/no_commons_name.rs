//! The commons' name stays out of everything this shell shows.
//!
//! Owner rule, 2026-11-02: "The name Trace Commons should currently not
//! appear anywhere in the app ... not in any user visible place." The
//! contributor crate holds the same guard over the shared copy
//! (`tests/no_commons_name_in_copy.rs`); this one covers what GTK holds
//! itself: its own `copy.rs` and widgets, the desktop entry, and the
//! AppStream metainfo that GNOME Software shows.
//!
//! Comment lines are skipped, Rust `\`-continued literals are joined, and a
//! use of `commons_name!` or `COMMONS_NAME` counts, because `concat!` would
//! put the name back without the literal appearing in source.

use std::path::{Path, PathBuf};

/// Lines that keep the name for now, each with why. Both are in the
/// metainfo and are listed in the PR for the owner rather than reworded:
/// replacing a developer name asserts who the developer is, and the other
/// is a past release's notes.
const ALLOWED_LINES: &[(&str, &str)] = &[
    (
        "flatpak/ai.tracecommons.Contributor.metainfo.xml",
        "<name>Trace Commons</name>",
    ),
    (
        "flatpak/ai.tracecommons.Contributor.metainfo.xml",
        "<p>The app is now called Near.ai. Trace Commons remains the name of the commons your traces are contributed to.",
    ),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
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
        if collapsed.to_lowercase().contains("trace commons")
            || collapsed.contains("commons_name!")
            || collapsed.contains("COMMONS_NAME")
        {
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
    assert_eq!(
        findings("let s = \"split Trace \\\n    Commons here\";").len(),
        1
    );
    assert_eq!(findings("concat!(\"a \", copy::commons_name!())").len(), 1);
    assert!(findings("// Trace Commons in a comment").is_empty());
}

#[test]
fn nothing_this_shell_shows_names_the_commons() {
    let root = root();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(files.len() > 10, "scanned only {} files", files.len());
    files.push(root.join("flatpak/ai.tracecommons.Contributor.desktop"));
    files.push(root.join("flatpak/ai.tracecommons.Contributor.metainfo.xml"));

    let mut offenders = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .expect("under the crate")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(file).expect("read source");
        for (line, text) in findings(&source) {
            if ALLOWED_LINES
                .iter()
                .any(|(f, needle)| *f == rel && text.contains(needle))
            {
                continue;
            }
            offenders.push(format!("{rel}:{line}: {text}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "The commons' name must not appear in anything a user sees (owner rule \
         2026-11-02):\n{}",
        offenders.join("\n")
    );
}
