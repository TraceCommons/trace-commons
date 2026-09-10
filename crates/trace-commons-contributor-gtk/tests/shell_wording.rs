//! What this shell says, checked against its own source.
//!
//! Task 5 of the shell-copy slice moves the consent statement out of
//! `src/copy.rs` and into `trace_commons_contributor::consent_copy`, where
//! all three shells read it. The sweep below is what keeps the move from
//! half-happening: a re-export with the old literal still sitting beside it
//! compiles, passes every parity test, and renders the stale sentence on
//! this shell while the other two render the shared one.
//!
//! It lives here rather than in `copy.rs`'s own test module on purpose. A
//! sweep that reads the file it is written in also reads its own needles, so
//! the marker text inside the assertion would open a region that does not
//! exist. Reading the source from disk, from outside, is the only shape that
//! counts the real markers.
//!
//! The second half of this file counts, rather than sweeps: the wording
//! ratchet, the mirror of `ShellWordingTests.cs` on Windows and
//! `ShellWordingTests.swift` on macOS, for the same reason. A sentence
//! hand-written in one shell survives a rename in the other two, and until
//! this guard existed nothing here noticed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// This crate's `src/`, resolved from the manifest rather than the cwd.
fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// A migrated region of `copy.rs` holds re-exports and nothing else.
///
/// The drift this catches is specific and has already happened once in
/// spirit: `copy.rs` is the file the other two shells historically
/// transcribed from, so a literal left beside the re-export renders the old
/// constant on the GTK screen while the ABI serves the new one to the other
/// two. Same technique as the routing sweep in `routing_copy.rs`, which
/// reads its own source between `TOOLS-SURFACE-BEGIN` and
/// `TOOLS-SURFACE-END`.
#[test]
fn a_migrated_region_of_copy_rs_holds_no_words_of_its_own() {
    let source = std::fs::read_to_string(src_root().join("copy.rs")).expect("copy.rs is readable");

    let regions: Vec<&str> = source
        .split("// COPY-MIGRATED-BEGIN")
        .skip(1)
        .map(|rest| {
            rest.split("// COPY-MIGRATED-END")
                .next()
                .expect("every COPY-MIGRATED-BEGIN is closed by a COPY-MIGRATED-END")
        })
        .collect();

    // A sweep over no regions is a sweep over nothing, which is the failure
    // mode this whole slice is written against.
    assert!(
        !regions.is_empty(),
        "copy.rs has no COPY-MIGRATED region; migrated copy must be marked"
    );
    assert_eq!(
        source.matches("// COPY-MIGRATED-BEGIN").count(),
        source.matches("// COPY-MIGRATED-END").count(),
        "every marker must be paired"
    );

    for region in regions {
        assert!(
            !region.contains('"'),
            "a migrated region of copy.rs holds a string literal. It may hold `pub use` and \
             nothing else -- a word left beside the re-export is the word this shell renders \
             while the other two render the shared one:\n{region}"
        );
        assert!(
            region.contains("pub use trace_commons_contributor::"),
            "a migrated region must re-export from the shared crate:\n{region}"
        );
    }
}

// ---------------------------------------------------------------------------
// Wording authored in this shell, over the whole shell.
//
// What counts as authored here needs saying, because this shell is the one
// the other two transcribed from. `copy.rs` is the migration target, so its
// `pub const` sentences are counted like any others. A `pub use` re-export
// is not a literal, so moving a constant into
// `trace_commons_contributor::*_copy` lowers the number by itself -- which
// is exactly the ratchet the migration needs.
//
// It is a ratchet, not a clean bill of health.
// ---------------------------------------------------------------------------

/// Files that author wording today, and exactly how much.
///
/// TODO(shell-copy): every entry here is a file whose wording should be
/// composed in `trace_commons_contributor` and re-exported, the way
/// `routing_copy` and `witness_copy` already are. Until then the number is a
/// CEILING AND A FLOOR both: adding a sentence fails, and removing one fails
/// too, so the entry has to be lowered deliberately as copy moves out. Never
/// raise a number. A new file must never be added here.
///
/// MEASURED, NOT ESTIMATED. Every number came from this file's own scanner
/// via `TC_WORDING_DUMP=1`; none was typed by hand.
const WORDING_BASELINE: &[(&str, usize)] = &[
    ("autostart.rs", 2),
    ("backend.rs", 1),
    ("bin/probe.rs", 4),
    ("copy.rs", 273),
    ("main.rs", 2),
    ("model.rs", 4),
    ("notify.rs", 7),
    ("tray.rs", 1),
    ("ui/history.rs", 3),
    ("ui/onboarding.rs", 2),
    ("ui/preview.rs", 10),
    ("ui/queue.rs", 8),
    ("ui/settings.rs", 24),
    ("update.rs", 2),
    ("worker.rs", 1),
];

