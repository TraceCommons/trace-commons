//! Syntax-checks `include/trace_commons.h` with a C compiler, when one is
//! available. Skips (rather than fails) on a machine with no `cc`, since
//! this crate must stay buildable without assuming a C toolchain is
//! present.

use std::process::Command;

#[test]
fn header_compiles_as_c() {
    let header = concat!(env!("CARGO_MANIFEST_DIR"), "/include/trace_commons.h");

    let found = Command::new("cc").arg("--version").output();
    if found.is_err() {
        eprintln!("skipping: no `cc` found on this machine");
        return;
    }

    let output = Command::new("cc")
        .args(["-fsyntax-only", "-Wall", "-Wextra", "-x", "c", header])
        .output()
        .expect("failed to run cc");

    assert!(
        output.status.success(),
        "cc -fsyntax-only failed on {header}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn header_compiles_as_cpp() {
    let header = concat!(env!("CARGO_MANIFEST_DIR"), "/include/trace_commons.h");

    let found = Command::new("c++").arg("--version").output();
    if found.is_err() {
        eprintln!("skipping: no `c++` found on this machine");
        return;
    }

    let output = Command::new("c++")
        .args(["-fsyntax-only", "-Wall", "-Wextra", "-x", "c++", header])
        .output()
        .expect("failed to run c++");

    assert!(
        output.status.success(),
        "c++ -fsyntax-only failed on {header}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Every `extern "C"` function this crate exports must be declared in the
/// header.
///
/// The copy-to-copy check below compares the two headers to each other, so
/// it is blind to a symbol missing from BOTH -- which is exactly what it
/// found: `tc_invite_issuer_host` was exported and callable, and declared
/// nowhere, so no C or Swift client could reach it without writing its own
/// prototype and guessing the signature.
///
/// Matches on the symbol name only. A signature mismatch between the Rust
/// export and the C declaration is a real hazard this cannot see, and the
/// thing that does catch it is `tests/abi.rs`, which calls these through the
/// declared types and would not link if they disagreed.
#[test]
fn every_exported_symbol_is_declared_in_the_header() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("reading the crate source");
    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/include/trace_commons.h"
    ))
    .expect("reading the header");

    let mut exported: Vec<String> = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.split_once("extern \"C\" fn ").map(|(_, r)| r) else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.starts_with("tc_") && !exported.contains(&name) {
            exported.push(name);
        }
    }
    assert!(
        exported.len() > 10,
        "the export scan found only {} symbols, so it is broken rather than passing",
        exported.len()
    );

    let undeclared: Vec<&String> = exported
        .iter()
        .filter(|name| !header.contains(name.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "exported but not declared in include/trace_commons.h: {undeclared:#?}"
    );
}

/// The macOS copy of this header must declare the same ABI as this one.
///
/// There are two hand-synced copies of `trace_commons.h`: this crate's, and
/// `macos/Sources/CTraceCommons/include/trace_commons.h`, which is what the
/// Swift client actually compiles against. Nothing checked that they agreed,
/// and they had drifted -- when this test was written the macOS copy was
/// missing `tc_preview_turns_json` entirely, so a function the dylib exports
/// was uncallable from Swift, silently.
///
/// This compares DECLARATIONS, not files. The two copies carry different
/// prose and are ~100 lines apart on comments alone, which is tolerable; a
/// differing set of functions or signatures is not. Comparing whole files
/// would fail constantly and then be turned off, which is how the drift
/// survived in the first place.
///
/// A client-facing comment that contradicts the ABI is the other half of
/// this failure mode and no test can catch it -- three separate clients
/// carried "the C ABI has no call to set claude_root/codex_root" for
/// releases after that call existed. Update both copies together.
#[test]
fn both_header_copies_declare_the_same_abi() {
    fn declarations(source: &str) -> Vec<String> {
        // Strip block comments, then take every declaration naming a `tc_`
        // symbol, normalized on whitespace so line-wrapping differences
        // between the copies do not read as an ABI difference.
        let mut stripped = String::with_capacity(source.len());
        let mut rest = source;
        while let Some(start) = rest.find("/*") {
            stripped.push_str(&rest[..start]);
            match rest[start..].find("*/") {
                Some(end) => rest = &rest[start + end + 2..],
                None => {
                    // An unterminated comment: nothing after it is a
                    // declaration, so drop the remainder rather than
                    // parsing comment prose as ABI.
                    rest = "";
                    break;
                }
            }
        }
        stripped.push_str(rest);

        stripped
            .split(';')
            .filter(|d| d.contains("tc_") && d.contains('('))
            .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|d| !d.is_empty())
            .collect()
    }

    let ours = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/include/trace_commons.h"
    ))
    .expect("reading this crate's header");
    let theirs = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../macos/Sources/CTraceCommons/include/trace_commons.h"
    ))
    .expect("reading the macOS header copy");

    let ours = declarations(&ours);
    let theirs = declarations(&theirs);

    let missing: Vec<_> = ours.iter().filter(|d| !theirs.contains(d)).collect();
    let extra: Vec<_> = theirs.iter().filter(|d| !ours.contains(d)).collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "the two trace_commons.h copies declare different ABIs.\n\
         Missing from the macOS copy: {missing:#?}\n\
         Only in the macOS copy: {extra:#?}"
    );
}

/// Every exported function that dereferences a `tc_handle*` must first
/// establish that the pointer is live.
///
/// The liveness check is centralised in `handle_pointer_is_live`, but its
/// *invocation* is hand-placed, in four different idioms (a bare early
/// return, an `error_frame`, an `Ok(0u64)`, an `anyhow::bail!`), and the
/// `unsafe { &*handle }` that follows is still perfectly writable without
/// it. Centralising the helper does not stop the eighth entry point from
/// forgetting to call it -- and forgetting it is not a wrong answer, it is
/// a use-after-free or a type confusion.
///
/// `tc_handle_free` is the one exception: it establishes liveness by
/// removing the registry entry with `registry_take`, which is a stronger
/// claim than `handle_pointer_is_live` makes, not a weaker one.
///
/// Scans the source rather than the behaviour, so it cannot see whether the
/// check actually precedes the dereference -- `tests/abi.rs` covers that per
/// entry point, calling each with a freed and a wrong-type pointer. What
/// this adds is that a NEW entry point cannot quietly skip both.
#[test]
fn every_exported_fn_that_derefs_a_handle_checks_it_first() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("reading the crate source");

    const EXPORT: &str = "\npub unsafe extern \"C\" fn ";
    let mut unguarded: Vec<String> = Vec::new();
    let mut checked = 0usize;

    let mut search = source.as_str();
    while let Some(at) = search.find(EXPORT) {
        let after = &search[at + EXPORT.len()..];
        let name: String = after.chars().take_while(|c| *c != '(').collect();
        // The body runs to the next export, or to the end of the file.
        let body = match after.find(EXPORT) {
            Some(next) => &after[..next],
            None => after,
        };

        if body.contains("&*handle") {
            checked += 1;
            if !body.contains("handle_pointer_is_live") && !body.contains("registry_take") {
                unguarded.push(name);
            }
        }

        search = after;
    }

    // A rename that broke the scan would leave nothing to check and pass.
    assert!(
        checked >= 7,
        "found only {checked} exported functions dereferencing a handle: the scan is \
         matching nothing, and an empty scan passes"
    );
    assert!(
        unguarded.is_empty(),
        "these exported functions dereference a tc_handle* without establishing that it \
         is live -- a stale or wrong-type pointer reaches `&*handle`: {unguarded:#?}"
    );
}
