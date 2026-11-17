//! The first run's dark stage, as the Mac forces its first-run stage dark
//! (`OnboardingCoordinatorView`).
//!
//! Two ways, chosen at run time (owner, 2026-11-17):
//!
//! - **Scoped**, on GTK 4.16 and libadwaita 1.6 or later. The stage's
//!   windows wear [`STAGE_CLASS`], and one display provider carries the dark
//!   stylesheet's rules under that class, its colours written out, with
//!   libadwaita's own colour variables set to the dark palette so native
//!   widgets inside the stage are dark too. Nothing outside the stage
//!   changes: the main window keeps the person's scheme.
//! - **App-wide**, below that. libadwaita's scheme is app-wide there and
//!   its widgets read no variables, so the stage forces the dark scheme
//!   while a stage window is up and puts the person's own back when the
//!   last one goes, as before.
use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

/// The class the stage's windows wear in the scoped mode.
pub(crate) const STAGE_CLASS: &str = "tc-stage-dark";

/// How the stage is made dark on this system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageMode {
    Scoped,
    AppWide,
}

/// The mode for GTK `gtk` and libadwaita `adw`, each as (major, minor).
/// Scoped needs CSS custom properties (GTK 4.16) and libadwaita widgets
/// that read their colours from them (libadwaita 1.6).
pub(crate) fn stage_mode(gtk: (u32, u32), adw: (u32, u32)) -> StageMode {
    if gtk >= (4, 16) && adw >= (1, 6) {
        StageMode::Scoped
    } else {
        StageMode::AppWide
    }
}

/// The mode on the running system.
pub(crate) fn current_mode() -> StageMode {
    stage_mode(
        (gtk::major_version(), gtk::minor_version()),
        (adw::major_version(), adw::minor_version()),
    )
}

/// The dark stylesheet `dark_css`, scoped to [`STAGE_CLASS`]: every rule's
/// selectors under the class, every `@name` written out as its dark value,
/// and libadwaita's colour variables (`--window-bg-color` for
/// `window_bg_color`, and so on) set on the class from the same palette.
pub(crate) fn scope_dark(dark_css: &str) -> String {
    let css = strip_comments(dark_css);
    let mut defines: Vec<(String, String)> = Vec::new();
    let mut rules: Vec<(String, String)> = Vec::new();
    let mut rest = css.as_str();
    loop {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            break;
        }
        if let Some(define) = trimmed.strip_prefix("@define-color") {
            let end = define.find(';').unwrap_or(define.len());
            if let Some((name, value)) = define[..end].trim().split_once(char::is_whitespace) {
                defines.push((name.to_owned(), value.trim().to_owned()));
            }
            rest = define.get(end + 1..).unwrap_or("");
            continue;
        }
        let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.find('}')) else {
            break;
        };
        rules.push((
            trimmed[..open].trim().to_owned(),
            trimmed[open + 1..close].trim().to_owned(),
        ));
        rest = &trimmed[close + 1..];
    }
    // Later definitions win, as they do in a stylesheet.
    let table: std::collections::HashMap<&str, &str> = defines
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let resolve = |text: &str| substitute(text, &table);
    let mut out = format!(".{STAGE_CLASS} {{\n");
    let mut seen = std::collections::HashSet::new();
    for (name, _) in defines.iter().rev() {
        if name.starts_with("tc_") || !name.ends_with("_color") || !seen.insert(name.as_str()) {
            continue;
        }
        let value = resolve(table[name.as_str()]);
        out.push_str(&format!("  --{}: {value};\n", name.replace('_', "-")));
    }
    // libadwaita's dark shades, which it does not take from the palette.
    out.push_str("  --shade-color: rgb(0 0 6 / 25%);\n");
    out.push_str("  --scrollbar-outline-color: rgb(0 0 12 / 95%);\n");
    if let Some(ink) = table.get("window_fg_color") {
        out.push_str(&format!("  color: {};\n", resolve(ink)));
    }
    out.push_str("}\n");
    for (selectors, body) in rules {
        let scoped: Vec<String> = selectors
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .flat_map(scope_selector)
            .collect();
        out.push_str(&format!("{} {{ {} }}\n", scoped.join(", "), resolve(&body)));
    }
    out
}

/// One selector under the stage class: inside a stage window, and, for a
/// selector that starts at `window`, the stage window itself.
fn scope_selector(selector: &str) -> Vec<String> {
    let mut scoped = vec![format!(".{STAGE_CLASS} {selector}")];
    if let Some(rest) = selector.strip_prefix("window") {
        if !rest.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_') {
            scoped.push(format!("window.{STAGE_CLASS}{rest}"));
        }
    }
    scoped
}

/// `text` with each `@name` replaced by its value, references followed.
/// An unknown name is left as written.
fn substitute(text: &str, table: &std::collections::HashMap<&str, &str>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let len = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(after.len());
        let name = &after[..len];
        match table.get(name) {
            Some(value) if !value.contains(&format!("@{name}")) => {
                out.push_str(&substitute(value, table));
            }
            _ => {
                out.push('@');
                out.push_str(name);
            }
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

/// Remove `/* ... */` comments.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

thread_local! {
    static SCOPED_INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// App-wide mode: how many stage windows (and handoffs) are up, and the
    /// person's own scheme to put back when the count reaches zero.
    static HOLDERS: Cell<(usize, adw::ColorScheme)> =
        const { Cell::new((0, adw::ColorScheme::Default)) };
}

/// The scoped stage's provider, once per process.
fn install_scoped() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    if SCOPED_INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&scope_dark(&super::style::stylesheet(true)));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
}

