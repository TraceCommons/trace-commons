//! Syntax-checks `include/trace_commons.h` with a C compiler, when one is
//! available. Skips (rather than fails) on a machine with no `cc`, since
//! this crate must stay buildable without assuming a C toolchain is
//! present.

use std::process::Command;

#[test]
fn header_compiles_as_c() {
    check_header("cc", "c");
}

#[test]
fn header_compiles_as_cpp() {
    check_header("c++", "c++");
}

fn check_header(compiler: &str, language: &str) {
    let header = concat!(env!("CARGO_MANIFEST_DIR"), "/include/trace_commons.h");

    let found = Command::new(compiler).arg("--version").output();
    if found.is_err() {
        eprintln!("skipping: no `{compiler}` found on this machine");
        return;
    }

    let output = Command::new(compiler)
        .args(["-fsyntax-only", "-Wall", "-Wextra", "-x", language, header])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {compiler}: {e}"));

    assert!(
        output.status.success(),
        "{compiler} -fsyntax-only failed on {header}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
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
