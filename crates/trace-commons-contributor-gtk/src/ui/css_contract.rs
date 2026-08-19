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
    /// Every one is a real defect: the widget wearing it renders unstyled.
    /// They are quarantined rather than fixed here because choosing the right
    /// replacement needs a photograph of the screen in question, and only the
    /// roots screen has ever been photographed. Fixing one means capturing
    /// that view under Xvfb -- `scripts/linux-build.sh --roots-shot` is the
    /// pattern -- deciding what it should have looked like, and deleting the
    /// line from this list.
    ///
    /// Nothing may be added to this list. It exists to stop the bleeding,
    /// not to license more of it.
    const QUARANTINE: &[(&str, &str)] = &[
        ("history.rs", "tc-brand-divided"),
        ("onboarding.rs", "tc-brand-emphasis"),
        ("onboarding.rs", "tc-error"),
        ("onboarding.rs", "tc-muted"),
        ("onboarding.rs", "tc-section-header"),
        ("preview.rs", "tc-flagged"),
        ("queue.rs", "tc-flagged"),
        ("settings.rs", "tc-brand-divided"),
        ("settings.rs", "tc-brand-mono"),
        ("settings.rs", "tc-brand-primary"),
    ];

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

    fn is_defined(class: &str) -> bool {
        let selector = format!(".{class}");
        for sheet in [STYLE_CSS, BRAND_CSS] {
            for line in sheet.lines() {
                let line = line.trim();
                if !line.starts_with(&selector) {
                    continue;
                }
                // `.tc-quiet` must not be satisfied by `.tc-quiet-thing`: the
                // next character has to end the selector rather than continue
                // the name.
                let tail = &line[selector.len()..];
                if tail.is_empty()
                    || tail.starts_with(|c: char| {
                        c.is_whitespace() || c == ',' || c == '{' || c == ':' || c == '.'
                    })
                {
                    return true;
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