/// The surfaces whose wording already comes from the shared crate. Nothing
/// may ever buy them an allowance in the baseline.
///
/// `ui/style.rs` and `ui/css_contract.rs` are not here: they hold CSS, which
/// is not wording, and the scanner does not count it.
const RUST_OWNED_SURFACES: &[&str] = &[];

/// Words a sentence has and an identifier, a wire key, a CSS class or a
/// format pattern does not.
///
/// The same list the Windows and macOS guards use, deliberately: three
/// shells counting the same corpus by different rules would produce three
/// numbers nobody could compare.
const FUNCTION_WORDS: &str = "\
a an and are as at be been being but by can cannot could did do does for from \
had has have how if in into is isn't it it's its just may never no not nothing of off on once only or \
so some still such than that the their them then there they this those to until up was we were what \
when where which while who will with would you your yours yet anything something everything already \
always about after again all any because before both each else ever every here more most much \
must need needs same see should since take takes tell these too under use used using very";

/// Every Rust source of this shell, keyed by its path relative to `src/`.
fn scan_shell_sources() -> BTreeMap<String, Vec<String>> {
    let root = src_root();
    let mut scanned = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the shell's sources are readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .expect("every scanned path is under src/")
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(&path).expect("a readable source");
            scanned.insert(relative, authored_wording(&source));
        }
    }
    scanned
}

/// The sentences a Rust source authors.
///
/// A character walk, for the reasons the routing sweep's scanner gives and
/// two more this file needs: raw strings hold CSS with braces in it, which
/// would wreck the brace counting below, and `#[cfg(test)]` modules are
/// interleaved with real copy in `copy.rs` rather than sitting at the end.
fn authored_wording(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut i = 0;
    // Set when the test attribute is seen; consumed by the next `{`, which
    // records the depth to skip back down to.
    let mut test_attribute_pending = false;
    let mut depth: usize = 0;
    let mut skip_below: Option<usize> = None;
    let test_attribute: Vec<char> = "#[cfg(test)]".chars().collect();

    while i < chars.len() {
        // Raw string, borrowed or not: r"..." and r#"..."# .
        if chars[i] == 'r' && i + 1 < chars.len() && (chars[i + 1] == '"' || chars[i + 1] == '#') {
            let mut hashes = 0;
            let mut j = i + 1;
            while j < chars.len() && chars[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < chars.len() && chars[j] == '"' {
                j += 1;
                'raw: while j < chars.len() {
                    if chars[j] == '"' {
                        let mut closing = 0;
                        while closing < hashes
                            && j + 1 + closing < chars.len()
                            && chars[j + 1 + closing] == '#'
                        {
                            closing += 1;
                        }
                        if closing == hashes {
                            j += 1 + hashes;
                            break 'raw;
                        }
                    }
                    j += 1;
                }
                // Raw strings in this shell are CSS and shell-out fixtures,
                // never sentences. Skipped whole rather than scanned.
                i = j;
                continue;
            }
        }
        // Line comment. Prose about the wire may quote it, and nothing in a
        // comment is rendered.
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // A char literal, so that a quote or a brace inside one cannot
        // unbalance the scan. Guarded so that the lifetime in
        // `&'static str` is not read as one.
        if chars[i] == '\'' && i + 2 < chars.len() && (chars[i + 2] == '\'' || chars[i + 1] == '\\')
        {
            i += 2;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if chars[i] == '#' && chars[i..].starts_with(&test_attribute[..]) {
            test_attribute_pending = true;
            i += test_attribute.len();
            continue;
        }
        // A test module in another file ends here. Its attribute must not
        // suppress the next production function's body.
        if chars[i] == ';' && test_attribute_pending {
            test_attribute_pending = false;
        }
        if chars[i] == '{' {
            depth += 1;
            if test_attribute_pending && skip_below.is_none() {
                skip_below = Some(depth);
                test_attribute_pending = false;
            }
            i += 1;
            continue;
        }
        if chars[i] == '}' {
            if let Some(floor) = skip_below {
                if depth == floor {
                    skip_below = None;
                }
            }
            depth = depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if chars[i] == '"' {
            i += 1;
            let mut literal = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    // Only the escapes this shell uses: a line continuation,
                    // whose payload is whitespace, and an escaped quote.
                    // Both fold to a space rather than being decoded,
                    // because this reads words and not punctuation.
                    i += 2;
                    literal.push(' ');
                    continue;
                }
                literal.push(if chars[i] == '\n' { ' ' } else { chars[i] });
                i += 1;
            }
            i += 1;
            if skip_below.is_none() && reads_as_a_sentence(&literal) {
                literals.push(literal);
            }
            continue;
        }
        i += 1;
    }
    literals
}

