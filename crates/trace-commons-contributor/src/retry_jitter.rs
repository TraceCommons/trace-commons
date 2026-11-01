//! Randomised waits for retries against the commons, and the standard
//! `Retry-After` header.
//!
//! After an ingest restart or an overload every contributor's daemon is
//! refused in the same few seconds. Fixed waits (1 s, then 4 s; a minute
//! doubling; the server's own "2 seconds") bring them all back together, so
//! the next wave arrives as one herd. Two shapes of spread are used:
//!
//! - **Our own backoff** (the in-call transport retries and the daemon's
//!   cross-pass outage backoff) uses *equal jitter*: half the scheduled wait
//!   is kept and a random amount up to the other half is added, so the wait
//!   falls in `[base / 2, base]`. Full jitter (`[0, base]`) spreads a little
//!   more but can come back almost at once, which defeats a backoff whose
//!   point is to leave a recovering server alone; equal jitter keeps the
//!   doubling meaningful and the caps unchanged (a jittered wait never
//!   exceeds the unjittered one).
//! - **A wait the server asked for** (a `Retry-After` hint) is a floor, never
//!   shortened: the jittered wait is the hint plus a bounded random extra
//!   (`hint / 5`, at least 5 s, at most 30 s), still inside the same overall
//!   cap, so clients all told "2 s" do not return in lockstep.
//!
//! Every function takes the random value as an argument so tests can sweep
//! it; [`random_u64`] draws it from the operating system's generator.

use chrono::{DateTime, Utc};

/// The smallest random extra added to a server-asked wait, in seconds.
pub const HINT_EXTRA_MIN_SECS: u32 = 5;
/// The largest random extra added to a server-asked wait, in seconds.
pub const HINT_EXTRA_MAX_SECS: u32 = 30;

/// A random `u64` from the operating system's generator (`ring`, already a
/// dependency). If the generator fails, falls back to the time's
/// sub-second nanoseconds mixed with the process id: a worse spread, but
/// retrying must never fail on account of the spread.
#[must_use]
pub fn random_u64() -> u64 {
    use ring::rand::SecureRandom;
    let mut bytes = [0_u8; 8];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_ok() {
        return u64::from_le_bytes(bytes);
    }
    let nanos = u64::from(Utc::now().timestamp_subsec_nanos());
    nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(std::process::id())
}

/// Equal jitter over `base_ms`: a wait in `[base_ms / 2, base_ms]`.
#[must_use]
pub fn equal_jitter_millis(base_ms: u64, random: u64) -> u64 {
    let half = base_ms / 2;
    half + random % (base_ms - half + 1)
}

/// Equal jitter over a whole number of seconds, in seconds.
#[must_use]
pub fn equal_jitter_secs(base_secs: u64, random: u64) -> u64 {
    equal_jitter_millis(base_secs.saturating_mul(1000), random) / 1000
}

/// The wait for a server-asked delay: `hint_secs` plus a random extra, never
/// below the hint and never above `cap_secs` (a hint already at or above the
/// cap is returned clamped to it, unjittered).
#[must_use]
pub fn hint_with_extra_secs(hint_secs: u32, cap_secs: u32, random: u64) -> u32 {
    let hint = hint_secs.min(cap_secs);
    let extra_max = (hint / 5).clamp(HINT_EXTRA_MIN_SECS, HINT_EXTRA_MAX_SECS);
    let extra = (random % (u64::from(extra_max) + 1)) as u32;
    hint.saturating_add(extra).min(cap_secs)
}

/// Seconds to wait from a `Retry-After` header value: delta-seconds
/// (`120`) or an HTTP date (`Wed, 21 Oct 2026 07:28:00 GMT`), measured from
/// `now`. `None` for anything else. A date in the past is `0`.
#[must_use]
pub fn parse_retry_after_header(value: &str, now: DateTime<Utc>) -> Option<u32> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        let seconds = value.parse::<u64>().unwrap_or(u64::MAX);
        return Some(u32::try_from(seconds).unwrap_or(u32::MAX));
    }
    let date = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    let secs = (date - now).num_seconds().max(0);
    Some(u32::try_from(secs).unwrap_or(u32::MAX))
}

