//! The one place this shell's visual decisions live.
//!
//! ## The direction: a customs declaration, not a feed
//!
//! This application stands between a developer's private transcripts and a
//! public research pool, and the only question its interface has to answer
//! is "what exactly is about to leave this machine, and can I stop it". So
//! the surfaces are built like a declaration form rather than a stream of
//! content: every session is one card, every card carries the SAME fields
//! in the SAME order, and each card ends in a fixed manifest strip set in
//! monospaced type.
//!
//! The repetition is the point. When every card's outbound facts land in
//! the same place, a person stops reading and starts scanning, and the row
//! that is different -- a large payload, a session where scrubbing matched
//! nothing -- is a break in a rhythm rather than a sentence they have to
//! notice. It is the one deliberately bold move in an otherwise quiet
//! window, and everything else is kept plain so it can carry.
//!
//! ## Family resemblance to the community site
//!
//! `community/public/styles.css` is the other face of this product. What is
//! carried across is the ROLES each colour plays: the accent is primary and
//! means "good standing", gold means "weigh this", coral means "refused",
//! blue means "held or ranked". Also the warm off-white ground rather than a
//! neutral grey, 6-8px radii with pill badges, hairlines instead of shadows,
//! and heavy uppercase micro labels over data. The accent is no longer the
//! site's green; see below.
//!
//! What is deliberately not carried across: Inter is not bundled. A font
//! file is a real download and packaging cost for a brand cue, and the
//! site's 680/760/800 weights are reproduced on the system face, which is
//! what a Linux desktop's user has already calibrated their eye against.
//!
//! ## Overriding the GNOME accent, on purpose: purple
//!
//! GNOME convention is that applications follow the user's chosen system
//! accent colour. This one does not: it pins the Trace Commons brand
//! purple. That is a deliberate product decision -- the owner ruled
//! "purple everywhere" on 2026-10-02, so this window wears the same accent
//! as the macOS and Windows apps, and the accent is also a role marker
//! ("good standing", the one action), not decoration. It replaces the site
//! green this file used to pin. The decision is provisional until the
//! designer's final brand lands; when it does, the tokens change and this
//! file does not.
//!
//! **This is not a bug. Please do not "fix" it back to
//! `AdwStyleManager`'s accent.** If it is ever reverted, it should be
//! reverted as a product decision, not as a theming cleanup.
//!
//! ## The accent and status colours are generated
//!
//! The accent and the status hues are not written here. They come from
//! `design-tokens/glass.tokens.json`, the macOS glass system's tokens, via
//! `scripts/design-tokens/generate.py`, which writes [`brand_tokens`]: one
//! `@define-color` sheet per scheme, loaded ahead of the palette below,
//! which maps this shell's `tc_*` names onto them. Edit the JSON and
//! regenerate; `generate.py --check` fails CI on a hand edit. The grounds,
//! ink, blue and the redaction wash stay this shell's own.
//!
//! ## Dark comes from the design spec, not from an inversion
//!
//! The site has no `prefers-color-scheme` block anywhere, so there was no
//! dark palette to copy from it -- and on this platform dark is not
//! optional, since a large share of GNOME and KDE users run it permanently.
//! The dark values below are now the approved mockups' native palette
//! (`design-import/DESIGN-SPEC.md` §2.1) rather than this file's earlier
//! derivation, and they hold the same relations:
//!
//! * The ground (`#23251D`) is not a neutral grey; it keeps the warm green
//!   cast of the light ground (`#F6F7F4`) at the other end of the scale
//!   rather than the blue-black a naive inversion produces.
//! * ground / surface / inset keep the same order and roughly the same
//!   perceptual spacing as `--bg` / `--surface` / `--surface-2`.
//! * Every accent keeps its hue and its role and is lifted in lightness
//!   until it clears text contrast on the dark ground.
//!
//! ## Where the tokens come from
//!
//! Every `tc_*` ground, ink, blue and redaction colour below is one row of
//! §2.1's native-palette table, and the mapping is recorded inline; the
//! accent and status rows are the generated tokens instead (above). Two
//! rows are deliberately absent:
//! `bg.sidebar.macos` and `bg.chrome.windows` are other platforms' chrome
//! and have no surface in this shell.
//!
//! ## Contrast is measured, never eyeballed
//!
//! Every ratio quoted below is a computed WCAG 2.1 relative-luminance
//! figure, not a judgement, and `tests::` below recomputes them from the
//! values this file actually loads. The one that matters most is the filled
//! primary action, `Contribute` -- the one irreversible control in the
//! product. The accent fill carries a white label at 6.90:1 light and
//! 5.01:1 dark, and because libadwaita derives `.suggested-action` from
//! `accent_bg_color`/`accent_fg_color`, setting that pair here fixes every
//! suggested action in the window at once.
//!
//! The dark fill purple (`#8A3DFF`) is a fill, not a line: it clears 3:1 on
//! the window ground and a card but measures 2.76:1 on the inset well. So
//! a border or rule drawn straight on a ground (`tc_green`, below) takes
//! `tc_accent_line`, which is the text purple in dark.

use adw::prelude::*;
use gtk::gdk;

use super::omarchy;

#[path = "brand_tokens.rs"]
pub mod brand_tokens;

/// Light tokens.
///
/// The first block is the design spec's native palette, §2.1's light
/// column. The accent and status values are the generated
/// [`brand_tokens::LIGHT_CSS`], each in a fill/rule weight and a text-safe
/// twin that clears 4.5:1 as small type.
///
/// Measured (WCAG 2.1, computed not estimated):
///
/// ```text
///  14.64:1  ink         #20241F on bg         #F6F7F4
///  15.75:1  ink         #20241F on surface    #FFFFFF
///   5.76:1  muted       #5C635B on bg         #F6F7F4
///   6.19:1  muted       #5C635B on surface    #FFFFFF
///   5.48:1  muted       #5C635B on surface-2  #EEF2F0
///   4.58:1  tertiary    #6D7269 on bg         #F6F7F4   (see the note below)
///   4.93:1  tertiary    #6D7269 on surface    #FFFFFF
///   8.17:1  accent text #5B10CC on bg         #F6F7F4
///   7.78:1  accent text #5B10CC on surface-2  #EEF2F0
///   6.95:1  gold text   #7A4A06 on bg         #F6F7F4
///   6.62:1  gold text   #7A4A06 on surface-2  #EEF2F0
///   6.97:1  coral text  #A3221C on bg         #F6F7F4
///   6.04:1  blue text   #315FBA on surface    #FFFFFF
///   6.90:1  PRIMARY     #FFFFFF on accent     #6D14F3   <- the consent action
///   6.11:1  accent rule #6D14F3 on surface-2  #EEF2F0   (non-text, >= 3:1)
///   3.96:1  gold rule   #A8690F on surface-2  #EEF2F0   (non-text, >= 3:1)
///  12.34:1  redaction   #202426 on gold wash  #F3E3C0
///   3.27:1  (rejected)  spec ink.tertiary #8A9086 on surface #FFFFFF
/// ```
///
/// **The one deviation from §2.1.** The spec's `ink.tertiary` is `#8A9086`
/// light / `#82887C` dark, and the spec assigns it to timestamps, eyebrow
/// labels and footnotes -- all small text. Measured, it is 3.27:1 on white
/// and 4.26:1 on the dark ground, so it fails the 4.5:1 body-text floor at
/// both ends. `tc_ink_tertiary` therefore carries the nearest accessible
/// twin on the same hue and saturation (`#6D7269` / `#878D81`, 4.58:1 and
/// 4.55:1 on their grounds), which keeps the three-step ink ramp the spec
/// asks for without putting sub-threshold type on a screen. It must not be
/// used on the inset surface, where the dark twin measures 4.05:1; small
/// type inside a manifest strip stays on `tc_muted`.
const LIGHT_TOKENS: &str = r#"
/* --- Ground and ink, from DESIGN-SPEC §2.1 (light column) --------- */
@define-color tc_bg        #F6F7F4;   /* bg.window */
@define-color tc_surface   #FFFFFF;   /* surface.card */
@define-color tc_surface2  #EEF2F0;   /* surface.inset */
@define-color tc_surface_inset @tc_surface2;   /* the spec's name for it */
@define-color tc_scrim     rgba(0, 0, 0, 0.06);   /* surface.scrim */
@define-color tc_selected  rgba(0, 0, 0, 0.07);   /* surface.selected */
@define-color tc_ink       #20241F;   /* ink.primary */
@define-color tc_muted     #5C635B;   /* ink.secondary */
@define-color tc_ink_tertiary #6D7269;   /* ink.tertiary, contrast-corrected */
@define-color tc_line      #D9DFDC;   /* hairline */
@define-color tc_line_divider #DDDFD8;   /* hairline.divider */

/* --- Accent and status. Fills, rules and glyph strokes only. The
       accent and status values are the generated brand_tokens sheet,
       loaded ahead of this one. tc_green keeps its name for the
       stylesheet's sake; it is the purple accent. ------------------- */
@define-color tc_green     @tc_accent_line;   /* purple */
@define-color tc_blue      #315FBA;   /* blue.brand */
@define-color tc_coral     @tc_status_outside;
@define-color tc_gold      @tc_status_ask;
@define-color tc_gold_highlight rgba(185, 130, 31, 0.28);   /* gold.highlight */

/* --- Text-safe twins. Type only; fills and rules keep the values
       above. See the note on this constant. ------------------------ */
