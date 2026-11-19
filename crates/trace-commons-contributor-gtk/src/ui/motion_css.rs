//! The Mac's control motion as CSS transitions, written from the motion
//! tokens (`brand_tokens::motion`) when the stylesheet loads, since GTK's
//! CSS has no variables to read them from.
//!
//! - Hover fill: a glass control's hover fill fades in and out over
//!   `motion.fast` on the token curve (`GlassTierModifier`,
//!   `GlassHoverFill`: `.animation(GlassMotion.fast, value: fill)`).
//! - Press: the fill goes `GlassPress.darkening` (8%) darker over the same
//!   fast curve, drawn as a black wash under the label so the label keeps
//!   its contrast (`glassPressedFill`, `glassPressedWash`).
//! - What the Mac does not animate snaps here too: a list row's hover and
//!   selection (`GlassListRow`, no animation) and the kebab's lit fill
//!   (`GlassKebab`, only its press fades).
//! - Buttons with no glass class of their own take the same fast token
//!   curve in place of the toolkit's 200 ms ease (`GlassButtonStyle`).
//! - Switch: the track's colour changes on SwiftUI's `.easeOut` over
//!   `motion.standard`, or `motion.fast` for a tree row's watch switch
//!   (`GlassToggleStyle.duration`); the knob is `glass_switch`'s.
//! - Expander: the chevron turns from pointing at the label to pointing
//!   down over `motion.fast` (`GlassExpander`), or `motion.standard` for a
//!   first-run folder's past traces (`ToolsScreen` repo disclosure); the
//!   toolkit swaps its icon instead, which this replaces.
//!
//! GTK itself drops every CSS transition while `gtk-enable-animations` is
//! off, and picks the setting up live, which is Reduce Motion's nil.

use super::style::brand_tokens::motion;

/// `GlassPress.darkening`: how much darker a pressed fill is.
pub const PRESS_DARKENING: f64 = 0.08;

/// SwiftUI's `.easeOut` as a CSS timing function.
pub const EASE_OUT_CSS: &str = "cubic-bezier(0, 0, 0.58, 1)";

/// The glass controls whose hover replaces their fill on the Mac, animated:
/// the toolbar's buttons (`GlassToolbarButton`), the graph's pills
/// (`GlassPillIconButton`), a tree's round control (`GlassRoundButton`) and
/// a row's Submit split button (`GlassSplitButton`).
pub const HOVER_FADES: &[&str] = &[
    ".tc-toolbar-toggle",
    ".tc-graph-pill",
    ".tc-tree-round",
    ".tc-tree-submit",
    ".tc-tree-split-menu > button",
];

/// Controls whose hover snaps on the Mac but whose press still fades.
pub const PRESS_ONLY: &[&str] = &[".tc-tree-kebab > button"];

/// Rows the Mac never animates, hover or selection.
pub const SNAPS: &[&str] = &[".tc-tree-row"];

/// A duration in seconds as CSS milliseconds.
fn ms(seconds: f64) -> String {
    format!("{}ms", (seconds * 1000.0).round())
}

