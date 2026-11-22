//! Sets the `red_team_platform` cfg on the targets Red Team runs on: macOS,
//! Windows and x86-64 Linux. One name for the list, so the gates in
//! `managed::red_team*` do not each repeat it.

#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rustc-check-cfg=cfg(red_team_platform)");
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os == "macos" || os == "windows" || (os == "linux" && arch == "x86_64") {
        println!("cargo:rustc-cfg=red_team_platform");
    }
}
