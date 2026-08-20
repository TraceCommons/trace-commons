//! Every `tc-` class a view asks for must be one a stylesheet defines.
//!
//! `add_css_class` takes a string. A name with no rule behind it is not an
//! error, not a warning, and not visible in any test that asserts on
//! behaviour -- the widget simply renders in GTK's defaults. The roots screen
//! shipped with 126 passing tests and three dead class names on it
//! (`tc-muted`, `tc-error`, `tc-brand-emphasis`), and the only thing that
//! ever noticed was a photograph of the window.
//!
//! This module closes that gap the cheap way: the sources are read at compile
//! time, the class names are extracted, and each is required to exist in one
//! of the two stylesheets the crate installs.

#[cfg(test)]
mod tests {
    /// The views that style themselves. Listed rather than globbed because
    /// `include_str!` needs a literal, and because a new view that forgets to
    /// appear here is a smaller problem than one that styles itself with
    /// names nothing defines.
    const SOURCES: &[(&str, &str)] = &[
        ("roots.rs", include_str!("roots.rs")),
        ("onboarding.rs", include_str!("onboarding.rs")),
        ("history.rs", include_str!("history.rs")),
        ("settings.rs", include_str!("settings.rs")),
        ("queue.rs", include_str!("queue.rs")),
        ("preview.rs", include_str!("preview.rs")),
        ("update.rs", include_str!("update.rs")),
        ("mark.rs", include_str!("mark.rs")),
    ];

    const STYLE_CSS: &str = include_str!("style.css");
    const BRAND_CSS: &str = include_str!("community_brand.rs");

    /// Class names owned by GTK or libadwaita, which our stylesheets do not
    /// define and must not be expected to.
    const UPSTREAM: &[&str] = &[
        "flat",
        "suggested-action",
        "destructive-action",
        "dim-label",
        "heading",
        "title",
        "subtitle",
        "body",
        "caption",
        "error",
        "warning",
        "success",
        "accent",
        "linked",
        "osd",
        "toolbar",
        "card",
        "boxed-list",
        "navigation-sidebar",
        "monospace",
        "numeric",
        "pill",
        "circular",
    ];

    /// Dead class names that already existed when this check was written.
    ///
    /// EMPTY, and it should stay that way. It held ten entries for one round:
    /// six were this check's own blindness to compound selectors (see
    /// `is_defined`) and were never defects at all, and the remaining four
    /// were real, all in `onboarding.rs`, all fixed by photographing the pages
    /// under Xvfb -- `scripts/linux-build.sh --onboarding-shots` -- and moving
    /// each name onto the class the surrounding code already used for that
    /// job.
    ///
    /// Nothing may be added to this list. A new dead class is a bug in the
    /// change that introduced it, not a debt to be recorded here.
    const QUARANTINE: &[(&str, &str)] = &[];

    /// Pull every literal passed to `add_css_class` / `remove_css_class`.
    fn classes_used(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for call in ["add_css_class(\"", "remove_css_class(\""] {
            let mut rest = source;
            while let Some(at) = rest.find(call) {
                rest = &rest[at + call.len()..];
                if let Some(end) = rest.find('"') {
                    found.push(rest[..end].to_string());
                }
            }
        }
        found
    }

    /// Is `class` named anywhere in a selector, in either sheet?
    ///
    /// The first version of this only looked at the START of a line, which
    /// made it blind to a class that is never the first name in its selector.
    /// Six of the ten entries this check originally quarantined were that
    /// blindness rather than real defects: `.tc-card.tc-flagged`,
    /// `.tc-brand-cell.tc-brand-divided`, `.tc-brand-field.tc-brand-mono` and
    /// `.tc-brand-button.tc-brand-primary` are all live rules, and every call
    /// site applies the partner class alongside.
    ///
    /// The residual blind spot, stated rather than papered over: this proves a
    /// rule MENTIONS the class, not that the widget wearing it also wears the
    /// partner a compound selector needs. `add_css_class("tc-flagged")` on
    /// something that is not a `.tc-card` still renders bare and still passes
    /// here. Catching that needs to know what else is on the widget, which is
    /// a dataflow question this cannot answer by reading text.
    fn is_defined(class: &str) -> bool {
        let selector = format!(".{class}");
        let ends_name =
            |c: char| c.is_whitespace() || c == ',' || c == '{' || c == ':' || c == '.' || c == '>';
        for sheet in [STYLE_CSS, BRAND_CSS] {
            for line in sheet.lines() {
                let line = line.trim();
                // Only selector lines. A declaration such as `color: #fff;`
                // cannot define anything, and skipping them keeps a property
                // value that happens to contain the text from counting.
                if line.starts_with("/*") || line.starts_with('*') {
                    continue;
                }
                let mut rest = line;
                while let Some(at) = rest.find(&selector) {
                    let tail = &rest[at + selector.len()..];
                    // `.tc-quiet` must not be satisfied by `.tc-quiet-thing`:
                    // the next character has to end the name.
                    // No check on the character BEFORE is needed, and adding
                    // one is a bug: in `.tc-card.tc-flagged` the character
                    // before `.tc-flagged` is `d`, so requiring a separator
                    // there would reject the compound rules this exists to
                    // see. A `.` cannot occur inside a class name, so matching
                    // one already puts us at a name boundary.
                    if tail.is_empty() || tail.starts_with(ends_name) {
                        return true;
                    }
                    rest = &rest[at + 1..];
                }
            }
        }
        false
    }

