//! Integration tests of `trace-commons-protocol`, built as one test binary.
//!
//! Each `tests/<name>.rs` below is a module, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-protocol --test it -- '<name>::'`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml, with the reason beside
//! it. The package sets `autotests = false`, so a file in neither place
//! would never be built; `it_harness` refuses that.

#![forbid(unsafe_code)]

#[macro_use]
#[path = "support/it_harness.rs"]
mod it_harness;

mod consent_policy_pin;
mod deflate_backend_pin;
mod privacy_filter_near_ai_http;
mod privacy_filter_self_hosted_http;
mod token_distribution;
mod token_distribution_chat;
mod upload_allowance;