@define-color tc_green_text @tc_accent_text;
@define-color tc_blue_text  #315FBA;
/* §2.1 lists blue.brand as #315FBA and blue.icon as #315FBB, one digit
   apart; standardised on #315FBA, which is the mark's own blue. */
@define-color tc_blue_icon  #315FBA;
@define-color tc_coral_text @tc_status_outside_text;
@define-color tc_gold_text  @tc_status_ask_text;

/* --- The filled primary action, as a measured pair ---------------- */
@define-color tc_primary_fill  @tc_accent;
@define-color tc_primary_label @tc_on_accent;

/* --- Where scrubbing fired, in the transcript --------------------- */
@define-color tc_redaction_bg  #F3E3C0;
@define-color tc_redaction_fg  #202426;

/* --- The libadwaita palette, recoloured wholesale ------------------
   Partial overrides are worse than none: a brand ground under theme
   cards reads as broken in a way neither pure theme nor pure brand
   does. Adwaita's own stylesheet is written against these names, so
   redefining them recolours the widgets this file never mentions. */
@define-color accent_bg_color      @tc_primary_fill;
@define-color accent_fg_color      @tc_primary_label;
@define-color accent_color         @tc_green_text;
@define-color window_bg_color      @tc_bg;
@define-color window_fg_color      @tc_ink;
@define-color view_bg_color        @tc_surface;
@define-color view_fg_color        @tc_ink;
@define-color headerbar_bg_color   @tc_bg;
@define-color headerbar_fg_color   @tc_ink;
@define-color headerbar_border_color @tc_line;
@define-color headerbar_backdrop_color @tc_bg;
@define-color card_bg_color        @tc_surface;
@define-color card_fg_color        @tc_ink;
@define-color popover_bg_color     @tc_surface;
@define-color popover_fg_color     @tc_ink;
@define-color dialog_bg_color      @tc_surface;
@define-color dialog_fg_color      @tc_ink;
/* A status fill carries a label, so in light it takes the text-safe
   twin under white rather than the rule weight. */
@define-color warning_bg_color     @tc_status_ask_text;
@define-color warning_fg_color     #FFFFFF;
@define-color warning_color        @tc_gold_text;
@define-color error_bg_color       @tc_status_outside_text;
@define-color error_fg_color       #FFFFFF;
@define-color error_color          @tc_coral_text;
@define-color destructive_bg_color @tc_status_outside_text;
@define-color destructive_fg_color #FFFFFF;
@define-color success_bg_color     @tc_status_on_text;
@define-color success_fg_color     #FFFFFF;
@define-color success_color        @tc_status_on_text;
@define-color theme_bg_color       @tc_bg;
@define-color theme_fg_color       @tc_ink;
@define-color theme_base_color     @tc_surface;
@define-color theme_text_color     @tc_ink;
@define-color borders              @tc_line;
"#;

/// Dark tokens. §2.1's dark column, with the same corrections.
///
/// Measured (WCAG 2.1, computed not estimated):
///
/// ```text
///  12.79:1  ink         #E8EAE3 on bg         #23251D
///  12.96:1  ink         #E8EAE3 on surface    #21241E
///   6.75:1  muted       #A6AC9F on surface    #21241E
///   6.66:1  muted       #A6AC9F on bg         #23251D
///   5.94:1  muted       #A6AC9F on surface-2  #2A2E27
///   4.55:1  tertiary    #878D81 on bg         #23251D   (see the light note)
///   4.61:1  tertiary    #878D81 on surface    #21241E
///   7.49:1  accent text #C9B3FF on surface-2  #2A2E27
///   8.29:1  gold text   #F5C142 on surface-2  #2A2E27
///   6.53:1  coral text  #FF9494 on surface-2  #2A2E27
///   7.78:1  blue text   #9DB6F1 on surface    #21241E
///   7.68:1  blue icon   #9DB6F1 on bg         #23251D
///   5.01:1  PRIMARY     #FFFFFF on accent     #8A3DFF   <- the consent action
///   3.10:1  accent fill #8A3DFF on bg         #23251D   (non-text, >= 3:1)
///   2.76:1  (not drawn) accent fill #8A3DFF on surface-2 #2A2E27
///   7.49:1  accent rule #C9B3FF on surface-2  #2A2E27
///   9.04:1  redaction   #F0EBDD on gold wash  #4A3C18
///   4.26:1  (rejected)  spec ink.tertiary #82887C on bg #23251D
/// ```
///
/// Dark keeps the white primary label: the brighter purple carries it at
/// 5:1, where the old dark mint had to flip its label to near-black.
const DARK_TOKENS: &str = r#"
/* --- Ground and ink, from DESIGN-SPEC §2.1 (dark column). Warm
       near-black carrying the light ground's green cast, not the
       blue-black a naive inversion produces. ------------------------ */
@define-color tc_bg        #23251D;
@define-color tc_surface   #21241E;
@define-color tc_surface2  #2A2E27;
@define-color tc_surface_inset @tc_surface2;
@define-color tc_scrim     rgba(255, 255, 255, 0.08);
@define-color tc_selected  rgba(255, 255, 255, 0.1);
@define-color tc_ink       #E8EAE3;
@define-color tc_muted     #A6AC9F;
@define-color tc_ink_tertiary #878D81;   /* contrast-corrected, see LIGHT */
@define-color tc_line      #3B4038;
@define-color tc_line_divider #373A33;

/* --- Same hue, same role, lifted until it clears the dark ground -- */
@define-color tc_green     @tc_accent_line;   /* purple, see LIGHT */
@define-color tc_blue      #7FA0EC;
@define-color tc_coral     @tc_status_outside;
@define-color tc_gold      @tc_status_ask;
@define-color tc_gold_highlight rgba(220, 170, 67, 0.32);

@define-color tc_green_text @tc_accent_text;
@define-color tc_blue_text  #9DB6F1;
@define-color tc_blue_icon  #9DB6F1;
@define-color tc_coral_text @tc_status_outside_text;
@define-color tc_gold_text  @tc_status_ask_text;

@define-color tc_primary_fill  @tc_accent;
@define-color tc_primary_label @tc_on_accent;

@define-color tc_redaction_bg  #4A3C18;
@define-color tc_redaction_fg  #F0EBDD;

@define-color accent_bg_color      @tc_primary_fill;
@define-color accent_fg_color      @tc_primary_label;
@define-color accent_color         @tc_green_text;
@define-color window_bg_color      @tc_bg;
@define-color window_fg_color      @tc_ink;
@define-color view_bg_color        @tc_surface;
@define-color view_fg_color        @tc_ink;
@define-color headerbar_bg_color   @tc_bg;
@define-color headerbar_fg_color   @tc_ink;
@define-color headerbar_border_color @tc_line;
@define-color headerbar_backdrop_color @tc_bg;
@define-color card_bg_color        @tc_surface;
@define-color card_fg_color        @tc_ink;
@define-color popover_bg_color     @tc_surface;
@define-color popover_fg_color     @tc_ink;
@define-color dialog_bg_color      @tc_surface;
@define-color dialog_fg_color      @tc_ink;
/* In dark the status hues are light, so a fill takes the near-black
   label instead of white. */
@define-color warning_bg_color     @tc_status_ask;
@define-color warning_fg_color     @tc_on_status;
@define-color warning_color        @tc_gold_text;
@define-color error_bg_color       @tc_status_outside;
@define-color error_fg_color       @tc_on_status;
@define-color error_color          @tc_coral_text;
@define-color destructive_bg_color @tc_status_outside;
@define-color destructive_fg_color @tc_on_status;
@define-color success_bg_color     @tc_status_on;
@define-color success_fg_color     @tc_on_status;
@define-color success_color        @tc_status_on_text;
@define-color theme_bg_color       @tc_bg;
@define-color theme_fg_color       @tc_ink;
@define-color theme_base_color     @tc_surface;
@define-color theme_text_color     @tc_ink;
@define-color borders              @tc_line;
"#;

/// The 4px spacing rhythm. Widget code should not write raw numbers; if a
/// value is missing here it is probably the wrong value.
pub mod space {
    pub const XXS: i32 = 4;
    pub const XS: i32 = 6;
    pub const S: i32 = 8;
    pub const M: i32 = 12;
    pub const L: i32 = 16;
    pub const XL: i32 = 20;
    pub const XXL: i32 = 28;
}

/// What a piece of information means, expressed as a colour AND a glyph AND
/// (at the call site) words. Never the colour on its own -- the state has to
/// survive greyscale, colour blindness, and a screenshot printed in black
/// and white.
///
/// The mapping is the site's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Ordinary, nothing to weigh.
    Neutral,
    /// A question that got a clean answer. Good standing.
    Clear,
    /// Something was found, or something cannot be checked. Caution, not
    /// alarm: a window that shouts on every row teaches people to stop
    /// looking.
    Attention,
    /// Held, waiting on somebody else. Not a failure.
    Held,
    /// Refused, withdrawn, or unavailable.
    Refused,
}

impl Tone {
    pub fn css(self) -> &'static str {
        match self {
            Tone::Neutral => "tc-neutral",
            Tone::Clear => "tc-clear",
            Tone::Attention => "tc-attention",
            Tone::Held => "tc-held",
            Tone::Refused => "tc-refused",
        }
    }

    /// A glyph, so the state is legible without colour. Restricted to
    /// characters DejaVu carries, since that is what a bare container run
    /// and many minimal desktops actually have.
    pub fn glyph(self) -> &'static str {
        match self {
            Tone::Neutral => "\u{00b7}",
            Tone::Clear => "\u{2713}",
            Tone::Attention => "!",
            Tone::Held => "\u{25f4}",
            Tone::Refused => "\u{2715}",
        }
    }
}