    #[test]
    fn every_styled_class_has_a_rule_behind_it() {
        let mut dead: Vec<String> = Vec::new();
        for (file, source) in SOURCES {
            for class in classes_used(source) {
                if UPSTREAM.contains(&class.as_str()) || is_defined(&class) {
                    continue;
                }
                if QUARANTINE.contains(&(file, class.as_str())) {
                    continue;
                }
                dead.push(format!("{file}: .{class}"));
            }
        }
        dead.sort();
        dead.dedup();
        assert!(
            dead.is_empty(),
            "these classes are asked for but defined by no stylesheet, so the \
             widgets wearing them render unstyled and nothing fails:\n  {}",
            dead.join("\n  ")
        );
    }

    #[test]
    fn the_quarantine_holds_nothing_that_is_already_fixed() {
        // A stale entry is worse than none: it silently re-permits a name
        // someone has since defined, and it makes the list look larger than
        // the debt actually is.
        for (file, class) in QUARANTINE {
            let source = SOURCES
                .iter()
                .find(|(name, _)| name == file)
                .map(|(_, source)| *source)
                .unwrap_or_else(|| panic!("quarantine names {file}, which is not in SOURCES"));
            assert!(
                classes_used(source).iter().any(|used| used == class),
                "quarantine says {file} uses .{class}, but it no longer does -- delete the line"
            );
            assert!(
                !is_defined(class),
                ".{class} is defined now, so the {file} quarantine line is stale -- delete it"
            );
        }
    }

    /// A view that wears `tc-brand-*` must install the sheet that defines it.
    ///
    /// A defined class is not a rendered one. The roots screen drew with no
    /// stylesheet at all because `style::install()` ran only from `App::build`,
    /// which needs a started daemon -- and that screen exists precisely
    /// because the daemon did not start. `every_styled_class_has_a_rule_behind_it`
    /// could not have caught it: the rules existed, the provider did not.
    ///
    /// This holds with no exceptions today. If a new view needs one, the
    /// honest fix is almost always to call `install()` -- it is idempotent and
    /// costs a bool check -- rather than to reason about which window opened
    /// first, which is the reasoning that failed here twice.
    #[test]
    fn a_view_wearing_brand_classes_installs_the_brand_sheet() {
        for (file, source) in SOURCES {
            let wears_brand = classes_used(source)
                .iter()
                .any(|class| class.starts_with("tc-brand-"));
            if !wears_brand {
                continue;
            }
            assert!(
                source.contains("community_brand::install()"),
                "{file} applies tc-brand-* classes but never calls \
                 community_brand::install(), so on any path where it is the \
                 first window shown those widgets render bare"
            );
        }
    }

    #[test]
    fn a_class_named_only_in_a_compound_selector_counts_as_defined() {
        // The bug that made this check report six false defects. Each of these
        // is the second name in a rule and is defined by nothing else, and
        // every call site applies the partner class beside it.
        for class in [
            "tc-flagged",
            "tc-brand-divided",
            "tc-brand-mono",
            "tc-brand-primary",
        ] {
            assert!(
                is_defined(class),
                ".{class} is defined as the second half of a compound selector \
                 and must count as defined"
            );
        }
        // Still not fooled by a longer name that merely starts the same way.
        assert!(!is_defined("tc-card-"));
        assert!(!is_defined("tc-fl"));
    }

    #[test]
    fn the_extractor_actually_finds_names() {
        // Guards the guard: a `classes_used` that silently returned nothing
        // would make the test above pass for every possible input.
        let found = classes_used(r#"x.add_css_class("tc-ledger"); y.add_css_class("flat");"#);
        assert_eq!(found, vec!["tc-ledger".to_string(), "flat".to_string()]);
        assert!(is_defined("tc-ledger"), "tc-ledger is in style.css");
        assert!(!is_defined("tc-definitely-not-a-real-class"));
    }
}