/// The motion rules, appended after the hand-written sheet so they win
/// over the toolkit's own transitions.
pub fn css() -> String {
    let fast = ms(motion::FAST);
    let standard = ms(motion::STANDARD);
    let ease = motion::EASE_CSS;
    let wash = format!("box-shadow: inset 0 0 0 999px rgba(0, 0, 0, {PRESS_DARKENING});");
    let active = |list: &[&str]| {
        list.iter()
            .map(|s| format!("{s}:active"))
            .collect::<Vec<_>>()
            .join(",\n")
    };
    let mut out = String::from("/* Motion (`ui::motion_css`): the Mac's control timings. */\n");
    out.push_str(&format!(
        "button {{\n  transition-duration: {fast};\n  transition-timing-function: {ease};\n}}\n"
    ));
    out.push_str(&format!(
        "{} {{\n  transition: background-color {fast} {ease}, box-shadow {fast} {ease};\n}}\n",
        HOVER_FADES.join(",\n")
    ));
    out.push_str(&format!(
        "{} {{\n  transition: box-shadow {fast} {ease};\n}}\n",
        PRESS_ONLY.join(",\n")
    ));
    out.push_str(&format!(
        "{} {{\n  transition: none;\n}}\n",
        SNAPS.join(",\n")
    ));
    let pressed: Vec<&str> = HOVER_FADES.iter().chain(PRESS_ONLY).copied().collect();
    out.push_str(&format!("{} {{\n  {wash}\n}}\n", active(&pressed)));
    // The switch's track; its knob moves in `glass_switch`.
    out.push_str(&format!(
        "switch {{\n  transition: background-color {standard} {EASE_OUT_CSS}, box-shadow {fast} {ease};\n}}\n\
         switch.tc-tree-switch {{\n  transition: background-color {fast} {EASE_OUT_CSS}, box-shadow {fast} {ease};\n}}\n"
    ));
    // The expander's chevron: one icon, turned, so it can turn smoothly.
    let chevron = "expander-widget > box > title > expander";
    out.push_str(&format!(
        "{chevron} {{\n  transition: -gtk-icon-transform {fast} {ease};\n}}\n\
         expander-widget.tc-disclosure-standard > box > title > expander {{\n  transition: -gtk-icon-transform {standard} {ease};\n}}\n\
         {chevron}:checked {{\n  -gtk-icon-source: -gtk-icontheme(\"pan-end-symbolic\");\n  -gtk-icon-transform: rotate(90deg);\n}}\n\
         {chevron}:checked:dir(rtl) {{\n  -gtk-icon-source: -gtk-icontheme(\"pan-end-symbolic-rtl\");\n  -gtk-icon-transform: rotate(-90deg);\n}}\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLS: &str =
        include_str!("../../../../macos/Sources/TCDesign/Components/Controls.swift");
    const PRESS: &str = include_str!("../../../../macos/Sources/TCDesign/Styling/GlassPress.swift");
    const STYLE: &str = include_str!("../../../../macos/Sources/TCDesign/Styling/GlassStyle.swift");
    const HOVER: &str = include_str!("../../../../macos/Sources/TCDesign/Styling/GlassHover.swift");
    const LIST_ROW: &str =
        include_str!("../../../../macos/Sources/TCDesign/Components/ListRow.swift");

    /// The rule for `selector`, as written.
    fn rule(css: &str, selector: &str) -> String {
        let start = css
            .find(&format!("{selector} {{"))
            .or_else(|| css.find(&format!("{selector},")))
            .unwrap_or_else(|| panic!("no rule for {selector}"));
        let open = start + css[start..].find('{').unwrap();
        let close = open + css[open..].find('}').unwrap();
        css[open + 1..close].to_string()
    }

    /// The hover fill and the press take `GlassMotion.fast` on the token
    /// curve, as the Mac's tier, hover and press modifiers do.
    #[test]
    fn hover_and_press_fade_over_the_macs_fast_curve() {
        assert!(STYLE.contains(".animation(GlassMotion.fast(reduceMotion), value: fill)"));
        assert!(HOVER.contains(".animation(GlassMotion.fast(reduceMotion), value: shows)"));
        assert!(PRESS.contains("public static let darkening: Double = 0.08"));
        assert!(PRESS.contains(".animation(GlassMotion.fast(reduceMotion), value: pressed)"));
        let css = css();
        for selector in HOVER_FADES {
            let body = rule(&css, selector);
            assert!(
                body.contains("background-color 150ms cubic-bezier(0.2, 0.8, 0.2, 1)"),
                "{selector}: {body}"
            );
        }
        for selector in HOVER_FADES.iter().chain(PRESS_ONLY) {
            let active = format!("{selector}:active");
            assert!(css.contains(&active), "{active}");
        }
        assert!(css.contains("box-shadow: inset 0 0 0 999px rgba(0, 0, 0, 0.08);"));
    }

    /// The kebab's lit fill and a list row's hover snap on the Mac: no
    /// animation on either. Only the kebab's press fades.
    #[test]
    fn what_the_mac_snaps_snaps() {
        assert!(CONTROLS.contains(".fill(lit ? Self.litFill(ground: ground) : .clear)"));
        assert!(LIST_ROW.contains(
            ".fill(Self.fill(selected: selected, hovering: hovering)?.color ?? Color.clear)"
        ));
        assert!(!LIST_ROW.contains(".animation("));
        let css = css();
        let kebab = rule(&css, PRESS_ONLY[0]);
        assert!(!kebab.contains("background-color"), "{kebab}");
        assert!(kebab.contains("box-shadow 150ms"), "{kebab}");
        assert_eq!(rule(&css, SNAPS[0]).trim(), "transition: none;");
    }

    /// `GlassToggleStyle`: the track on `.easeOut` over standard (0.22 s),
    /// the watch switch over fast (0.15 s).
    #[test]
    fn the_switch_track_eases_out_over_the_macs_durations() {
        assert!(
            CONTROLS
                .contains("kind == .watch ? GlassTokens.Motion.fast : GlassTokens.Motion.standard")
        );
        assert!(CONTROLS.contains(".easeOut(duration: Self.duration(kind))"));
        let css = css();
        assert!(css.contains(
            "switch {\n  transition: background-color 220ms cubic-bezier(0, 0, 0.58, 1)"
        ));
        assert!(css.contains("switch.tc-tree-switch {\n  transition: background-color 150ms cubic-bezier(0, 0, 0.58, 1)"));
    }

    /// `GlassExpander`: the chevron turns 90 degrees over fast; the
    /// first-run folder disclosure over standard.
    #[test]
    fn the_chevron_turns_over_the_macs_durations() {
        assert!(CONTROLS.contains(".rotationEffect(.degrees(isOpen ? 90 : 0))"));
        assert!(CONTROLS.contains(
            "withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) { isOpen.toggle() }"
        ));
        let tools = include_str!(
            "../../../../macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift"
        );
        assert!(tools.contains(".animation(GlassMotion.standard(reduceMotion), value: isOpen)"));
        let css = css();
        assert!(css.contains("expander-widget > box > title > expander {\n  transition: -gtk-icon-transform 150ms cubic-bezier(0.2, 0.8, 0.2, 1);"));
        assert!(css.contains("expander-widget.tc-disclosure-standard > box > title > expander {\n  transition: -gtk-icon-transform 220ms"));
        assert!(css.contains("-gtk-icon-transform: rotate(90deg);"));
        assert!(css.contains("-gtk-icon-transform: rotate(-90deg);"));
    }

    /// Every stylesheet the shell loads carries the motion rules.
    #[test]
    fn every_sheet_carries_the_motion_rules() {
        for dark in [false, true] {
            assert!(super::super::style::stylesheet(dark).contains(&css()));
        }
    }

    /// GTK parses every motion rule: a property it does not know would be
    /// dropped with only a parse error to say so.
    #[test]
    #[ignore = "requires a GTK display; run alone with --ignored --test-threads=1"]
    fn mounted_gtk_parses_the_motion_rules() {
        gtk::init().expect("GTK display");
        let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = errors.clone();
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(move |_, section, error| {
            captured.borrow_mut().push(format!("{section}: {error}"));
        });
        provider.load_from_data(&css());
        assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    }
}