/// Install the stylesheet, and keep it in step with the system's light/dark
/// preference.
///
/// One provider for the whole application. It is reloaded rather than
/// duplicated when the scheme flips, because `@define-color` cannot be
/// scoped to a selector -- there is no `prefers-color-scheme` in GTK CSS --
/// so the palette has to be chosen at load time.
pub fn install() {
    // Idempotent because there are now two entry points: `App::build`, which
    // runs once a daemon exists, and the roots screen, which runs when one
    // does not. Without the guard the second caller would add a second
    // provider and a second `dark_notify` handler for the same stylesheet.
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    // The display is checked BEFORE the flag is set: marking the work done on
    // a call that could not do it would leave the application permanently
    // unstyled, which is a worse failure than installing twice.
    let Some(display) = gdk::Display::default() else {
        return;
    };
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let manager = adw::StyleManager::default();

    // A development hook, so a capture run can photograph both schemes on a
    // machine whose desktop is pinned to one. Unset -- the normal case --
    // the application follows the system exactly.
    match std::env::var("TRACE_COMMONS_APPEARANCE").as_deref() {
        Ok("dark") => manager.set_color_scheme(adw::ColorScheme::ForceDark),
        Ok("light") => manager.set_color_scheme(adw::ColorScheme::ForceLight),
        _ => {}
    }

    PROVIDER.with(|slot| *slot.borrow_mut() = Some(provider.clone()));
    load(&provider, manager.is_dark());
    let p = provider.clone();
    manager.connect_dark_notify(move |manager| load(&p, manager.is_dark()));

    // The Appearance choice. With Standard -- the default -- this changes
    // nothing above.
    super::appearance::start();
}

thread_local! {
    /// The one provider [`install`] made, for [`reload`] and [`set_translucent`].
    static PROVIDER: std::cell::RefCell<Option<gtk::CssProvider>> =
        const { std::cell::RefCell::new(None) };
    /// Whether the translucent glass sheet is loaded (`ui::glass_backdrop`).
    static TRANSLUCENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn load(provider: &gtk::CssProvider, dark: bool) {
    provider.load_from_data(&sheet_for_window(
        super::appearance::active_palette().as_ref(),
        dark,
        is_translucent(),
    ));
}

/// Load the stylesheet again into the existing provider, after the
/// appearance or its palette changed. Main thread only, like everything
/// that touches GTK; a no-op before `install`.
pub(crate) fn reload() {
    let provider = PROVIDER.with(|slot| slot.borrow().clone());
    if let Some(provider) = provider {
        load(&provider, adw::StyleManager::default().is_dark());
    }
}

/// Whether the translucent glass sheet is loaded.
pub fn is_translucent() -> bool {
    TRANSLUCENT.with(|on| on.get())
}

/// Load the translucent glass sheet over the painted one, or take it off.
/// Only `ui::glass_backdrop` calls this, and only while KDE's blur is
/// granted: without blur behind it a translucent pane fails the contrast
/// floors over a busy desktop.
pub fn set_translucent(on: bool) {
    if TRANSLUCENT.with(|current| current.replace(on)) == on {
        return;
    }
    reload();
}

/// What [`load`] loads: an Omarchy palette's sheet while the skin has one,
/// and it stays painted, since the blur sheets redefine the glass colours
/// the palette set; otherwise the standard sheet, translucent or not.
fn sheet_for_window(palette: Option<&omarchy::Palette>, dark: bool, translucent: bool) -> String {
    match palette {
        Some(_) => sheet_for(palette, dark),
        None => stylesheet_for(dark, translucent),
    }
}

/// The painted stylesheet for one scheme ([`stylesheet_for`]).
fn stylesheet(dark: bool) -> String {
    stylesheet_for(dark, false)
}

/// The whole stylesheet for one scheme: the generated accent and status
/// roles first, since the palette refers to them by name, then the palette,
/// then the rules. `translucent` appends the blur sheets last: they redefine
/// the glass colours, and GTK resolves a `@define-color` to its last
/// definition, so every rule naming one follows, hand-written ones included.
fn stylesheet_for(dark: bool, translucent: bool) -> String {
    let (brand, glass, tokens) = if dark {
        (
            brand_tokens::DARK_CSS,
            brand_tokens::GLASS_DARK_CSS,
            DARK_TOKENS,
        )
    } else {
        (
            brand_tokens::LIGHT_CSS,
            brand_tokens::GLASS_LIGHT_CSS,
            LIGHT_TOKENS,
        )
    };
    let insights = if dark {
        brand_tokens::INSIGHTS_DARK_CSS
    } else {
        brand_tokens::INSIGHTS_LIGHT_CSS
    };
    let mut sheet = format!(
        "{brand}\n{glass}\n{tokens}\n{}\n{insights}\n{}",
        include_str!("style.css"),
        super::hero_art::css()
    );
    if translucent {
        let (glass, insights) = if dark {
            (
                brand_tokens::GLASS_DARK_BLUR_CSS,
                brand_tokens::INSIGHTS_DARK_BLUR_CSS,
            )
        } else {
            (
                brand_tokens::GLASS_LIGHT_BLUR_CSS,
                brand_tokens::INSIGHTS_LIGHT_BLUR_CSS,
            )
        };
        sheet.push_str(&format!("\n{glass}\n{insights}"));
    }
    sheet
}

/// The accent the mark's open bracket is stroked in, for `scheme`.
///
/// The `trace-commons-mark` crate still carries the site mint for the macOS
/// shell and the icon pipeline; this window's mark wears the brand purple
/// with the rest of the window.
pub fn mark_accent(dark: bool) -> &'static str {
    if dark {
        brand_tokens::dark::ACCENT
    } else {
        brand_tokens::light::ACCENT
    }
}

/// The stylesheet `install` loads: the standard one for `dark`, or, while
/// the Omarchy skin has a palette, that palette's.
pub(crate) fn sheet_for(palette: Option<&omarchy::Palette>, dark: bool) -> String {
    match palette {
        Some(palette) => omarchy_stylesheet(palette),
        None => stylesheet(dark),
    }
}

/// The standard stylesheet with every colour value replaced from `palette`.
///
/// Structure is the standard sheet's for the palette's mode -- the same
/// blocks, rules and `@define-color` names -- and only values change:
/// every literal definition by its role name, and every literal colour in
/// the generated Insights rules by the role it plays there. Rules only the
/// skin has are appended, scoped under [`super::appearance::SKIN_CLASS`].
pub(crate) fn omarchy_stylesheet(palette: &omarchy::Palette) -> String {
    let dark = palette.mode == omarchy::Mode::Dark;
    let (brand, glass, tokens, insights) = if dark {
        (
            brand_tokens::DARK_CSS,
            brand_tokens::GLASS_DARK_CSS,
            DARK_TOKENS,
            brand_tokens::INSIGHTS_DARK_CSS,
        )
    } else {
        (
            brand_tokens::LIGHT_CSS,
            brand_tokens::GLASS_LIGHT_CSS,
            LIGHT_TOKENS,
            brand_tokens::INSIGHTS_LIGHT_CSS,
        )
    };
    let define = |css: &str| rewrite_defines(css, |name| omarchy_define(palette, name));
    let skin = super::appearance::SKIN_CLASS;
    let scoped = format!(
        ".{skin} textview text selection, .{skin} label selection, \
         .{skin} entry text selection {{ background-color: {}; color: {}; }}\n\
         .{skin} .dim-label {{ color: {}; opacity: 1; }}\n",
        palette.selection.hex(),
        palette.selection_ink.hex(),
        palette.tertiary.hex(),
    );
    format!(
        "{}\n{}\n{}\n{}\n{}\n{scoped}",
        define(brand),
        define(glass),
        define(tokens),
        include_str!("style.css"),
        recolour_insights(insights, palette, dark),
    )
}

