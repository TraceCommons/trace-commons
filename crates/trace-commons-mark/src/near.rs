//! The NEAR AI "N", for the Linux app icon.
//!
//! The Linux desktop shows the NEAR AI mark, not "The Turn" (owner,
//! 2026-11-18). Its path data is the brand kit's, as the Mac carries it in
//! `macos/Sources/TCDesign/Logos/GlassBrandMark.swift`; the test below reads
//! that file, so the two cannot drift apart.
//!
//! The app icon is built as the Mac's is (`macos/icons/README.md`): the
//! kit's tile, a `brand-tile` #0B8FF6 ground with the N in `brand-paper`
//! #F3F3F2 at the kit's own proportion, 58.5% of the tile's width, centred.
//! Neither ink is recoloured. A freedesktop icon is one file for light and
//! dark desktops alike, so it keeps the coloured ground rather than a bare
//! single-ink N that would vanish on one of them.
//!
//! The Mac's icon is a full-bleed square the system masks. No Linux desktop
//! masks an icon, so the tile carries its own rounded corners: the radius
//! macOS masks app icons with, 22.5% of the side.

/// The N's own bounds inside the kit's tile: x, y, width, height.
pub const VIEW_BOX: [f64; 4] = [82.96, 82.13, 234.08, 231.72];

/// The kit's path, spaces written as commas, exactly as `GlassBrandMark.data`.
pub const PATH: &str = "M292.106,82.1357C283.445,82.1357,275.379,86.5848,270.847,93.9047L221.912,165.811C220.318,168.188,220.971,171.382,223.353,172.96C225.292,174.234,227.866,174.082,229.633,172.58L277.799,131.227C278.605,130.504,279.834,130.581,280.545,131.379C280.872,131.74,281.044,132.216,281.044,132.691V262.169C281.044,263.233,280.18,264.089,279.085,264.089C278.509,264.089,277.952,263.842,277.587,263.404L132.014,90.8817C127.271,85.3489,120.318,82.1548,112.982,82.1357H107.893C94.1229,82.1357,82.9648,93.1822,82.9648,106.814V289.167C82.9648,302.799,94.1229,313.846,107.893,313.846C116.554,313.846,124.62,309.397,129.153,302.077L178.087,230.17C179.681,227.793,179.028,224.599,176.646,223.021C174.707,221.747,172.133,221.899,170.366,223.401L122.2,264.754C121.394,265.477,120.165,265.401,119.454,264.602C119.128,264.241,118.955,263.766,118.955,263.29V133.775C118.955,132.71,119.819,131.854,120.914,131.854C121.49,131.854,122.047,132.102,122.412,132.539L267.966,305.1C272.709,310.632,279.662,313.827,286.998,313.846H292.087C305.857,313.846,317.034,302.799,317.034,289.167V106.833C317.034,93.2012,305.876,82.1548,292.106,82.1548V82.1357Z";

/// The app icon's ground, `brand-tile`, as the Mac's `AppIcon.icon` fills it.
pub const TILE: &str = "#0B8FF6";

/// The N on the tile, `brand-paper`.
pub const PAPER: &str = "#F3F3F2";

/// The kit's proportion: the N is this share of the tile's width.
pub const N_SHARE: f64 = 0.585;

/// The tile's corner radius as a share of its side.
pub const CORNER_SHARE: f64 = 0.225;

/// The tile's side in the document's own units, the kit's 400.
const SIDE: f64 = 400.0;

/// The app icon: the N in paper on the brand tile, `size` pixels across.
pub fn app_icon_svg(size: u32) -> String {
    let [bx, by, bw, bh] = VIEW_BOX;
    let scale = SIDE * N_SHARE / bw;
    let tx = (SIDE - bw * scale) / 2.0 - bx * scale;
    let ty = (SIDE - bh * scale) / 2.0 - by * scale;
    let r = SIDE * CORNER_SHARE;
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 400 400">"#,
            r#"<rect width="400" height="400" rx="{r}" fill="{tile}"/>"#,
            r#"<path transform="matrix({s} 0 0 {s} {tx} {ty})" d="{path}" fill="{paper}"/>"#,
            r#"</svg>"#,
        ),
        size = size,
        r = fmt(r),
        tile = TILE,
        s = fmt(scale),
        tx = fmt(tx),
        ty = fmt(ty),
        path = PATH,
        paper = PAPER,
    )
}

