//! A signed bundle is the only thing that can answer whether it reaches its
//! credential store -- entitlements are a property of the code signature, and
//! `cargo test` never has one.

use trace_commons_contributor_ffi::tc_credential_store_self_check;

/// The test process is unentitled, so the honest answer here is 1. A 0 would
/// mean the check cannot tell the two apart, which would make it useless as
/// the thing standing between us and shipping an app that does not launch.
#[test]
#[cfg(target_os = "macos")]
fn an_unentitled_process_reports_one() {
    assert_eq!(tc_credential_store_self_check(), 1);
}
