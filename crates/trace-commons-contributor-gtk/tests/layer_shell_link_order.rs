//! libgtk4-layer-shell must load ahead of libwayland-client.
//!
//! gtk4-layer-shell (>= 1.0) carries a libwayland shim: it defines the
//! `wl_proxy_*` entry points itself and forwards to the real ones with
//! `dlsym(RTLD_NEXT, ...)`. That only works when libwayland-client comes
//! AFTER libgtk4-layer-shell in the process's lookup order. If it comes first,
//! the shim's lookup finds nothing past itself, every launch prints
//! `libwayland_shim: dlsym failed to load wl_proxy_marshal_array_flags`, and
//! the window never draws (the weston job's OCR then finds no text at all).
//!
//! The binary's own DT_NEEDED order decides that lookup order. Until the KDE
//! blur work linked wayland-client directly (through wayland-sys, for
//! gdk4-wayland's `wayland_crate` feature), libwayland-client arrived only as
//! a dependency of libgtk-4, behind every direct NEEDED entry. Now it is a
//! direct entry too, and which of the two cargo lists first is an accident
//! of crate order. `build.rs` names libgtk4-layer-shell from this package so
//! it comes first; this test reads the built binary and holds that.

#![cfg(all(target_os = "linux", feature = "layer-shell"))]

use std::process::Command;

/// The binary's DT_NEEDED entries, in order, from `readelf -d`.
fn needed(binary: &str) -> Vec<String> {
    let out = Command::new("readelf")
        .args(["-d", "--wide", binary])
        .output()
        .expect("readelf (binutils) is needed to read the shell's NEEDED entries");
    assert!(
        out.status.success(),
        "readelf -d {binary} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains("(NEEDED)"))
        .filter_map(|line| {
            let start = line.find('[')? + 1;
            let end = line.rfind(']')?;
            Some(line[start..end].to_owned())
        })
        .collect()
}

fn position(needed: &[String], prefix: &str) -> Option<usize> {
    needed.iter().position(|lib| lib.starts_with(prefix))
}

fn assert_layer_shell_first(binary: &str) {
    let needed = needed(binary);
    let layer_shell = position(&needed, "libgtk4-layer-shell.so").unwrap_or_else(|| {
        panic!("{binary} does not link libgtk4-layer-shell directly; NEEDED: {needed:?}")
    });
    // Absent is fine: libwayland-client then loads as libgtk-4's dependency,
    // after every direct entry.
    if let Some(wayland) = position(&needed, "libwayland-client.so") {
        assert!(
            layer_shell < wayland,
            "{binary} lists libwayland-client (#{wayland}) before libgtk4-layer-shell \
             (#{layer_shell}); the layer-shell libwayland shim then cannot find the real \
             entry points. NEEDED: {needed:?}"
        );
    }
}

#[test]
fn the_shell_loads_layer_shell_before_wayland_client() {
    assert_layer_shell_first(env!("CARGO_BIN_EXE_trace-commons-shell"));
}