/// The value the skin gives a `@define-color` that the standard sheet
/// defines as a literal. `None` keeps the standard value; only the neutral
/// translucent overlays are kept (`tc_scrim`, and `tc_selected` under
/// selected rows), since they darken or lighten whatever ground they sit on.
/// They are not neutral for contrast: an ink checked on a bare ground can
/// lose some through them (measured in the spec). The search highlight is
/// checked through them; the other inks are checked on the bare grounds.
/// The theme's `selection` colour is used for selected text, in the scoped
/// rule.
fn omarchy_define(p: &omarchy::Palette, name: &str) -> Option<String> {
    let hex = |c: omarchy::Rgb| Some(c.hex());
    match name {
        // The generated brand roles.
        "tc_accent" => hex(p.accent),
        "tc_accent_line" => hex(p.accent_line),
        "tc_accent_text" => hex(p.accent_text),
        "tc_on_accent" => hex(p.on_accent),
        "tc_status_on" => hex(p.status_on),
        "tc_status_on_text" => hex(p.status_on_text),
        "tc_status_ask" => hex(p.status_ask),
        "tc_status_ask_text" => hex(p.status_ask_text),
        "tc_status_outside" => hex(p.status_outside),
        "tc_status_outside_text" => hex(p.status_outside_text),
        "tc_on_status" => hex(p.on_status),
        // The glass tiers.
        "tc_glass_pane" => hex(p.bg),
        "tc_glass_card"
        | "tc_glass_card_quiet"
        | "tc_glass_control"
        | "tc_glass_popover"
        | "tc_glass_menu"
        | "tc_glass_node_card" => hex(p.surface),
        "tc_glass_well" => hex(p.inset),
        "tc_glass_control_selected" => hex(p.accent),
        "tc_glass_scene" => hex(p.scene),
        "tc_glass_ink" | "tc_glass_toolbar_glyph" => hex(p.ink),
        "tc_glass_ink_secondary" => hex(p.muted),
        "tc_glass_selected_ink" => hex(p.on_accent),
        "tc_glass_toolbar_glyph_hidden" => hex(p.tertiary),
        "tc_glass_toolbar_expanded" => hex(p.accent_text),
        "tc_glass_control_hover" => hex(p.control_hover),
        "tc_glass_row_hover" => hex(p.row_hover),
        "tc_glass_tab_ember" => hex(p.accent_line),
        // The shell's own grounds and inks.
        "tc_bg" => hex(p.bg),
        "tc_surface" => hex(p.surface),
        "tc_surface2" => hex(p.inset),
        "tc_ink" => hex(p.ink),
        "tc_muted" => hex(p.muted),
        "tc_ink_tertiary" => hex(p.tertiary),
        "tc_line" | "tc_line_divider" => hex(p.line),
        "tc_blue" => hex(p.held),
        "tc_blue_text" | "tc_blue_icon" => hex(p.held_text),
        "tc_gold_highlight" => {
            let omarchy::Rgb(r, g, b) = p.highlight;
            let alpha = omarchy::HIGHLIGHT_WASH;
            Some(format!("rgba({r}, {g}, {b}, {alpha})"))
        }
        "tc_redaction_bg" => hex(p.redaction_bg),
        "tc_redaction_fg" => hex(p.redaction_ink),
        // Labels on status fills; light mode writes them as white.
        "warning_fg_color" | "error_fg_color" | "destructive_fg_color" | "success_fg_color" => {
            hex(p.on_status)
        }
        _ => None,
    }
}

/// Every `@define-color NAME VALUE;` line whose value is a literal (not a
/// reference) and for which `value` answers, rewritten. Everything else is
/// kept as it is.
fn rewrite_defines(css: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(css.len());
    for line in css.split_inclusive('\n') {
        let rewritten = line
            .trim_start()
            .strip_prefix("@define-color ")
            .and_then(|rest| {
                let (name, current) = rest.split_once(char::is_whitespace)?;
                if current.trim_start().starts_with('@') {
                    return None;
                }
                value(name).map(|v| format!("@define-color {name} {v};\n"))
            });
        out.push_str(rewritten.as_deref().unwrap_or(line));
    }
    out
}

/// The generated Insights rules carry literal colours (they mirror the Mac
/// layout); each is replaced by the role it plays there.
fn recolour_insights(css: &str, p: &omarchy::Palette, dark: bool) -> String {
    let roles: &[(&str, omarchy::Rgb)] = if dark {
        &[
            ("#171b2d", p.bg),
            ("#f2f3f8", p.ink),
            ("#14182a", p.surface),
            ("#2a2d3e", p.control_hover),
            ("#7ebcff", p.accent),
            ("#10131f", p.inset),
            ("#3b3d47", p.control_hover),
            ("#071a2e", p.on_accent),
            ("#d8d9e0", p.muted),
        ]
    } else {
        &[
            ("#f2f3f6", p.bg),
            ("#1d1d1f", p.ink),
            ("#fafafb", p.surface),
            ("#dbdde3", p.control_hover),
            ("#0558b6", p.accent),
            ("#fbfbfc", p.surface),
            ("#dbdee4", p.control_hover),
            ("#ffffff", p.on_accent),
            ("#3c3c43", p.muted),
        ]
    };
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let candidate = rest.get(at..at + 7).unwrap_or("");
        let replacement = roles
            .iter()
            .find(|(literal, _)| literal.eq_ignore_ascii_case(candidate))
            .map(|(_, role)| role.hex());
        match replacement {
            Some(hex) => {
                out.push_str(&hex);
                rest = &rest[at + 7..];
            }
            None => {
                out.push('#');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The colours custom (cairo and Pango) drawing uses, as `#rrggbb`.
///
/// GTK offers no supported way to read a `@define-color` back out of a
/// provider, so drawing cannot ask the stylesheet. This is the one place it
/// asks instead: the standard values below are the ones the drawing sites
/// used to carry as literals, and in the Omarchy skin every one comes from
/// the same resolved palette the stylesheet was generated from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inks {
    /// The mark: its frame fill, frame hairline, open and close brackets,
    /// and the single ink of its frameless variant.
    pub mark_surface: String,
    pub mark_line: String,
    pub mark_open: String,
    pub mark_close: String,
    pub mark_template: String,
    /// Fills the map draws dots and rings in.
    pub accent: String,
    pub status_on: String,
    pub status_ask: String,
    pub status_outside: String,
    /// Text-safe inks History strokes its 11px status glyphs in.
    pub accent_text: String,
    pub held_icon: String,
    pub muted: String,
    pub outside_text: String,
    /// Where scrubbing fired, in the transcript.
    pub redaction_bg: String,
    pub redaction_fg: String,
    /// The preview's highlighted excerpt: a colour and Pango `bgalpha`.
    pub highlight: String,
    pub highlight_alpha: String,
    /// The hit's own text colour, set only in the skin; the standard look
    /// keeps the label's ink.
    pub highlight_ink: Option<String>,
    /// The Omarchy palette these inks came from; `None` in the standard
    /// look. The Traces map draws its every role from it in the skin
    /// (`monitor::map::MapColours::resolve`).
    pub skin: Option<omarchy::Palette>,
}

impl Inks {
    pub fn standard(dark: bool) -> Self {
        use trace_commons_mark::Scheme;
        let scheme = if dark { Scheme::Dark } else { Scheme::Light };
        let s = |v: &str| v.to_string();
        if dark {
            use brand_tokens::dark as t;
            Self {
                mark_surface: s(scheme.surface()),
                mark_line: s(scheme.line()),
                mark_open: s(mark_accent(true)),
                mark_close: s(scheme.bracket_close()),
                mark_template: s(scheme.ink()),
                accent: s(t::ACCENT),
                status_on: s(t::STATUS_ON),
                status_ask: s(t::STATUS_ASK),
                status_outside: s(t::STATUS_OUTSIDE),
                accent_text: s(t::ACCENT_TEXT),
                held_icon: s("#9DB6F1"),
                muted: s("#A6AC9F"),
                outside_text: s(t::STATUS_OUTSIDE_TEXT),
                redaction_bg: s("#4A3C18"),
                redaction_fg: s("#F0EBDD"),
                highlight: s("#DCAA43"),
                highlight_alpha: s("32%"),
                highlight_ink: None,
                skin: None,
            }
        } else {
            use brand_tokens::light as t;
            Self {
                mark_surface: s(scheme.surface()),
                mark_line: s(scheme.line()),
                mark_open: s(mark_accent(false)),
                mark_close: s(scheme.bracket_close()),
                mark_template: s(scheme.ink()),
                accent: s(t::ACCENT),
                status_on: s(t::STATUS_ON),
                status_ask: s(t::STATUS_ASK),
                status_outside: s(t::STATUS_OUTSIDE),
                accent_text: s(t::ACCENT_TEXT),
                held_icon: s("#315FBA"),
                muted: s("#5C635B"),
                outside_text: s(t::STATUS_OUTSIDE_TEXT),
                redaction_bg: s("#f3e3c0"),
                redaction_fg: s("#202426"),
                highlight: s("#B9821F"),
                highlight_alpha: s("28%"),
                highlight_ink: None,
                skin: None,
            }
        }
    }

    pub fn omarchy(p: &omarchy::Palette) -> Self {
        Self {
            mark_surface: p.surface.hex(),
            mark_line: p.ink.hex(),
            mark_open: p.accent_line.hex(),
            mark_close: p.ink.hex(),
            mark_template: p.ink.hex(),
            accent: p.accent.hex(),
            status_on: p.status_on.hex(),
            status_ask: p.status_ask.hex(),
            status_outside: p.status_outside.hex(),
            accent_text: p.accent_text.hex(),
            held_icon: p.held_text.hex(),
            muted: p.muted.hex(),
            outside_text: p.status_outside_text.hex(),
            redaction_bg: p.redaction_bg.hex(),
            redaction_fg: p.redaction_ink.hex(),
            highlight: p.highlight.hex(),
            highlight_alpha: omarchy::HIGHLIGHT_ALPHA.to_string(),
            highlight_ink: Some(p.highlight_ink.hex()),
            skin: Some(p.clone()),
        }
    }
}

/// The inks for what is on screen now.
pub fn inks() -> Inks {
    match super::appearance::active_palette() {
        Some(palette) => Inks::omarchy(&palette),
        None => Inks::standard(adw::StyleManager::default().is_dark()),
    }
}

// The gradient-square mark that used to live here is gone. It was a
// transcription of `.brand-mark` in `community/public/styles.css`, and the
// clients no longer wear it: The Turn is the mark now, drawn as real
// geometry in `ui::mark`. The website keeps its own.

/// A micro label over a figure: the site's `.eyebrow` / `th` / `.kpi
/// .label` treatment. Uppercased here rather than in CSS so it does not
/// depend on `text-transform`, which older GTK 4 releases do not implement.
pub fn eyebrow(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .build();
    label.add_css_class("tc-eyebrow");
    label
}

/// A short state token: glyph plus words, in a tone. The site's `.pill`.
///
/// Both halves are mandatory. The glyph is what keeps the state legible
/// without colour; the words are what keep it legible without the glyph.
pub fn tag(text: &str, tone: Tone) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::XXS);
    row.add_css_class("tc-tag");
    row.add_css_class(tone.css());
    row.set_valign(gtk::Align::Center);
    let glyph = gtk::Label::new(Some(tone.glyph()));
    let label = gtk::Label::new(Some(text));
    row.append(&glyph);
    row.append(&label);
    // One accessible object, read as "check, in the commons" rather than as
    // two unrelated labels.
    row.update_property(&[gtk::accessible::Property::Label(text)]);
    row
}

