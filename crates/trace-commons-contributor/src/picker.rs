//! Pure helpers for the interactive session picker: parsing a selection
//! line ("3", "1,3-5", "a"/"all") and a `--since` duration flag ("12h",
//! "2d", bare integer days). No I/O lives here so these are unit-testable
//! without touching stdin or the filesystem.

use anyhow::{anyhow, bail, Result};

/// Parse a 1-based selection string against `max` available items, returning
/// 0-based, sorted, deduplicated indices.
///
/// Accepts:
/// - a single index: `"3"`
/// - a comma-separated list of indices and/or ranges: `"1,3-5"`
/// - `"a"` or `"all"` (case-insensitive): every index `0..max`
///
/// Rejects empty input, out-of-range indices (`< 1` or `> max`), and
/// unparseable tokens with a clear error message.
pub fn parse_selection(input: &str, max: usize) -> Result<Vec<usize>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("no selection entered; expected a number, a range like 1,3-5, or 'all'");
    }
    if trimmed.eq_ignore_ascii_case("a") || trimmed.eq_ignore_ascii_case("all") {
        return Ok((0..max).collect());
    }

    let mut selected = std::collections::BTreeSet::new();
    for token in trimmed.split(',') {
        let token = token.trim();
        if token.is_empty() {
            bail!("empty selection token in '{trimmed}'");
        }
        if let Some((start, end)) = token.split_once('-') {
            let start = parse_one_based(start.trim(), max)?;
            let end = parse_one_based(end.trim(), max)?;
            if start > end {
                bail!("invalid range '{token}': start is after end");
            }
            for n in start..=end {
                selected.insert(n - 1);
            }
        } else {
            let n = parse_one_based(token, max)?;
            selected.insert(n - 1);
        }
    }
    Ok(selected.into_iter().collect())
}

/// Parse and range-check a single 1-based token against `max`.
fn parse_one_based(token: &str, max: usize) -> Result<usize> {
    let n: usize = token
        .parse()
        .map_err(|_| anyhow!("'{token}' is not a valid selection number"))?;
    if n < 1 || n > max {
        bail!("selection '{token}' is out of range (expected 1-{max})");
    }
    Ok(n)
}

/// Parse a `--since` duration: `"<n>h"` (hours), `"<n>d"` (days), or a bare
/// integer treated as days.
pub fn parse_since(s: &str) -> Result<chrono::Duration> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        bail!("empty --since value; expected e.g. '12h', '2d', or '3'");
    }
    if let Some(hours) = trimmed.strip_suffix(['h', 'H']) {
        let n: i64 = hours
            .parse()
            .map_err(|_| anyhow!("'{s}' is not a valid --since duration"))?;
        return Ok(chrono::Duration::hours(n));
    }
    if let Some(days) = trimmed.strip_suffix(['d', 'D']) {
        let n: i64 = days
            .parse()
            .map_err(|_| anyhow!("'{s}' is not a valid --since duration"))?;
        return Ok(chrono::Duration::days(n));
    }
    let n: i64 = trimmed
        .parse()
        .map_err(|_| anyhow!("'{s}' is not a valid --since duration"))?;
    Ok(chrono::Duration::days(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_range_and_all() {
        assert_eq!(parse_selection("3", 5).unwrap(), vec![2]);
        assert_eq!(parse_selection("1,3-5", 5).unwrap(), vec![0, 2, 3, 4]);
        assert_eq!(parse_selection("a", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_selection("all", 3).unwrap(), vec![0, 1, 2]);
        assert!(parse_selection("6", 5).is_err());
        assert!(parse_selection("0", 5).is_err());
        assert!(parse_selection("", 5).is_err());
    }

    #[test]
    fn parses_since() {
        assert_eq!(parse_since("12h").unwrap(), chrono::Duration::hours(12));
        assert_eq!(parse_since("2d").unwrap(), chrono::Duration::days(2));
        assert_eq!(parse_since("3").unwrap(), chrono::Duration::days(3));
        assert!(parse_since("nope").is_err());
    }
}
