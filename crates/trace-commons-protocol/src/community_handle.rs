//! Validation rules for self-declared community handles used by the
//! public-attribution opt-in surface
//! (`/v1/community/profile`). The rules are deliberately strict so the
//! handle can be used as a URL slug, as a stable join key in cached
//! snapshots, and as a display string without further escaping.
//!
//! The rules are NOT a substitute for a profanity / impersonation review;
//! they're the floor that catches malformed input before it ever reaches
//! that review.

use std::collections::BTreeSet;

/// Inclusive length bounds for `display_handle`.
pub const HANDLE_MIN_LEN: usize = 3;
pub const HANDLE_MAX_LEN: usize = 32;

/// Inclusive byte cap for `bio` (UTF-8 bytes, not chars — matches the
/// way Postgres TEXT length is bounded operationally).
pub const BIO_MAX_BYTES: usize = 280;

/// Reserved handles that contributors may not claim. Operator-owned
/// names (admin, system, root) plus the project's branding terms so
/// the operator can stand up an official profile later without
/// fighting a squatter.
pub const PILOT_RESERVED_HANDLES: &[&str] = &[
    "admin",
    "administrator",
    "anonymous",
    "api",
    "billing",
    "community",
    "contact",
    "ironclaw",
    "leaderboard",
    "legal",
    "moderator",
    "operator",
    "owner",
    "privacy",
    "profile",
    "root",
    "security",
    "staff",
    "support",
    "system",
    "team",
    "trace",
    "trace-commons",
    "tracecommons",
];

/// Reason a candidate handle was rejected. Returned to the contributor
/// so they can correct the input; never logged with the raw handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleValidationError {
    /// Empty or below `HANDLE_MIN_LEN`.
    TooShort,
    /// Above `HANDLE_MAX_LEN`.
    TooLong,
    /// Contains a character outside `[a-z0-9_-]` (after normalisation).
    InvalidCharacter,
    /// Starts or ends with `-` or `_`.
    InvalidBoundary,
    /// Contains consecutive `-` or `_` (e.g. `foo--bar`, `foo__bar`).
    ConsecutiveSeparators,
    /// Matches a `PILOT_RESERVED_HANDLES` entry after normalisation.
    Reserved,
}

/// Returned alongside a successfully-validated handle so the caller
/// can persist both the display form (preserving the contributor's
/// chosen case) and the normalised form (for uniqueness).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedHandle {
    pub display: String,
    pub normalized: String,
}

/// Validate a community handle and return its display + normalised
/// forms.
///
/// Rules:
/// - Length in `[HANDLE_MIN_LEN, HANDLE_MAX_LEN]`.
/// - Characters: ASCII `[a-zA-Z0-9_-]`. Unicode handles are
///   intentionally not supported in the pilot — confusable-character
///   attacks are a separate review.
/// - First and last characters must be alphanumeric.
/// - No consecutive `-` or `_`.
/// - Normalised (lowercased) form must not be in
///   `PILOT_RESERVED_HANDLES`.
pub fn validate_handle(input: &str) -> Result<ValidatedHandle, HandleValidationError> {
    let trimmed = input.trim();
    if trimmed.len() < HANDLE_MIN_LEN {
        return Err(HandleValidationError::TooShort);
    }
    if trimmed.len() > HANDLE_MAX_LEN {
        return Err(HandleValidationError::TooLong);
    }
    let mut prev_separator = false;
    for (i, byte) in trimmed.bytes().enumerate() {
        let is_alnum = byte.is_ascii_alphanumeric();
        let is_separator = byte == b'-' || byte == b'_';
        if !is_alnum && !is_separator {
            return Err(HandleValidationError::InvalidCharacter);
        }
        if (i == 0 || i == trimmed.len() - 1) && is_separator {
            return Err(HandleValidationError::InvalidBoundary);
        }
        if is_separator && prev_separator {
            return Err(HandleValidationError::ConsecutiveSeparators);
        }
        prev_separator = is_separator;
    }
    let normalized = trimmed.to_ascii_lowercase();
    let reserved: BTreeSet<&&str> = PILOT_RESERVED_HANDLES.iter().collect();
    if reserved.contains(&normalized.as_str()) {
        return Err(HandleValidationError::Reserved);
    }
    Ok(ValidatedHandle {
        display: trimmed.to_string(),
        normalized,
    })
}