/// One field of a manifest strip: a heavy micro label over a monospaced
/// figure.
pub fn manifest_field(label: &str, value: &str, tone: Tone) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&eyebrow(label));
    let figure = gtk::Label::builder().label(value).xalign(0.0).build();
    figure.add_css_class("tc-ledger");
    if tone != Tone::Neutral {
        figure.add_css_class(tone.css());
    }
    column.append(&figure);
    column
}

/// The manifest strip itself: the same fields, in the same order, on every
/// card in the application.
///
/// This is the signature element. A card whose payload is unusual, or whose
/// scrubbing matched nothing, breaks the rhythm of a column of these
/// without anyone having to read a sentence to notice.
pub fn manifest(fields: &[(&str, String, Tone)]) -> gtk::Box {
    let strip = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(space::L)
        .homogeneous(true)
        .build();
    strip.add_css_class("tc-manifest");
    for (label, value, tone) in fields {
        strip.append(&manifest_field(label, value, *tone));
    }
    strip
}

/// A section heading: the eyebrow plus a hairline running to the end of the
/// column. The rule is structural, not decorative -- it is what says the
/// group below is a different kind of thing from the group above.
pub fn section(title: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    row.set_valign(gtk::Align::Center);
    let label = eyebrow(title);
    label.add_css_class("tc-clear");
    row.append(&label);
    let rule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    rule.add_css_class("tc-rule");
    rule.set_hexpand(true);
    rule.set_valign(gtk::Align::Center);
    rule.set_height_request(1);
    row.append(&rule);
    row
}

/// Body copy: the wrapped, left-aligned paragraph face used throughout the
/// declaration cards. The site's default running text.
pub fn body(text: impl AsRef<str>) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.as_ref())
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-body");
    label
}

/// Build a [`body`] label and append it to `parent` in one step.
///
/// An empty text appends nothing: the owner cut some core lines to "" (Settings
/// copy pass 2, 2026-11-04) and an empty label would still take its spacing.
pub fn append_body(parent: &gtk::Box, text: impl AsRef<str>) {
    if text.as_ref().is_empty() {
        return;
    }
    parent.append(&body(text));
}

/// Caveat copy: the same wrapped paragraph face, toned down for a footnote
/// or condition rather than the card's primary sentence.
pub fn caveat(text: impl AsRef<str>) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.as_ref())
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-caveat");
    label
}

/// Build a [`caveat`] label and append it to `parent` in one step.
///
/// An empty text appends nothing, as with [`append_body`].
pub fn append_caveat(parent: &gtk::Box, text: impl AsRef<str>) {
    if text.as_ref().is_empty() {
        return;
    }
    parent.append(&caveat(text));
}

/// Meta copy: the same wrapped paragraph face, for secondary or
/// small-print facts.
pub fn meta(text: impl AsRef<str>) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text.as_ref())
        .xalign(0.0)
        .wrap(true)
        .build();
    label.add_css_class("tc-meta");
    label
}

/// Build a [`meta`] label and append it to `parent` in one step.
pub fn append_meta(parent: &gtk::Box, text: impl AsRef<str>) {
    parent.append(&meta(text));
}

/// A card face: surface, 8px radius, one hairline, no shadow.
///
/// The site's `0 18px 48px` card shadow is a web idiom; inside an
/// application window it reads as a floating dialog. A hairline does the
/// same separating work natively.
pub fn card(orientation: gtk::Orientation, spacing: i32) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(orientation)
        .spacing(spacing)
        .build();
    card.add_css_class("tc-card");
    card
}

/// A row of controls or figures that wraps onto further lines when its
/// column is narrower than the row, so it never makes the column wider
/// than the Monitor window's 440 main pane. Fill it with [`wrap_append`].
///
/// Its own container, not a `GtkFlowBox`: on GTK 4.14 a flow box of
/// buttons asked for less height than the lines it drew, clipping its last
/// buttons ([`super::wrap_row`]).
pub fn wrap_row(spacing: i32) -> super::wrap_row::WrapRow {
    super::wrap_row::WrapRow::new(spacing)
}

/// Appends to a [`wrap_row`].
pub fn wrap_append(row: &super::wrap_row::WrapRow, widget: &impl IsA<gtk::Widget>) {
    row.append(widget);
}

/// Appends to a grid of equal tiles (a homogeneous `GtkFlowBox`). The flow
/// box's own child wrapper takes no focus, so the keyboard reaches the tile
/// itself in one step.
///
/// A grid with nothing in it to focus (the stat cards: figures and words)
/// is taken off the Tab chain. A `GtkFlowBox` answers Tab as handled even
/// when the tile it offered focus to refused it, so focus stayed where it
/// was: Tab never got past it to the records below (KDE box, 2026-11-18).
pub fn grid_append(grid: &gtk::FlowBox, widget: &impl IsA<gtk::Widget>) {
    grid.insert(widget, -1);
    if let Some(child) = widget.parent() {
        child.set_focusable(false);
    }
    grid.set_can_focus(takes_focus(grid.upcast_ref()));
}

