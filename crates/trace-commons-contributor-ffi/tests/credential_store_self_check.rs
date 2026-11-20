//! A signed bundle is the only thing that can answer whether it reaches its
//! credential store -- entitlements are a property of the code signature, and
//! `cargo test` never has one.
//!
//! macOS and Windows only. On macOS the one assertion is about entitlements,
//! which exist nowhere else. On Windows there are no entitlements, so a plain
//! test process is a fair stand-in for the app: it must reach the Credential
//! Manager. CI sets `-D warnings`, so a `use` left ungated for a gated
//! consumer is a hard error on the Linux job rather than an unused-import
//! warning.
#![cfg(any(target_os = "macos", windows))]

use trace_commons_contributor_ffi::tc_credential_store_self_check;

/// The test process is unentitled, so the honest answer here is 1. A 0 would
/// mean the check cannot tell the two apart, which would make it useless as
/// the thing standing between us and shipping an app that does not launch.
#[cfg(target_os = "macos")]
#[test]
fn an_unentitled_process_reports_one() {
    assert_eq!(tc_credential_store_self_check(), 1);
}

/// W25-D2 (a). The real Credential Manager answers, with nothing stored under
/// a reference that was never written, so the check reports 0. Ignored
/// because it reaches the real OS store (it reads only); CI runs it by name on
/// Windows in a build without the test credential-store feature, which would
/// refuse the store and report 2.
#[cfg(windows)]
#[test]
#[ignore = "reads the real Windows Credential Manager; run by name on Windows"]
fn a_windows_process_reaches_the_credential_manager() {
    assert_eq!(tc_credential_store_self_check(), 0);
}
