//! The community brand, as one stylesheet installed once.
//!
//! Two surfaces in this window are drawn in the *site's* visual language
//! rather than this window's: History's Community panel (§5.5) and
//! Settings' public-profile block with its go-public dialog (§5.6, §5.7).
//! Both are pictures of what becomes public, so both are set in 2px black
//! frames, Helvetica, uppercase display type and mint, with no rounding
//! anywhere inside a framed panel.
//!
//! ## Why these hexes are not `tc_*` tokens
//!
//! The `tc_*` token set in `style.rs` is the *native* palette and follows
//! the desktop's light/dark preference. The community brand is a second
//! palette (§2.2) and it is **light-only** -- the site declares
//! `color-scheme: light`. A black-framed panel must be black in both
//! schemes, because it is a picture of the public web page rather than a
//! piece of this window; the seam is the design. Putting `#000000` /
//! `#00d4aa` / `#eafaf5` in the token layer would make them look like they
//! participate in the scheme flip. They do not. So every colour below is
//! stated literally, and every rule sets both its background and its
//! foreground -- a class that inherited either one would follow the scheme
//! and break the seam.
//!
//! ## Why this is a separate provider
//!
//! `style.rs` reloads its own provider on every scheme flip. This
//! stylesheet must not be part of that: it never changes. It is added one
//! step above the application stylesheet so a brand panel's black frame
//! wins over the native card rules it would otherwise inherit, and
//! `style.rs` stays the only place the native tokens live.
//!
//! ## Why it is one stylesheet
//!
//! History and Settings both draw brand panels and both name their nodes
//! `tc-brand-*`. Two providers defining the same class names at the same
//! priority would resolve by install order, which is not a design
//! decision -- it is whichever view happened to be constructed first. One
//! provider, installed once, removes the question.

use std::cell::Cell;

/// The brand palette and type scale, §2.2 and §3.3, as CSS classes.
///
/// Values: `brand.ink` `#000000`, `brand.paper` `#ffffff`,
/// `brand.accent` `#00d4aa`, `brand.rim` `#00b894`, `brand.tint` `#eafaf5`,
/// `brand.muted` `#6b6b6b`. Frames 2px, internal dividers and field boxes
/// 1px, radius 0 everywhere (§4.2, §4.3). Type is §3.3: `display.panel`,
/// `display.dialog`, `figure.brand`, `field.value`, `label.mono`,
/// `body.brand`; buttons and the text link are §6.1, the bare checkbox is
/// §6.9.
///
/// Letter-spacing is written in px because GTK CSS has no `em` here; each
/// value is the spec's em figure multiplied by its own font size. GTK 4 CSS
/// implements neither `line-height` nor `text-transform`, so the spec's
/// leading is left to the font and the uppercasing is done in Rust at the
/// call sites.
const CSS: &str = r#"
/* The panel and the dialog ground. Radius 0 is not an omission -- §4.2
   states no rounding anywhere inside a black-framed panel. */
.tc-brand-panel,
.tc-brand-surface {
  background-color: #ffffff;
  color: #000000;
  border-radius: 0;
}

.tc-brand-panel {
  border: 2px solid #000000;
  padding: 14px;
}

/* Helvetica is the site's face (§3.1). A Linux desktop usually resolves it
   to a metric substitute, which is the correct outcome: what has to survive
   is that the public surface is set in a DIFFERENT face from the window
   around it. Liberation Sans and Nimbus Sans are named after Arial rather
   than before it, so the spec's own order still decides wherever the spec's
   fonts are present -- they are the tail that keeps the seam on a box where
   none of the three are. */
.tc-brand-panel,
.tc-brand-surface,
.tc-brand-display,
.tc-brand-dialog-title,
.tc-brand-body {
  font-family: "Helvetica Neue", Helvetica, Arial, "Liberation Sans", "Nimbus Sans", sans-serif;
}

/* `display.panel`: 700 / 24px / UPPERCASE / -.035em -> -0.84px. */
.tc-brand-display {
  font-size: 24px;
  font-weight: 700;
  letter-spacing: -0.84px;
  color: #000000;
}

/* `display.dialog`: 700 / 27px / UPPERCASE / -.035em -> -0.95px. */
.tc-brand-dialog-title {
  font-size: 27px;
  font-weight: 700;
  letter-spacing: -0.95px;
  color: #000000;
}

/* `figure.brand`: 700 / 26px mono, tabular figures, -.03em -> -0.78px. */
.tc-brand-figure {
  font-family: monospace;
  font-weight: 700;
  font-size: 26px;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.78px;
  color: #000000;
}

/* `label.mono`: every micro label on a brand surface, in the one grey the
   brand allows on paper. 700 / 11px mono / UPPERCASE / .02em -> 0.22px. */
.tc-brand-label {
  font-family: monospace;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.22px;
  color: #6b6b6b;
}

/* `body.brand`: panel prose and notice boxes. 500 / 13px / -.01em. */
.tc-brand-body {
  font-size: 13px;
  font-weight: 500;
  letter-spacing: -0.13px;
  color: #000000;
}

/* `field.value`: 500 / 15px, mono for the handle and sans for the bio.
   The 1px field box, not the 2px panel frame. */
