//! Integration tests of `trace-commons-contributor-ffi`, built as one test binary.
//!
//! Each `tests/<name>.rs` below is a module, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-contributor-ffi --test it -- '<name>::'`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml, with the reason beside
//! it. The package sets `autotests = false`, so a file in neither place
//! would never be built; `it_harness` refuses that.
#![allow(unsafe_code)]

#[macro_use]
#[path = "../../trace-commons-protocol/tests/support/it_harness.rs"]
mod it_harness;

mod abi_attach;
mod abi_header_surface;
mod cli_tool;
mod credential_recovery;
mod credential_store_self_check;
mod harness_copy_is_central;
mod header;
mod private_inference_lifecycle;
mod swift_copy_surface_is_central;
mod tauri_copy_surface_is_central;
