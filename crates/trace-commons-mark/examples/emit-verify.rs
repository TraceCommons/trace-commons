//! Write the framed mark as PNGs at a spread of sizes, for verifying the
//! encoder against a decoder this repository did not write.
//!
//! An example rather than a test because the check that matters -- that a
//! foreign decoder agrees with our encoder -- cannot be made from inside the
//! encoder. `scripts/mark/verify-png.py` reads what this writes.

use trace_commons_mark::{Scheme, raster};

fn main() {
    let out = std::env::args().nth(1).expect("usage: emit-verify <dir>");
    for size in [16u32, 44, 50, 150, 256] {
        let path = format!("{out}/mark-{size}.png");
        std::fs::write(&path, raster::png(Scheme::Light, size)).expect("write");
        // The raw pixels the encoder was handed, so a foreign decoder can be
        // compared against the renderer's own output rather than only against
        // the PNG being well-formed.
        let raw = format!("{out}/mark-{size}.rgba");
        std::fs::write(&raw, raster::render_framed(Scheme::Light, size)).expect("write raw");
        println!("wrote {path}");
    }
}