/// The N alone in one ink, `size` pixels across, for a surface that wants
/// the mark without a tile: the logo kit's black on light, white on dark.
pub fn template_svg(ink: &str, size: u32) -> String {
    let [bx, by, bw, bh] = VIEW_BOX;
    let side = bw.max(bh);
    let x = bx - (side - bw) / 2.0;
    let y = by - (side - bh) / 2.0;
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" "#,
            r#"viewBox="{x} {y} {side} {side}"><path d="{path}" fill="{ink}"/></svg>"#,
        ),
        size = size,
        x = fmt(x),
        y = fmt(y),
        side = fmt(side),
        path = PATH,
        ink = ink,
    )
}

/// Four decimals at most, trailing zeros dropped: the same bytes on every
/// runner, so the drift check can compare them.
fn fmt(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swift() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../macos/Sources/TCDesign/Logos/GlassBrandMark.swift"
        );
        std::fs::read_to_string(path).expect("GlassBrandMark.swift")
    }

    /// The path and its bounds are the Mac's, character for character.
    #[test]
    fn the_n_is_the_macs_path() {
        let source = swift();
        assert!(source.contains(&format!("\"{PATH}\"")), "path differs");
        assert!(
            source.contains("CGRect(x: 82.96, y: 82.13, width: 234.08, height: 231.72)"),
            "viewBox differs"
        );
        assert_eq!(VIEW_BOX, [82.96, 82.13, 234.08, 231.72]);
    }

    /// The tile's inks and proportion are the Mac app icon's.
    #[test]
    fn the_app_icon_is_the_macs_tile() {
        let icon = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../macos/icons/production/AppIcon.icon"
        );
        let json = std::fs::read_to_string(format!("{icon}/icon.json")).expect("icon.json");
        // 0x0B/255, 0x8F/255, 0xF6/255, as Icon Composer writes the fill.
        assert!(
            json.contains("srgb:0.04314,0.56078,0.96471,1.00000"),
            "tile ground moved"
        );
        let n = std::fs::read_to_string(format!("{icon}/Assets/near-n.svg")).expect("near-n.svg");
        assert!(n.contains(r##"fill="#F3F3F2""##), "paper moved");
        // The Mac's N layer: the same path, spaces for commas.
        assert!(
            n.contains(&PATH.replace(',', " ")),
            "the Mac icon's N moved"
        );
        assert_eq!((TILE, PAPER, N_SHARE), ("#0B8FF6", "#F3F3F2", 0.585));

        let doc = app_icon_svg(512);
        assert!(doc.contains(r#"width="512" height="512""#), "{doc}");
        assert!(doc.contains(r##"fill="#0B8FF6""##), "{doc}");
        assert!(doc.contains(r##"fill="#F3F3F2""##), "{doc}");
        assert!(doc.contains(PATH), "{doc}");
        // 400 * 0.585 / 234.08: the N spans 58.5% of the tile.
        assert!(doc.contains("matrix(0.9997 0 0 0.9997 "), "{doc}");
        assert!(doc.contains(r#"rx="90""#), "{doc}");
    }

    /// The N, transformed, sits centred on the tile.
    #[test]
    fn the_n_is_centred_on_the_tile() {
        let [bx, by, bw, bh] = VIEW_BOX;
        let s = SIDE * N_SHARE / bw;
        let tx = (SIDE - bw * s) / 2.0 - bx * s;
        let ty = (SIDE - bh * s) / 2.0 - by * s;
        let left = bx * s + tx;
        let right = SIDE - ((bx + bw) * s + tx);
        let top = by * s + ty;
        let bottom = SIDE - ((by + bh) * s + ty);
        assert!((left - right).abs() < 1e-9 && (top - bottom).abs() < 1e-9);
        assert!((bw * s / SIDE - 0.585).abs() < 1e-12);
    }

    /// The single-ink N: one ink, no tile, never the product's old name.
    #[test]
    fn the_template_is_one_ink_without_a_tile() {
        let doc = template_svg("#000000", 16);
        assert!(!doc.contains("<rect"), "{doc}");
        assert!(doc.contains(r##"fill="#000000""##), "{doc}");
        assert!(!doc.contains(TILE), "{doc}");
        assert!(
            doc.contains(r#"viewBox="82.96 80.95 234.08 234.08""#),
            "{doc}"
        );
        for doc in [template_svg("#FFFFFF", 16), app_icon_svg(64)] {
            assert!(!doc.contains("Trace Commons"), "{doc}");
            assert!(!doc.contains("aria-label"), "{doc}");
        }
    }
}
