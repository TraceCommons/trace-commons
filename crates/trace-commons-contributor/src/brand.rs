//! The app's own name, written once.
//!
//! Every shell prints this name, and until now every shell held its own
//! copy of it: a `const` in the GTK crate, bare literals in `routing_copy`,
//! more bare literals in the Swift views. A name kept in four places is
//! four names that have not diverged *yet* -- the same argument that put
//! the routing words in [`crate::routing_copy`], applied one level up.
//!
//! # Why a macro and not just a `const`
//!
//! Most of the sentences that name us do not *only* name us; they wrap the
//! name in prose. `"Trace Commons could not use the file at {path}."` is one
//! string, and a `const` cannot be pasted into another `const`'s middle:
//! `concat!` takes literals, not `const` items. So the literal lives in
//! [`app_name!`] and [`APP_NAME`] is defined *from* the macro, which makes
//! the macro the source and the constant a view of it rather than a second
//! place to edit.
//!
//! A sentence that needs the name at compile time writes
//! `concat!(crate::app_name!(), " reads ...")`. A caller that needs it at
//! runtime -- a window title, a notification's app id -- takes [`APP_NAME`].

/// The app's name as a string literal, usable inside `concat!`.
///
/// This is the single definition. [`APP_NAME`] is built from it, and so is
/// every sentence on a user-facing surface that names us.
#[macro_export]
macro_rules! app_name {
    () => {
        "Trace Commons"
    };
}

/// The app's name, for callers that want a value rather than a literal.
pub const APP_NAME: &str = crate::app_name!();

#[cfg(test)]
mod tests {
    use super::*;

    /// The constant is the macro, not a transcription of it.
    ///
    /// Cheap to assert and it is the whole point of the module: if these
    /// two could disagree, the file would be two sources of truth wearing
    /// one name.
    #[test]
    fn the_constant_and_the_macro_are_the_same_name() {
        assert_eq!(APP_NAME, crate::app_name!());
    }

    /// A name with a brace in it would be pasted into `format!` templates
    /// on this surface and silently become an argument hole.
    #[test]
    fn the_name_carries_nothing_a_format_string_would_read() {
        assert!(!APP_NAME.contains('{'), "{APP_NAME}");
        assert!(!APP_NAME.contains('}'), "{APP_NAME}");
        assert!(!APP_NAME.is_empty());
    }
}
