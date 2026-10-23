//! Write every staging icon option as an SVG glyph layer, for previews.
//! Usage: `staging-options <dir>`; writes `<option>-<scheme>.svg`.
use trace_commons_mark::{Scheme, StagingOption, staging_glyph_svg};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: staging-options <dir>");
    std::fs::create_dir_all(&dir).expect("create dir");
    for option in StagingOption::ALL {
        for scheme in Scheme::ALL {
            let path = format!("{dir}/{}-{}.svg", option.name(), scheme.name());
            std::fs::write(&path, staging_glyph_svg(option, scheme, 1024)).expect("write");
            println!("wrote {path}");
        }
    }
}