/// True where the literal reads as a sentence somebody wrote for a
/// contributor to read.
fn reads_as_a_sentence(literal: &str) -> bool {
    if !literal.contains(' ') {
        return false;
    }
    let function_words: Vec<&str> = FUNCTION_WORDS.split_whitespace().collect();
    let words: Vec<String> = literal
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_ascii_alphabetic() || *c == '\'')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    words.len() >= 2 && words.iter().any(|w| function_words.contains(&w.as_str()))
}

#[test]
fn an_external_test_module_does_not_hide_the_next_production_function() {
    let source = r#"
        #[cfg(test)]
        #[path = "../tests/support/example.rs"]
        mod tests;
        fn render() { label("A production sentence is visible"); }
    "#;
    assert_eq!(
        authored_wording(source),
        ["A production sentence is visible"]
    );
}

/// No file in this shell authors more wording than it did when this guard
/// was written, and no file starts authoring wording that did not.
#[test]
fn no_wording_is_authored_in_this_shell_beyond_the_recorded_baseline() {
    let scanned = scan_shell_sources();

    // A scan that found nothing would turn this test into a pass over
    // nothing, which is the failure mode the Windows guard names by name.
    // There are 33 sources under src/ today.
    assert!(
        scanned.len() >= 30,
        "only {} sources were scanned under {}; the whole tree is expected",
        scanned.len(),
        src_root().display()
    );
    assert!(
        scanned.contains_key("copy.rs"),
        "copy.rs was not scanned; it is the file this ratchet exists for"
    );

    if std::env::var("TC_WORDING_DUMP").as_deref() == Ok("1") {
        for (path, wording) in &scanned {
            if !wording.is_empty() {
                println!("    (\"{path}\", {}),", wording.len());
            }
        }
    }

    let baseline: BTreeMap<&str, usize> = WORDING_BASELINE.iter().copied().collect();
    let mut failures = Vec::new();
    for (path, wording) in &scanned {
        let allowed = baseline.get(path.as_str()).copied().unwrap_or(0);
        if wording.len() == allowed {
            continue;
        }
        if wording.len() > allowed {
            failures.push(format!(
                "{path}: {} authored sentences, baseline allows {allowed}. \
                 First one over the line: {:?}",
                wording.len(),
                wording[allowed]
            ));
        } else {
            failures.push(format!(
                "{path}: {} authored sentences, baseline still allows {allowed}. \
                 Wording moved out -- lower the entry (or delete it at zero).",
                wording.len()
            ));
        }
    }
    for recorded in baseline.keys() {
        if !scanned.contains_key(*recorded) {
            failures.push(format!(
                "{recorded}: recorded in the baseline but no longer in the shell. Delete the entry."
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Wording on this shell's surfaces should come from trace_commons_contributor.\n\
         A sentence written here is one the other two shells will not get, and one a rename \
         in the shared crate will not reach.\n\n{}",
        failures.join("\n")
    );
}

/// The surfaces the shared crate already owns hold no wording at all, and
/// hold no baseline entry either.
#[test]
fn the_rust_owned_surfaces_are_not_given_a_wording_allowance() {
    let scanned = scan_shell_sources();
    let baseline: BTreeMap<&str, usize> = WORDING_BASELINE.iter().copied().collect();
    for surface in RUST_OWNED_SURFACES {
        assert!(
            !baseline.contains_key(surface),
            "{surface} has a wording baseline entry. Its wording comes from the shared crate; \
             an allowance here would quietly undo that."
        );
        let wording = scanned.get(*surface).unwrap_or_else(|| {
            panic!("{surface} was not scanned; the guard would pass over nothing")
        });
        assert!(wording.is_empty(), "{surface} authors wording: {wording:?}");
    }
}
