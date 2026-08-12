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