fn acquire() {
    let manager = adw::StyleManager::default();
    HOLDERS.with(|holders| {
        let (count, previous) = holders.get();
        let previous = if count == 0 {
            manager.color_scheme()
        } else {
            previous
        };
        holders.set((count + 1, previous));
    });
    manager.set_color_scheme(adw::ColorScheme::ForceDark);
}

fn release() {
    HOLDERS.with(|holders| {
        let (count, previous) = holders.get();
        let count = count.saturating_sub(1);
        holders.set((count, previous));
        if count == 0 {
            adw::StyleManager::default().set_color_scheme(previous);
        }
    });
}

/// Holds the stage dark for a window, or across a handoff between windows.
pub(crate) struct StageScheme;

thread_local! {
    static HANDOFF: Cell<bool> = const { Cell::new(false) };
}

impl StageScheme {
    /// Make `window` part of the dark stage until it goes. In the app-wide
    /// mode this is counted, so two windows that overlap cannot put the
    /// forced scheme back as the person's.
    pub(crate) fn hold(window: &impl IsA<gtk::Window>) {
        let window = window.upcast_ref::<gtk::Window>();
        if current_mode() == StageMode::Scoped {
            install_scoped();
            window.add_css_class(STAGE_CLASS);
            return;
        }
        acquire();
        // GTK 4 emits `destroy` at dispose, which waits for the last
        // reference; `gtk_window_destroy` unrealizes at once. Release on
        // whichever comes first, exactly once per hold.
        let held = Rc::new(Cell::new(true));
        let once = move || {
            if held.replace(false) {
                release();
            }
        };
        window.connect_unrealize({
            let once = once.clone();
            move |_| once()
        });
        window.connect_destroy(move |_| once());
    }

    /// Keep the stage dark from one first-run window closing until the
    /// next one holds it (the roots window, then the coordinator), so the
    /// person's scheme does not flash in between. Released by
    /// [`Self::end_handoff`], or after `limit` if no window takes it.
    pub(crate) fn begin_handoff(limit: std::time::Duration) {
        if current_mode() == StageMode::Scoped || HANDOFF.with(|h| h.replace(true)) {
            return;
        }
        acquire();
        gtk::glib::timeout_add_local_once(limit, Self::end_handoff);
    }

    /// The next window holds the stage: the handoff's hold goes.
    pub(crate) fn end_handoff() {
        if HANDOFF.with(|h| h.replace(false)) {
            release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_needs_gtk_4_16_and_libadwaita_1_6() {
        assert_eq!(stage_mode((4, 16), (1, 6)), StageMode::Scoped);
        assert_eq!(stage_mode((4, 22), (1, 8)), StageMode::Scoped);
        assert_eq!(stage_mode((4, 14), (1, 5)), StageMode::AppWide, "GNOME 46");
        assert_eq!(stage_mode((4, 16), (1, 5)), StageMode::AppWide);
        assert_eq!(stage_mode((4, 15), (1, 6)), StageMode::AppWide);
        assert_eq!(stage_mode((5, 0), (2, 0)), StageMode::Scoped);
    }

    fn rules(css: &str) -> Vec<(String, String)> {
        let css = strip_comments(css);
        let mut out = Vec::new();
        let mut rest = css.as_str();
        loop {
            let trimmed = rest.trim_start();
            if trimmed.is_empty() {
                return out;
            }
            if trimmed.starts_with('@') {
                rest = &trimmed[trimmed.find(';').unwrap() + 1..];
                continue;
            }
            let open = trimmed.find('{').unwrap();
            let close = trimmed.find('}').unwrap();
            out.push((
                trimmed[..open].trim().to_owned(),
                trimmed[open + 1..close].trim().to_owned(),
            ));
            rest = &trimmed[close + 1..];
        }
    }

    #[test]
    fn the_scoped_sheet_is_every_dark_rule_under_the_stage_class() {
        let dark = super::super::style::stylesheet(true);
        let scoped = scope_dark(&dark);
        assert!(!scoped.contains('@'), "every colour is written out");
        let source = rules(&dark);
        let out = rules(&scoped);
        assert_eq!(out.len(), source.len() + 1, "the variables, then each rule");
        for (selectors, _) in &out {
            for selector in selectors.split(',') {
                assert!(
                    selector.trim().starts_with(".tc-stage-dark")
                        || selector.trim().starts_with("window.tc-stage-dark"),
                    "{selector}"
                );
            }
        }
    }

    #[test]
    fn native_widgets_in_the_stage_read_the_dark_palette() {
        let dark = super::super::style::stylesheet(true);
        let light = super::super::style::stylesheet(false);
        let scoped = scope_dark(&dark);
        let value = |css: &str, name: &str| {
            let line = css
                .lines()
                .find(|l| l.trim_start().starts_with(&format!("{name}:")))
                .unwrap_or_else(|| panic!("{name}"));
            line.split_once(':')
                .unwrap()
                .1
                .trim()
                .trim_end_matches(';')
                .to_owned()
        };
        let bg = value(&scoped, "--window-bg-color");
        assert!(bg.starts_with('#'), "{bg}");
        assert!(
            !light.contains(&format!("tc_bg        {bg}")),
            "the stage ground is not the light one"
        );
        for var in [
            "--window-fg-color",
            "--view-bg-color",
            "--accent-bg-color",
            "--headerbar-bg-color",
        ] {
            assert!(!value(&scoped, var).is_empty(), "{var}");
        }
    }
}
