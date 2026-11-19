// Puts libgtk4-layer-shell ahead of libwayland-client in the binaries'
// DT_NEEDED list.
//
// gtk4-layer-shell (>= 1.0) shims libwayland: it defines the `wl_proxy_*`
// entry points and forwards to the real ones with `dlsym(RTLD_NEXT, ...)`,
// so libwayland-client must load after it. Since the KDE blur work, wayland-sys
// links libwayland-client directly (gdk4-wayland's `wayland_crate` feature), and
// rustc orders the two -sys crates' `-l` flags by crate-graph accident. When
// wayland-client won, every launch printed `libwayland_shim: dlsym failed to
// load wl_proxy_marshal_array_flags` and drew nothing.
//
// rustc places a crate's native libraries ahead of those of the crates it
// depends on, so naming the library from this package puts it before both
// -sys crates' entries. The library's search path still comes from
// gtk4-layer-shell-sys (pkg-config), which also covers the Flatpak, whose
// gtk4-layer-shell module installs into /app.
//
// `tests/layer_shell_link_order.rs` reads the built shell binary and holds the
// order.

#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let linux = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    let layer_shell = std::env::var_os("CARGO_FEATURE_LAYER_SHELL").is_some();
    if linux && layer_shell {
        println!("cargo:rustc-link-lib=dylib=gtk4-layer-shell");
    }
}