/// The server's wait in seconds: the body's `retry_after_seconds` when it has
/// one, otherwise the standard header. Unclamped; the caller applies its caps.
#[must_use]
pub fn server_retry_after(
    body_seconds: Option<u64>,
    header: Option<&str>,
    now: DateTime<Utc>,
) -> Option<u32> {
    body_seconds
        .map(|seconds| u32::try_from(seconds).unwrap_or(u32::MAX))
        .or_else(|| header.and_then(|value| parse_retry_after_header(value, now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<u64> {
        let mut v: Vec<u64> = (0..2000).collect();
        v.extend([u64::MAX, u64::MAX - 1, 1 << 63, 0xDEAD_BEEF_CAFE_F00D]);
        v
    }

    #[test]
    fn equal_jitter_stays_between_half_and_the_whole_base() {
        for base in [0_u64, 1, 2, 999, 1000, 4000, 3_600_000] {
            let (mut lo, mut hi) = (u64::MAX, 0);
            for r in samples() {
                let w = equal_jitter_millis(base, r);
                assert!(w >= base / 2 && w <= base, "base {base} gave {w}");
                lo = lo.min(w);
                hi = hi.max(w);
            }
            if base >= 1000 {
                assert!(hi > lo, "base {base} did not spread");
            }
        }
        for base in [1_u64, 60, 3600] {
            for r in samples() {
                let w = equal_jitter_secs(base, r);
                assert!(w >= base / 2 && w <= base, "{base} -> {w}");
            }
        }
    }

    #[test]
    fn a_server_hint_is_a_floor_and_the_cap_still_holds() {
        for hint in [1_u32, 2, 17, 60, 600, 899, 900] {
            let mut distinct = std::collections::BTreeSet::new();
            for r in samples() {
                let w = hint_with_extra_secs(hint, 900, r);
                assert!(w >= hint, "hint {hint} came back as {w}");
                assert!(w <= 900, "hint {hint} passed the cap: {w}");
                assert!(
                    w <= hint + (hint / 5).clamp(HINT_EXTRA_MIN_SECS, HINT_EXTRA_MAX_SECS),
                    "hint {hint} extra too large: {w}"
                );
                distinct.insert(w);
            }
            if hint < 900 {
                assert!(distinct.len() > 1, "hint {hint} did not spread");
            }
        }
        // A hint over the cap is clamped to it, never exceeded.
        assert_eq!(hint_with_extra_secs(86_400, 900, 12345), 900);
        // A two-second hint spreads over several seconds, not 20% of two.
        let spread: std::collections::BTreeSet<u32> = samples()
            .into_iter()
            .map(|r| hint_with_extra_secs(2, 900, r))
            .collect();
        assert!(spread.len() >= 5, "{spread:?}");
    }

    #[test]
    fn the_os_generator_gives_varying_values() {
        let values: std::collections::BTreeSet<u64> = (0..16).map(|_| random_u64()).collect();
        assert!(values.len() > 1);
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn retry_after_header_parses_delta_seconds_and_http_dates() {
        let now = at("2026-10-21T07:27:00Z");
        assert_eq!(parse_retry_after_header("120", now), Some(120));
        assert_eq!(parse_retry_after_header(" 7 ", now), Some(7));
        assert_eq!(parse_retry_after_header("0", now), Some(0));
        assert_eq!(
            parse_retry_after_header("Wed, 21 Oct 2026 07:28:00 GMT", now),
            Some(60)
        );
        assert_eq!(
            parse_retry_after_header("Wed, 21 Oct 2026 07:00:00 GMT", now),
            Some(0),
            "a date in the past is no wait"
        );
        for bad in ["", "soon", "-5", "1.5", "12abc"] {
            assert_eq!(parse_retry_after_header(bad, now), None, "{bad:?}");
        }
        assert_eq!(
            parse_retry_after_header("99999999999999999999", now),
            Some(u32::MAX)
        );
    }

    #[test]
    fn the_body_field_wins_over_the_header() {
        let now = at("2026-10-21T07:27:00Z");
        assert_eq!(server_retry_after(Some(17), Some("90"), now), Some(17));
        assert_eq!(server_retry_after(Some(0), Some("90"), now), Some(0));
        assert_eq!(server_retry_after(None, Some("90"), now), Some(90));
        assert_eq!(server_retry_after(None, Some("junk"), now), None);
        assert_eq!(server_retry_after(None, None, now), None);
    }
}
