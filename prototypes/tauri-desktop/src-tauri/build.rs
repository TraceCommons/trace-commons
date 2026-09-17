use std::{env, path::PathBuf, process::Command};

fn main() {
    tauri_build::build();

    #[cfg(target_os = "macos")]
    build_macos_native_bridge();
}

#[cfg(target_os = "macos")]
fn build_macos_native_bridge() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let object = out_dir.join("trace_commons_macos.o");
    let archive = out_dir.join("libtrace_commons_macos.a");
    let target = env::var("TARGET").expect("TARGET is set by Cargo");
    let architecture = match target.as_str() {
        "aarch64-apple-darwin" => "arm64",
        "x86_64-apple-darwin" => "x86_64",
        _ => panic!("unsupported macOS target for native bridge: {target}"),
    };

    println!("cargo:rerun-if-changed=native_macos.m");

    let status = Command::new("clang")
        .args([
            "-fobjc-arc",
            "-fblocks",
            "-arch",
            architecture,
            "-mmacosx-version-min=13.0",
            "-c",
            "native_macos.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("failed to start clang for macOS native bridge");
    assert!(
        status.success(),
        "clang failed to build macOS native bridge"
    );

    let status = Command::new("ar")
        .args(["-rcs"])
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("failed to archive macOS native bridge");
    assert!(status.success(), "ar failed to archive macOS native bridge");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=trace_commons_macos");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ServiceManagement");
    println!("cargo:rustc-link-lib=framework=UserNotifications");
}