.tc-brand-field {
  border: 1px solid #000000;
  border-radius: 0;
  background-color: #ffffff;
  background-image: none;
  box-shadow: none;
  color: #000000;
  padding: 8px 12px;
  font-size: 15px;
  font-weight: 500;
  letter-spacing: -0.15px;
  min-height: 0;
}

.tc-brand-field.tc-brand-mono {
  font-family: monospace;
}

/* GtkEntry and GtkTextView both draw their own inner node, which keeps the
   theme's ground unless it is told otherwise. */
.tc-brand-field > text,
.tc-brand-bio,
.tc-brand-bio text {
  background-color: #ffffff;
  background-image: none;
  color: #000000;
}

.tc-brand-bio {
  min-height: 56px;
  font-size: 15px;
  font-weight: 500;
  letter-spacing: -0.15px;
}

/* The boxed structures: a 2px frame around N cells split by 1px. This is
   the site's table, not a row of cards -- History's metric strip and
   Settings' published/never columns are the same component. */
.tc-brand-box {
  border: 2px solid #000000;
  border-radius: 0;
  background-color: #ffffff;
}

/* The divider is opt-in rather than opt-out: a cell has a rule to its right
   when something follows it, which is a fact the call site knows and the
   stylesheet does not. */
.tc-brand-cell {
  padding: 12px 14px;
}

.tc-brand-cell.tc-brand-divided {
  border-right: 1px solid #000000;
}

/* `brand.tint` behind an acknowledgement or a withheld notice. */
.tc-brand-notice {
  border: 2px solid #000000;
  border-radius: 0;
  background-color: #eafaf5;
  color: #000000;
  padding: 12px 14px;
}

/* §6.1's brand pair. Secondary is white, primary is mint; both carry the
   same 2px frame and the same mono uppercase label, so the difference
   between them is the fill and nothing else. */
.tc-brand-button {
  border: 2px solid #000000;
  border-radius: 0;
  background-color: #ffffff;
  background-image: none;
  box-shadow: none;
  color: #000000;
  padding: 10px 16px;
  min-height: 0;
  font-family: monospace;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.22px;
}

.tc-brand-button.tc-brand-primary {
  background-color: #00d4aa;
}

/* §6.1: the disabled brand primary is the fill at .4, not a grey. */
.tc-brand-button:disabled {
  opacity: 0.4;
  color: #000000;
}

.tc-brand-button:hover {
  background-color: #eafaf5;
}

.tc-brand-button.tc-brand-primary:hover {
  background-color: #00b894;
}

/* §6.9's brand checkbox: a bare 14px square with a 2px frame and no fill.
   Adwaita draws the tick as an icon in the node's colour, so the colour is
   set rather than the background. */
.tc-brand-check check {
  min-width: 14px;
  min-height: 14px;
  border: 2px solid #000000;
  border-radius: 0;
  background-color: #ffffff;
  background-image: none;
  box-shadow: none;
  color: #000000;
}

.tc-brand-check {
  color: #000000;
}

/* §6.1's brand text link: 700 / 11px mono, uppercase, underlined. The
   underline is drawn by the link widget or as Pango markup on the label,
   since GTK 4 CSS has no text-decoration. GtkLinkButton keeps its label in
   a child node, so the type is stated on both. */
.tc-brand-link,
.tc-brand-link label {
  background: none;
  background-color: transparent;
  background-image: none;
  border: none;
  box-shadow: none;
  outline: none;
  padding: 0;
  min-height: 0;
  color: #000000;
  font-family: monospace;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.22px;
}

/* §5.7's dialog footnote: 11px / 500 on `brand.muted`. Not `label.mono` --
   this one is a sentence, not a micro-label. */
.tc-brand-footnote {
  font-size: 11px;
  font-weight: 500;
  color: #6b6b6b;
}
"#;

/// Install the community stylesheet for the default display, once.
///
/// Every view that draws a `tc-brand-*` node calls this before it builds
/// one. The guard is what makes that safe: repeated calls add nothing, so
/// no view has to know whether another view got there first.
pub fn install() {
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
}

#[cfg(test)]
mod tests {
    use super::CSS;

    #[test]
    fn nothing_inside_a_brand_panel_is_rounded() {
        // §4.2: radius 0 everywhere inside a black-framed panel. Any
        // non-zero radius here would be the native language leaking across
        // the seam.
        for declaration in CSS.split("border-radius:").skip(1) {
            let value = declaration
                .split(';')
                .next()
                .expect("a declaration ends in a semicolon")
                .trim();
            assert_eq!(value, "0", "brand surfaces are never rounded");
        }
    }

    #[test]
    fn the_brand_palette_is_light_only_and_literal() {
        // The seam depends on these being fixed values rather than tokens
        // that follow the desktop's scheme. If a `@` reference or a
        // scheme-dependent colour ever appears here, a black-framed panel
        // stops being black in dark mode.
        assert!(!CSS.contains('@'));
        for hex in ["#000000", "#ffffff", "#eafaf5", "#6b6b6b", "#00d4aa"] {
            assert!(CSS.contains(hex), "{hex} is part of the brand palette");
        }
    }
}
