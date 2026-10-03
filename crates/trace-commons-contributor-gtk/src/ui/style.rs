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
///   4.98:1  coral text  #FF6B6B on surface-2  #2A2E27
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

    load(&provider, manager.is_dark());
    let p = provider.clone();
    manager.connect_dark_notify(move |manager| load(&p, manager.is_dark()));
}

fn load(provider: &gtk::CssProvider, dark: bool) {
    provider.load_from_data(&stylesheet(dark));
}

/// The whole stylesheet for one scheme: the generated accent and status
/// roles first, since the palette refers to them by name, then the palette,
/// then the rules.
fn stylesheet(dark: bool) -> String {
    let (brand, tokens) = if dark {
        (brand_tokens::DARK_CSS, DARK_TOKENS)
    } else {
        (brand_tokens::LIGHT_CSS, LIGHT_TOKENS)
    };
    format!("{brand}\n{tokens}\n{}", include_str!("style.css"))
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
pub fn append_body(parent: &gtk::Box, text: impl AsRef<str>) {
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
pub fn append_caveat(parent: &gtk::Box, text: impl AsRef<str>) {
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

#[cfg(test)]
mod tests {
    //! Contrast, measured from the values this file actually loads.
    //!
    //! The generated accent and status roles come from the design tokens;
    //! the grounds they sit on are this shell's own, so only this shell can
    //! say whether a pairing holds. Every ratio is WCAG 2.1 relative
    //! luminance: 4.5:1 for text, 3:1 for a fill, rule or border.

    use std::collections::HashMap;

    use super::{brand_tokens, stylesheet};

    const GROUNDS: [&str; 3] = ["tc_bg", "tc_surface", "tc_surface2"];

    /// Every `@define-color` in the stylesheet, references resolved, opaque
    /// hex values only.
    fn palette(dark: bool) -> HashMap<String, String> {
        let css = stylesheet(dark);
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
        let colours = palette(dark);
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
                (ratio < floor).then(|| format!("dark={dark}: {fg} on {bg} is {ratio:.2}:1"))
            })
            .collect()
    }

    fn on_every_ground<'a>(inks: &[&'a str]) -> Vec<(&'a str, &'static str)> {
        inks.iter()
            .flat_map(|ink| GROUNDS.iter().map(move |ground| (*ink, *ground)))
            .collect()
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
}