/// Validate a bio. Returns `Ok(())` for empty (bio is optional).
pub fn validate_bio(input: &str) -> Result<(), BioValidationError> {
    if input.len() > BIO_MAX_BYTES {
        return Err(BioValidationError::TooLong);
    }
    if input.chars().any(|c| c == '\0' || c.is_control() && c != '\n') {
        return Err(BioValidationError::InvalidCharacter);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BioValidationError {
    TooLong,
    /// Contains NUL or non-newline control characters.
    InvalidCharacter,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_handles() {
        let v = validate_handle("zaki").unwrap();
        assert_eq!(v.display, "zaki");
        assert_eq!(v.normalized, "zaki");
    }

    #[test]
    fn preserves_display_case_normalises_for_uniqueness() {
        let v = validate_handle("ZakiManian").unwrap();
        assert_eq!(v.display, "ZakiManian");
        assert_eq!(v.normalized, "zakimanian");
    }

    #[test]
    fn rejects_too_short() {
        assert_eq!(
            validate_handle("ab"),
            Err(HandleValidationError::TooShort),
        );
    }

    #[test]
    fn rejects_too_long() {
        let too_long = "a".repeat(HANDLE_MAX_LEN + 1);
        assert_eq!(
            validate_handle(&too_long),
            Err(HandleValidationError::TooLong),
        );
    }

    #[test]
    fn rejects_invalid_characters() {
        for bad in ["abc def", "abc.def", "abc!def", "abc/def", "café", "ab@c"] {
            assert_eq!(
                validate_handle(bad),
                Err(HandleValidationError::InvalidCharacter),
                "expected reject: {bad}",
            );
        }
    }

    #[test]
    fn rejects_separator_boundaries() {
        for bad in ["-abc", "abc-", "_abc", "abc_"] {
            assert_eq!(
                validate_handle(bad),
                Err(HandleValidationError::InvalidBoundary),
                "expected reject: {bad}",
            );
        }
    }

    #[test]
    fn rejects_consecutive_separators() {
        for bad in ["foo--bar", "foo__bar", "foo-_bar", "foo_-bar"] {
            assert_eq!(
                validate_handle(bad),
                Err(HandleValidationError::ConsecutiveSeparators),
                "expected reject: {bad}",
            );
        }
    }

    #[test]
    fn rejects_reserved_handles_case_insensitive() {
        for reserved in ["admin", "Admin", "ADMIN", "trace-commons"] {
            assert_eq!(
                validate_handle(reserved),
                Err(HandleValidationError::Reserved),
                "expected reject: {reserved}",
            );
        }
    }

    #[test]
    fn trims_surrounding_whitespace_before_validating() {
        let v = validate_handle("  zaki  ").unwrap();
        assert_eq!(v.display, "zaki");
        assert_eq!(v.normalized, "zaki");
    }

    #[test]
    fn accepts_internal_single_separators() {
        validate_handle("foo-bar").unwrap();
        validate_handle("foo_bar").unwrap();
        validate_handle("foo-bar_baz").unwrap();
    }

    #[test]
    fn validate_bio_empty_ok() {
        validate_bio("").unwrap();
    }

    #[test]
    fn validate_bio_caps_length() {
        let too_long = "a".repeat(BIO_MAX_BYTES + 1);
        assert_eq!(
            validate_bio(&too_long),
            Err(BioValidationError::TooLong),
        );
    }

    #[test]
    fn validate_bio_allows_newlines_rejects_null() {
        validate_bio("line one\nline two").unwrap();
        assert_eq!(
            validate_bio("contains\0null"),
            Err(BioValidationError::InvalidCharacter),
        );
    }
}