/// Whether anything inside `root` can hold keyboard focus.
fn takes_focus(root: &gtk::Widget) -> bool {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.can_focus() && (widget.is_focusable() || takes_focus(&widget)) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

#[cfg(test)]
mod tests {
    //! Contrast, measured from the values this file actually loads.
    //!
    //! The generated accent and status roles come from the design tokens;
    //! the grounds they sit on are this shell's own, so only this shell can
    //! say whether a pairing holds. Every ratio is WCAG 2.1 relative
    //! luminance: 4.5:1 for text, 3:1 for a fill, rule or border.

    use std::collections::HashMap;

    use super::{brand_tokens, stylesheet, stylesheet_for};
    use crate::ui::glass_backdrop;

    /// A grid of figures is no stop on the Tab chain, and Tab goes on
    /// past it; a grid of buttons still takes focus on its tiles.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_tab_passes_a_grid_of_figures() {
        use gtk::prelude::*;
        adw::init().expect("GTK display unavailable");
        let mount = |tile: gtk::Widget| {
            let grid = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .homogeneous(true)
                .build();
            super::grid_append(&grid, &tile);
            let after = gtk::Button::with_label("after");
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.append(&grid);
            column.append(&after);
            let window = gtk::Window::builder().child(&column).build();
            window.present();
            let context = gtk::glib::MainContext::default();
            let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < until {
                context.iteration(false);
            }
            // From no focus, as the first Tab into a window that opened
            // with none.
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            window.child_focus(gtk::DirectionType::TabForward);
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            window.destroy();
            (focus, after.upcast::<gtk::Widget>())
        };
        let figure = gtk::Box::new(gtk::Orientation::Vertical, 0);
        figure.append(&gtk::Label::new(Some("3")));
        let (focus, after) = mount(figure.upcast());
        assert_eq!(focus, Some(after), "Tab stopped on a grid of figures");
        let button = gtk::Button::with_label("tile");
        let (focus, _) = mount(button.clone().upcast());
        assert_eq!(
            focus,
            Some(button.upcast()),
            "a grid of buttons is passed over"
        );
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn both_schemes_parse_with_insights_styles() {
        assert_eq!(std::env::consts::OS, "linux");
        adw::init().expect("GTK display unavailable");
        for (dark, translucent) in [(false, false), (true, false), (false, true), (true, true)] {
            let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let captured = errors.clone();
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(move |_, _, error| {
                captured.borrow_mut().push(error.to_string());
            });
            provider.load_from_data(&stylesheet_for(dark, translucent));
            assert!(
                errors.borrow().is_empty(),
                "dark={dark} translucent={translucent}: {:?}",
                errors.borrow()
            );
        }
    }

    /// The translucent sheet works by redefining colours the painted rules
    /// already name, which holds only if GTK resolves a `@define-color` to
    /// its LAST definition in a provider. Pinned against GTK itself.
    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn gtk_resolves_a_redefined_colour_to_its_last_definition() {
        assert_eq!(std::env::consts::OS, "linux");
        adw::init().expect("GTK display unavailable");
        use gtk::prelude::*;
        let display = gtk::gdk::Display::default().expect("display");
        let provider = gtk::CssProvider::new();
        provider.load_from_data(
            "@define-color tc_probe #ff0000;\n\
             label.tc-probe { color: @tc_probe; }\n\
             @define-color tc_probe #0000ff;",
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        let label = gtk::Label::new(None);
        label.add_css_class("tc-probe");
        let window = gtk::Window::new();
        window.set_child(Some(&label));
        #[allow(deprecated)]
        let colour = label.style_context().color();
        gtk::style_context_remove_provider_for_display(&display, &provider);
        window.destroy();
        assert_eq!(
            (colour.red(), colour.green(), colour.blue()),
            (0.0, 0.0, 1.0),
            "{colour:?}"
        );
    }

    /// The translucent sheets come after everything they redefine, and the
    /// painted stylesheet carries none of them.
    #[test]
    fn the_translucent_sheet_is_last_and_only_when_asked() {
        for dark in [false, true] {
            let (glass, insights) = if dark {
                (
                    brand_tokens::GLASS_DARK_BLUR_CSS,
                    brand_tokens::INSIGHTS_DARK_BLUR_CSS,
                )
            } else {
                (
                    brand_tokens::GLASS_LIGHT_BLUR_CSS,
                    brand_tokens::INSIGHTS_LIGHT_BLUR_CSS,
                )
            };
            let painted = stylesheet(dark);
            assert!(!painted.contains("@define-color tc_glass_scene transparent;"));
            let translucent = stylesheet_for(dark, true);
            assert!(translucent.starts_with(&painted), "dark={dark}");
            assert!(
                translucent.ends_with(&format!("\n{glass}\n{insights}")),
                "dark={dark}"
            );
        }
    }

    const GROUNDS: [&str; 3] = ["tc_bg", "tc_surface", "tc_surface2"];

    /// Every `@define-color` in the stylesheet, references resolved, opaque
    /// hex values only.
    fn palette(dark: bool) -> HashMap<String, String> {
        palette_of(&stylesheet(dark))
    }

    fn palette_of(css: &str) -> HashMap<String, String> {
        let mut raw = HashMap::new();
        for line in css.lines() {
            let Some(rest) = line.trim().strip_prefix("@define-color ") else {
                continue;
            };
            let (name, value) = rest.split_once(char::is_whitespace).unwrap();
            let value = value.split(';').next().unwrap().trim();
            raw.insert(name.to_string(), value.to_string());
        }
        let mut resolved = HashMap::new();
        for name in raw.keys() {
            let mut value = raw[name].as_str();
            while let Some(reference) = value.strip_prefix('@') {
                value = raw
                    .get(reference)
                    .unwrap_or_else(|| panic!("{name} refers to undefined @{reference}"));
            }
            if value.len() == 7 && value.starts_with('#') {
                resolved.insert(name.clone(), value.to_ascii_uppercase());
            }
        }
        resolved
    }

    fn luminance(hex: &str) -> f64 {
        let channel = |i: usize| {
            let v = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn check(dark: bool, pairs: &[(&str, &str)], floor: f64) -> Vec<String> {
        check_css(&format!("dark={dark}"), &stylesheet(dark), pairs, floor)
    }

    fn check_css(label: &str, css: &str, pairs: &[(&str, &str)], floor: f64) -> Vec<String> {
        let colours = palette_of(css);
        let get = |name: &str| {
            colours
                .get(name)
                .unwrap_or_else(|| panic!("{name} is not an opaque colour"))
                .clone()
        };
        pairs
            .iter()
            .filter_map(|(fg, bg)| {
                let ratio = contrast(&get(fg), &get(bg));
                (ratio < floor).then(|| format!("{label}: {fg} on {bg} is {ratio:.2}:1"))
            })
            .collect()
    }

    fn on_every_ground<'a>(inks: &[&'a str]) -> Vec<(&'a str, &'static str)> {
        inks.iter()
            .flat_map(|ink| GROUNDS.iter().map(move |ground| (*ink, *ground)))
            .collect()
    }

    /// A colour the translucent sheet defines, as r, g, b (0-255) and
    /// alpha.
    fn blur_colour(dark: bool, name: &str) -> (f64, f64, f64, f64) {
        let sheet = if dark {
            brand_tokens::GLASS_DARK_BLUR_CSS
        } else {
            brand_tokens::GLASS_LIGHT_BLUR_CSS
        };
        let value = sheet
            .lines()
            .find_map(|line| line.strip_prefix(&format!("@define-color {name} ")))
            .unwrap_or_else(|| panic!("the translucent sheet defines no {name}"));
        let inner = value
            .trim_end_matches(';')
            .trim()
            .strip_prefix("rgba(")
            .and_then(|v| v.strip_suffix(')'))
            .unwrap_or_else(|| panic!("{name} is not rgba: {value}"));
        let parts: Vec<f64> = inner
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        (parts[0], parts[1], parts[2], parts[3])
    }

    fn flatten(colour: (f64, f64, f64, f64), ground: &str) -> String {
        let (r, g, b, a) = colour;
        let base: Vec<f64> = (0..3)
            .map(|i| f64::from(u8::from_str_radix(&ground[1 + 2 * i..3 + 2 * i], 16).unwrap()))
            .collect();
        let mix = |c: f64, i: usize| (c * a + base[i] * (1.0 - a)).round() as u8;
        format!("#{:02x}{:02x}{:02x}", mix(r, 0), mix(g, 1), mix(b, 2))
    }

    /// Every text colour the panes' screens use, for the translucent check:
    /// the glass inks, tertiary included (column titles and sub-lines), the
    /// shell's own and every status and accent text. `tc_glass_status_off`
    /// is left out: it colours the chevron glyph and a turned-off Submit,
    /// which carry no text a person must read. The selected row's ink sits
    /// on the selection's opaque fill, not on the pane.
    const PANE_INKS: [&str; 15] = [
        "tc_glass_ink",
        "tc_glass_ink_secondary",
        "tc_glass_ink_tertiary",
        "tc_glass_status_on_text",
        "tc_glass_status_ask_text",
        "tc_ink",
        "tc_muted",
        "accent_color",
        "tc_green_text",
        "tc_gold_text",
        "tc_coral_text",
        "tc_blue_text",
        "success_color",
        "warning_color",
        "error_color",
    ];

    /// The desktop as KWin hands it to the pane: `desktop` itself, or
    /// through the frost when the contrast is granted.
    fn behind_pane(desktop: [f64; 3], frost: Option<glass_backdrop::Frost>) -> String {
        let shown = frost.map_or(desktop, |frost| glass_backdrop::frosted(desktop, frost));
        let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            byte(shown[0]),
            byte(shown[1]),
            byte(shown[2])
        )
    }

    /// Every failure of [`PANE_INKS`] on the translucent panes over a black
    /// and a white desktop, in and out of focus, on the pane and on the
    /// tiers that hold text.
    fn translucent_failures(dark: bool, frost: Option<glass_backdrop::Frost>) -> Vec<String> {
        let mut failures = Vec::new();
        let colours = palette(dark);
        for desktop in [[0.0; 3], [1.0; 3]] {
            let desktop = behind_pane(desktop, frost);
            for pane in ["tc_glass_pane", "tc_glass_pane_inactive"] {
                let ground = flatten(blur_colour(dark, pane), &desktop);
                let mut grounds = vec![(pane.to_string(), ground.clone())];
                for tier in ["tc_glass_card", "tc_glass_card_quiet", "tc_glass_control"] {
                    grounds.push((
                        format!("{tier} on {pane}"),
                        flatten(blur_colour(dark, tier), &ground),
                    ));
                }
                for (where_, under) in &grounds {
                    for ink in PANE_INKS {
                        let ratio = contrast(&colours[ink], under);
                        if ratio < 4.5 {
                            failures.push(format!(
                                "dark={dark}: {ink} on {where_} over {desktop} is {ratio:.2}:1"
                            ));
                        }
                    }
                }
            }
        }
        failures
    }

    /// Text in a translucent pane sits on whatever KWin's blur averages the
    /// desktop to, through the frost where KWin's contrast is granted. A
    /// scheme may go translucent on its own (`glass_backdrop::readable`)
    /// only if every text colour the panes' screens use reads at 4.5:1
    /// there too, over a black and a white desktop, in and out of focus,
    /// on the pane and on the tiers that hold text. Two-sided: `readable`
    /// must agree with the measurement, so the gate can neither open on a
    /// failure nor stay shut after a fix.
    #[test]
    fn every_text_colour_reads_on_the_translucent_panes() {
        for dark in [false, true] {
            for granted in [false, true] {
                let frost = granted.then(|| glass_backdrop::frost(dark));
                let failures = translucent_failures(dark, frost);
                assert_eq!(
                    failures.is_empty(),
                    glass_backdrop::readable(dark, granted),
                    "dark={dark} contrast={granted}: {failures:#?}"
                );
            }
        }
    }

    /// The frost's contrast is the largest, in steps of 0.05, at which the
    /// text reads: the most desktop that can show through. In light no step
    /// reads (a mid-grey desktop at contrast 0 still leaves `tc_muted`
    /// under 4.5:1), so light asks for none and stays painted.
    #[test]
    fn the_frost_shows_as_much_desktop_as_the_text_allows() {
        for dark in [false, true] {
            let chosen = glass_backdrop::frost(dark);
            let best = (0..=20)
                .rev()
                .map(|step| f64::from(step) * 0.05)
                .find(|&contrast| {
                    translucent_failures(dark, Some(glass_backdrop::Frost { contrast, ..chosen }))
                        .is_empty()
                });
            match best {
                Some(best) => assert!((chosen.contrast - best).abs() < 1e-9, "dark={dark}: {best}"),
                None => assert_eq!(chosen.contrast, 1.0, "dark={dark}"),
            }
            assert_eq!(best.is_some(), dark, "dark={dark}: {best:?}");
        }
    }

    #[test]
    fn every_status_and_accent_text_clears_4_5_on_every_ground() {
        let text = on_every_ground(&[
            "accent_color",
            "tc_green_text",
            "tc_gold_text",
            "tc_coral_text",
            "success_color",
            "warning_color",
            "error_color",
        ]);
        for dark in [false, true] {
            assert_eq!(check(dark, &text, 4.5), Vec::<String>::new());
        }
    }

    #[test]
    fn every_accent_and_status_rule_clears_3_on_every_ground() {
        let rules = on_every_ground(&["tc_green", "tc_gold", "tc_coral"]);
        for dark in [false, true] {
            assert_eq!(check(dark, &rules, 3.0), Vec::<String>::new());
        }
    }

    /// The fills libadwaita draws controls in (switches, checks, the
    /// suggested action) sit on the window ground and on cards.
    #[test]
    fn the_accent_fill_clears_3_on_the_window_and_a_card() {
        let fills = [
            ("accent_bg_color", "tc_bg"),
            ("accent_bg_color", "tc_surface"),
        ];
        for dark in [false, true] {
            assert_eq!(check(dark, &fills, 3.0), Vec::<String>::new());
        }
    }

    #[test]
    fn every_label_on_a_fill_clears_4_5() {
        let labels = [
            ("accent_fg_color", "accent_bg_color"),
            ("tc_primary_label", "tc_primary_fill"),
            ("success_fg_color", "success_bg_color"),
            ("warning_fg_color", "warning_bg_color"),
            ("error_fg_color", "error_bg_color"),
            ("destructive_fg_color", "destructive_bg_color"),
        ];
        for dark in [false, true] {
            assert_eq!(check(dark, &labels, 4.5), Vec::<String>::new());
        }
    }

    /// The colour a rule for exactly `selector` in this shell's stylesheet
    /// gives its text, if one does.
    fn rule_colour(selector: &str) -> Option<String> {
        let css = include_str!("style.css");
        css.split('}').find_map(|rule| {
            let (selectors, body) = rule.split_once('{')?;
            let selectors: Vec<&str> = selectors
                .rsplit("*/")
                .next()
                .unwrap_or(selectors)
                .split(',')
                .map(str::trim)
                .collect();
            if !selectors.contains(&selector) {
                return None;
            }
            body.split(';').find_map(|line| {
                let value = line.trim().strip_prefix("color:")?.trim();
                Some(value.trim_start_matches('@').to_string())
            })
        })
    }

    /// The Traces tab's count badge with the attention glyph beside it
    /// (`App::set_queue_count`) is text on the accent fill. `.tc-attention`
    /// alone is gold text for a plain ground, and it comes after
    /// `.tc-count-badge`, so without a rule of its own the badge read dark
    /// brown on purple in light mode. The Mac's badge has one colour pair
    /// whatever it counts (`GlassBadgeValue`).
    #[test]
    fn the_attention_badge_is_readable_on_its_fill() {
        let colour = rule_colour(".tc-count-badge.tc-attention")
            .or_else(|| rule_colour(".tc-attention"))
            .expect("the attention badge has a colour");
        assert_eq!(
            rule_colour(".tc-count-badge").as_deref(),
            Some("tc_primary_label")
        );
        for dark in [false, true] {
            assert_eq!(
                check(dark, &[(colour.as_str(), "tc_primary_fill")], 4.5),
                Vec::<String>::new()
            );
        }
    }

    /// The mark's open bracket on the mark's own frame fill.
    #[test]
    fn the_mark_accent_clears_3_on_the_mark_frame() {
        use trace_commons_mark::Scheme;
        for (dark, scheme) in [(false, Scheme::Light), (true, Scheme::Dark)] {
            let ratio = contrast(super::mark_accent(dark), scheme.surface());
            assert!(ratio >= 3.0, "dark={dark}: {ratio:.2}:1");
        }
    }

    /// Purple everywhere: the accent is the brand purple and its dark twin,
    /// and the green and mint this file used to pin are gone.
    #[test]
    fn the_accent_is_the_brand_purple() {
        assert_eq!(palette(false)["accent_bg_color"], "#6D14F3");
        assert_eq!(palette(true)["accent_bg_color"], "#8A3DFF");
        assert_eq!(brand_tokens::light::ACCENT, "#6D14F3");
        assert_eq!(brand_tokens::dark::ACCENT, "#8A3DFF");
        for dark in [false, true] {
            let css = stylesheet(dark).to_ascii_uppercase();
            for retired in [
                "#178F70", "#137C61", "#0F7256", "#3FBE9A", "#5CD3AF", "#00D4AA",
            ] {
                assert!(
                    !css.contains(retired),
                    "dark={dark}: {retired} is still loaded"
                );
            }
        }
    }

    // --- The Omarchy skin -------------------------------------------------

    use super::omarchy::{self, fixtures::FIXTURES};

    fn omarchy_sheets() -> Vec<(String, omarchy::Palette, String)> {
        FIXTURES
            .iter()
            .map(|(name, text)| {
                let palette = omarchy::parse(text, false).expect(name);
                let css = super::omarchy_stylesheet(&palette);
                (name.to_string(), palette, css)
            })
            .collect()
    }

    fn define_names(css: &str) -> std::collections::BTreeSet<String> {
        css.lines()
            .filter_map(|line| line.trim().strip_prefix("@define-color "))
            .map(|rest| rest.split_whitespace().next().unwrap().to_string())
            .collect()
    }

    /// Off is off: with Standard chosen, what loads is today's sheet, byte
    /// for byte.
    #[test]
    fn the_standard_sheet_is_unchanged_without_a_palette() {
        for dark in [false, true] {
            assert_eq!(super::sheet_for(None, dark), stylesheet(dark));
        }
        let (_, palette, css) = omarchy_sheets().remove(0);
        for dark in [false, true] {
            assert_eq!(super::sheet_for(Some(&palette), dark), css);
        }
    }

    /// The skin's sheet loads whether or not the translucent one is asked
    /// for: the blur sheets would redefine the glass colours the palette
    /// set. Without a palette, the translucent sheet loads as asked.
    #[test]
    fn a_skin_stays_painted_and_the_standard_sheet_follows_the_blur() {
        let (_, palette, css) = omarchy_sheets().remove(0);
        for dark in [false, true] {
            for translucent in [false, true] {
                assert_eq!(
                    super::sheet_for_window(Some(&palette), dark, translucent),
                    css
                );
                assert_eq!(
                    super::sheet_for_window(None, dark, translucent),
                    super::stylesheet_for(dark, translucent)
                );
            }
        }
        assert_ne!(
            super::sheet_for_window(None, true, true),
            super::sheet_for_window(None, true, false)
        );
    }

    /// `@define-color` cannot be scoped, so the skin replaces values and
    /// never structure: the same names, every one of them.
    #[test]
    fn the_omarchy_sheet_defines_exactly_the_standard_names() {
        for (name, palette, css) in omarchy_sheets() {
            let dark = palette.mode == omarchy::Mode::Dark;
            assert_eq!(
                define_names(&css),
                define_names(&stylesheet(dark)),
                "{name}"
            );
            let glass = define_names(&css)
                .into_iter()
                .filter(|n| n.starts_with("tc_glass_"))
                .count();
            assert_eq!(glass, 19, "{name}");
        }
    }

    /// Every literal colour in the skin's sheet is one of the palette's own,
    /// so no brand colour is left behind on an Omarchy screen. Comments are
    /// stripped first: style.css mentions hex values in prose.
    #[test]
    fn no_standard_colour_survives_in_the_omarchy_sheet() {
        fn strip_comments(css: &str) -> String {
            let mut out = String::new();
            let mut rest = css;
            while let Some(start) = rest.find("/*") {
                out.push_str(&rest[..start]);
                rest = rest[start..]
                    .split_once("*/")
                    .map_or("", |(_, after)| after);
            }
            out.push_str(rest);
            out
        }
        for (name, p, css) in omarchy_sheets() {
            let allowed: std::collections::BTreeSet<String> = [
                p.bg,
                p.surface,
                p.inset,
                p.scene,
                p.control_hover,
                p.row_hover,
                p.ink,
                p.strong,
                p.muted,
                p.tertiary,
                p.line,
                p.accent,
                p.on_accent,
                p.accent_line,
                p.accent_text,
                p.status_on,
                p.status_on_text,
                p.status_ask,
                p.status_ask_text,
                p.status_outside,
                p.status_outside_text,
                p.held,
                p.held_text,
                p.on_status,
                p.selection,
                p.selection_ink,
                p.redaction_bg,
                p.redaction_ink,
                p.highlight,
            ]
            .iter()
            .map(|c| c.hex())
            .collect();
            let css = strip_comments(&css);
            let mut stray = Vec::new();
            for (i, _) in css.match_indices('#') {
                let candidate = css.get(i..i + 7).unwrap_or("");
                if omarchy::Rgb::parse(candidate).is_some()
                    && !allowed.contains(&candidate.to_ascii_uppercase())
                {
                    stray.push(candidate.to_string());
                }
            }
            assert!(stray.is_empty(), "{name}: {stray:?}");
        }
    }

    /// The same matrix the standard sheet is held to, over every vendored
    /// palette, read back out of the generated CSS -- so the sheet and the
    /// adapter cannot drift apart.
    #[test]
    fn the_omarchy_sheet_meets_the_standard_contrast_matrix() {
        let text = on_every_ground(&[
            "accent_color",
            "tc_green_text",
            "tc_gold_text",
            "tc_coral_text",
            "tc_blue_text",
            "success_color",
            "warning_color",
            "error_color",
            "tc_ink",
            "tc_muted",
            "tc_ink_tertiary",
        ]);
        let rules = on_every_ground(&["tc_green", "tc_gold", "tc_coral", "tc_blue"]);
        let fills = [
            ("accent_bg_color", "tc_bg"),
            ("accent_bg_color", "tc_surface"),
        ];
        let labels = [
            ("accent_fg_color", "accent_bg_color"),
            ("tc_primary_label", "tc_primary_fill"),
            ("success_fg_color", "success_bg_color"),
            ("warning_fg_color", "warning_bg_color"),
            ("error_fg_color", "error_bg_color"),
            ("destructive_fg_color", "destructive_bg_color"),
            ("tc_redaction_fg", "tc_redaction_bg"),
        ];
        let glass_grounds = [
            "tc_glass_pane",
            "tc_glass_card",
            "tc_glass_card_quiet",
            "tc_glass_well",
            "tc_glass_control",
            "tc_glass_popover",
            "tc_glass_menu",
            "tc_glass_node_card",
            "tc_glass_scene",
            "tc_glass_control_hover",
            "tc_glass_row_hover",
        ];
        let glass_text: Vec<(&str, &str)> = ["tc_glass_ink", "tc_glass_ink_secondary"]
            .iter()
            .flat_map(|ink| glass_grounds.iter().map(move |g| (*ink, *g)))
            .collect();
        let glass_label = [("tc_glass_selected_ink", "tc_glass_control_selected")];
        for (name, _, css) in omarchy_sheets() {
            let mut failures = check_css(&name, &css, &text, 4.5);
            failures.extend(check_css(&name, &css, &rules, 3.0));
            failures.extend(check_css(&name, &css, &fills, 3.0));
            failures.extend(check_css(&name, &css, &labels, 4.5));
            failures.extend(check_css(&name, &css, &glass_text, 4.5));
            failures.extend(check_css(&name, &css, &glass_label, 4.5));
            assert_eq!(failures, Vec::<String>::new());
        }
    }

    /// Rules only the skin has are scoped under the class the shell puts on
    /// its windows while the skin is on.
    #[test]
    fn skin_only_rules_are_scoped_and_absent_from_the_standard_sheet() {
        for dark in [false, true] {
            assert!(!stylesheet(dark).contains(".tc-omarchy"));
        }
        for (name, p, css) in omarchy_sheets() {
            let scoped = css
                .split('}')
                .find(|rule| rule.contains(".tc-omarchy") && rule.contains("selection"))
                .unwrap_or_else(|| panic!("{name}: no scoped selection rule"));
            assert!(scoped.contains(&p.selection.hex()), "{name}");
            assert!(scoped.contains(&p.selection_ink.hex()), "{name}");
        }
    }

    /// libadwaita draws `.dim-label` as the text colour at 55% opacity. On a
    /// theme's own foreground that fell to 2.5:1 on Rose Pine and Catppuccin
    /// Latte (measured on the Omarchy box: the map legend). Under the skin a
    /// dim label is the palette's tertiary ink, opaque, which clears 4.5:1 on
    /// every ground.
    #[test]
    fn skin_dim_labels_are_the_opaque_tertiary_ink() {
        for (name, p, css) in omarchy_sheets() {
            let rule = css
                .split('}')
                .find(|rule| rule.contains(".tc-omarchy .dim-label"))
                .unwrap_or_else(|| panic!("{name}: no scoped dim-label rule"));
            assert!(rule.contains(&p.tertiary.hex()), "{name}: {rule}");
            assert!(rule.contains("opacity: 1"), "{name}: {rule}");
        }
    }

    #[test]
    #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
    fn every_omarchy_sheet_parses() {
        assert_eq!(std::env::consts::OS, "linux");
        adw::init().expect("GTK display unavailable");
        for (name, _, css) in omarchy_sheets() {
            let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let captured = errors.clone();
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(move |_, _, error| {
                captured.borrow_mut().push(error.to_string());
            });
            provider.load_from_data(&css);
            assert!(errors.borrow().is_empty(), "{name}: {:?}", errors.borrow());
        }
    }

    /// The standard inks are the literals the drawing sites carried before
    /// they asked here, so the standard look draws exactly as it did.
    #[test]
    fn the_standard_inks_are_the_old_literals() {
        use trace_commons_mark::Scheme;
        for (dark, scheme) in [(false, Scheme::Light), (true, Scheme::Dark)] {
            let inks = super::Inks::standard(dark);
            let tokens = if dark {
                (brand_tokens::dark::ACCENT, brand_tokens::dark::ACCENT_TEXT)
            } else {
                (
                    brand_tokens::light::ACCENT,
                    brand_tokens::light::ACCENT_TEXT,
                )
            };
            assert_eq!(inks.mark_surface, scheme.surface());
            assert_eq!(inks.mark_line, scheme.line());
            assert_eq!(inks.mark_open, super::mark_accent(dark));
            assert_eq!(inks.mark_close, scheme.bracket_close());
            assert_eq!(inks.mark_template, scheme.ink());
            assert_eq!(inks.accent, tokens.0);
            assert_eq!(inks.accent_text, tokens.1);
        }
        let light = super::Inks::standard(false);
        assert_eq!(light.held_icon, "#315FBA");
        assert_eq!(light.muted, "#5C635B");
        assert_eq!(light.outside_text, brand_tokens::light::STATUS_OUTSIDE_TEXT);
        assert_eq!(light.status_on, brand_tokens::light::STATUS_ON);
        assert_eq!(light.status_ask, brand_tokens::light::STATUS_ASK);
        assert_eq!(light.status_outside, brand_tokens::light::STATUS_OUTSIDE);
        assert_eq!(
            (light.redaction_bg.as_str(), light.redaction_fg.as_str()),
            ("#f3e3c0", "#202426")
        );
        assert_eq!(
            (light.highlight.as_str(), light.highlight_alpha.as_str()),
            ("#B9821F", "28%")
        );
        let dark = super::Inks::standard(true);
        assert_eq!(dark.held_icon, "#9DB6F1");
        assert_eq!(dark.muted, "#A6AC9F");
        assert_eq!(
            (dark.redaction_bg.as_str(), dark.redaction_fg.as_str()),
            ("#4A3C18", "#F0EBDD")
        );
        assert_eq!(
            (dark.highlight.as_str(), dark.highlight_alpha.as_str()),
            ("#DCAA43", "32%")
        );
        // The standard look keeps the label's own ink on the wash.
        assert_eq!((light.highlight_ink, dark.highlight_ink), (None, None));
    }

    /// In the skin every drawing ink is the palette's.
    #[test]
    fn the_omarchy_inks_come_from_the_palette() {
        for (name, p, _) in omarchy_sheets() {
            let inks = super::Inks::omarchy(&p);
            assert_eq!(inks.mark_surface, p.surface.hex(), "{name}");
            assert_eq!(inks.mark_line, p.ink.hex(), "{name}");
            assert_eq!(inks.mark_open, p.accent_line.hex(), "{name}");
            assert_eq!(inks.mark_close, p.ink.hex(), "{name}");
            assert_eq!(inks.mark_template, p.ink.hex(), "{name}");
            assert_eq!(inks.accent, p.accent.hex(), "{name}");
            assert_eq!(inks.status_on, p.status_on.hex(), "{name}");
            assert_eq!(inks.status_ask, p.status_ask.hex(), "{name}");
            assert_eq!(inks.status_outside, p.status_outside.hex(), "{name}");
            assert_eq!(inks.accent_text, p.accent_text.hex(), "{name}");
            assert_eq!(inks.held_icon, p.held_text.hex(), "{name}");
            assert_eq!(inks.muted, p.muted.hex(), "{name}");
            assert_eq!(inks.outside_text, p.status_outside_text.hex(), "{name}");
            assert_eq!(inks.redaction_bg, p.redaction_bg.hex(), "{name}");
            assert_eq!(inks.redaction_fg, p.redaction_ink.hex(), "{name}");
            assert_eq!(inks.highlight, p.highlight.hex(), "{name}");
            assert_eq!(inks.highlight_alpha, omarchy::HIGHLIGHT_ALPHA, "{name}");
            assert_eq!(inks.highlight_ink, Some(p.highlight_ink.hex()), "{name}");
            // The mark's open bracket on its own frame, as the standard
            // mark is held to.
            let ratio = omarchy::contrast(p.accent_line, p.surface);
            assert!(ratio >= 3.0, "{name}: {ratio:.2}:1");
        }
    }

    /// The overlays the adapter checks the highlight through are the ones
    /// the token blocks define, and the skin keeps them.
    #[test]
    fn the_adapters_overlays_are_the_sheets() {
        let rgba = |(tint, alpha): (omarchy::Rgb, f64)| {
            let omarchy::Rgb(r, g, b) = tint;
            format!("rgba({r}, {g}, {b}, {alpha})")
        };
        for (mode, block) in [
            (omarchy::Mode::Light, super::LIGHT_TOKENS),
            (omarchy::Mode::Dark, super::DARK_TOKENS),
        ] {
            let [scrim, selected] = omarchy::overlays(mode);
            for (name, value) in [("tc_scrim", rgba(scrim)), ("tc_selected", rgba(selected))] {
                let line = block
                    .lines()
                    .find(|l| l.starts_with(&format!("@define-color {name} ")))
                    .unwrap_or_else(|| panic!("{name}"));
                assert!(line.contains(&value), "{mode:?}: {line} vs {value}");
            }
        }
    }
}
