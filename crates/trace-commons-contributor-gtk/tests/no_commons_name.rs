//! The commons' name stays out of everything this shell shows.
//!
//! Owner rule, 2026-11-02: "The name Trace Commons should currently not
//! appear anywhere in the app ... not in any user visible place." The
//! contributor crate holds the same guard over the shared copy
//! (`tests/no_commons_name_in_copy.rs`); this one covers what GTK holds
//! itself: its own `copy.rs` and widgets, the desktop entry, the AppStream
//! metainfo that GNOME Software shows, the `.flatpakref` that
//! `scripts/flatpak/publish-repo.sh` writes, whose `Title=` the software
//! centre shows when the app is installed from it, and the GNOME Shell
//! extension.
//!
//! Comment lines are skipped, Rust `\`-continued literals are joined, a
//! name wrapped across two lines of prose counts, and a use of
//! `commons_name!` or `COMMONS_NAME` counts, because `concat!` would put the
//! name back without the literal appearing in source.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The repository root, two levels above this crate.
fn repo_root() -> PathBuf {
    root()
        .parent()
        .and_then(Path::parent)
        .expect("crate sits at crates/<name>")
        .to_path_buf()
}

fn names_the_commons(text: &str) -> bool {
    text.to_lowercase().contains("trace commons")
        || text.contains("commons_name!")
        || text.contains("COMMONS_NAME")
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
        if names_the_commons(&collapsed) {
            found.push((start, collapsed));
        }
    }
    // Prose wrapped at "Trace" / "Commons" has no `\` to join on, so each
    // pair of lines is read as one when neither line names it alone.
    let lines: Vec<&str> = source.lines().collect();
    for (index, pair) in lines.windows(2).enumerate() {
        let comment = |l: &str| l.trim_start().starts_with("//");
        if comment(pair[0]) || comment(pair[1]) {
            continue;
        }
        let collapsed = |l: &str| l.split_whitespace().collect::<Vec<_>>().join(" ");
        let (first, second) = (collapsed(pair[0]), collapsed(pair[1]));
        let both = format!("{first} {second}");
        if names_the_commons(&both) && !names_the_commons(&first) && !names_the_commons(&second) {
            found.push((index + 1, both));
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
    assert_eq!(
        findings("    <p>It contributes to Trace\n      Commons, a corpus.</p>").len(),
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
    files.push(repo_root().join("scripts/flatpak/publish-repo.sh"));
    // The GNOME Shell extension: GNOME's Extensions app shows its name and
    // description, and its code is shipped beside the app.
    for name in ["metadata.json", "extension.js", "placement.js"] {
        files.push(
            root.join("gnome-shell-extension/edge-surfaces@tracecommons.ai")
                .join(name),
        );
    }

    let repo = repo_root();
    let mut offenders = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&repo)
            .expect("under the repository")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(file).expect("read source");
        for (line, text) in findings(&source) {
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

/// The launcher and the software centre name the app the way its windows
/// do. Both files are plain text the shell cannot fill in from
/// [`APP_NAME`], so this ties them to it; a name kept from before a rename
/// (`FORMER_APP_NAMES`) may appear only in the release notes that announce
/// the rename.
#[test]
fn the_desktop_entry_and_the_store_listing_use_the_app_name() {
    use trace_commons_contributor::brand::{APP_NAME, FORMER_APP_NAMES};
    let root = root();
    let desktop = std::fs::read_to_string(root.join("flatpak/ai.tracecommons.Contributor.desktop"))
        .expect("read desktop entry");
    let names: Vec<&str> = desktop
        .lines()
        .filter_map(|l| l.strip_prefix("Name="))
        .collect();
    assert_eq!(names, [APP_NAME], "desktop entry Name=");

    let metainfo =
        std::fs::read_to_string(root.join("flatpak/ai.tracecommons.Contributor.metainfo.xml"))
            .expect("read metainfo");
    let listing = metainfo
        .split("<releases>")
        .next()
        .expect("text before the release notes");
    let component_name = listing
        .split("<name>")
        .nth(1)
        .and_then(|rest| rest.split("</name>").next())
        .expect("a <name> element");
    assert_eq!(component_name, APP_NAME, "metainfo <name>");
    for former in FORMER_APP_NAMES {
        assert!(
            !desktop.contains(former),
            "desktop entry still names the app {former}"
        );
        assert!(
            !listing.contains(former),
            "store listing (outside the release notes) still names the app {former}"
        );
    }
}
