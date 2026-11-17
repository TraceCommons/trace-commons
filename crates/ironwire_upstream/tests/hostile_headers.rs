//! Response headers are upstream-controlled input, and two of them reach
//! arithmetic that panics rather than saturates.
//!
//! `retry-after` is the dangerous one: every provider sends it, it feeds
//! `now + Duration::seconds(..)`, and `chrono` panics on an out-of-range
//! duration. An unclamped header was therefore a remotely-triggerable crash —
//! and a crash in the daemon takes down every conversation on the machine, not
//! just the request that received the header.
//!
//! The percentage headers are the quieter one. `parse::<f32>` accepts `"NaN"`,
//! a NaN survives `clamp` (it propagates), and `used_pct >= 90.0` is then false
//! forever — so a backend would look permanently healthy while `ironwire
//! status` printed "NaN% used".

use chrono::Utc;
use ironwire_upstream::observe::{MAX_RETRY_AFTER_SECS, anthropic_rate_limit, retry_after};
use ironwire_upstream::openai_responses::chatgpt_rate_limit;

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn a_huge_retry_after_is_clamped_rather_than_overflowing() {
    let now = Utc::now();
    for value in [
        "18446744073709551615", // u64::MAX
        "99999999999999999",
        "9223372036854775807", // i64::MAX
    ] {
        let secs = retry_after(&headers(&[("retry-after", value)]), now)
            .expect("a numeric retry-after parses");
        assert!(
            secs <= MAX_RETRY_AFTER_SECS,
            "{value} came back as {secs}, which overflows the Duration it feeds"
        );
        // The arithmetic it exists to protect must now be safe.
        let _ = now + chrono::Duration::seconds(i64::try_from(secs).expect("fits"));
    }
}

#[test]
fn an_ordinary_retry_after_is_untouched() {
    let now = Utc::now();
    assert_eq!(
        retry_after(&headers(&[("retry-after", "30")]), now),
        Some(30)
    );
    assert_eq!(
        retry_after(&headers(&[("retry-after", "3600")]), now),
        Some(3600)
    );
}

#[test]
fn a_retry_after_date_far_in_the_future_is_clamped_too() {
    let now = Utc::now();
    let secs = retry_after(
        &headers(&[("retry-after", "Fri, 31 Dec 9999 23:59:59 +0000")]),
        now,
    );
    assert!(
        secs.is_some_and(|s| s <= MAX_RETRY_AFTER_SECS),
        "the HTTP-date form bypassed the clamp: {secs:?}"
    );
}

#[test]
fn a_retry_after_in_the_past_is_ignored_rather_than_wrapping() {
    let now = Utc::now();
    assert_eq!(
        retry_after(
            &headers(&[("retry-after", "Mon, 01 Jan 2001 00:00:00 +0000")]),
            now
        ),
        None,
        "a past date must not become a huge unsigned wait"
    );
}

#[test]
fn a_non_finite_percentage_reads_as_unknown_not_as_healthy() {
    // The failure mode: NaN compares false against every threshold, so the
    // backend never looks pressured and never descends the ladder.
    for value in ["NaN", "nan", "inf", "-inf", "Infinity"] {
        let reading = anthropic_rate_limit(&headers(&[(
            "anthropic-ratelimit-unified-used-percent",
            value,
        )]));
        assert!(
            reading.is_none_or(|r| r.used_pct.is_finite()),
            "{value} produced a non-finite used_pct"
        );
    }
}

#[test]
fn a_non_finite_derived_percentage_reads_as_unknown() {
    // The remaining/limit path has the same exposure.
    for (remaining, limit) in [("NaN", "1000"), ("100", "NaN"), ("inf", "inf")] {
        let reading = anthropic_rate_limit(&headers(&[
            ("anthropic-ratelimit-unified-remaining", remaining),
            ("anthropic-ratelimit-unified-limit", limit),
        ]));
        assert!(
            reading.is_none_or(|r| r.used_pct.is_finite()),
            "remaining={remaining} limit={limit} produced a non-finite used_pct"
        );
    }
}

#[test]
fn the_chatgpt_windows_reject_non_finite_values_too() {
    for value in ["NaN", "inf"] {
        let reading = chatgpt_rate_limit(
            &headers(&[("x-codex-primary-used-percent", value)]),
            "primary",
        );
        assert!(
            reading.is_none_or(|r| r.used_pct.is_finite()),
            "{value} produced a non-finite used_pct"
        );
    }
}

#[test]
fn a_huge_chatgpt_reset_window_does_not_overflow() {
    // Same arithmetic, different header.
    let reading = chatgpt_rate_limit(
        &headers(&[
            ("x-codex-primary-used-percent", "50"),
            ("x-codex-primary-reset-after-seconds", "9223372036854775807"),
        ]),
        "primary",
    );
    assert!(reading.is_some(), "the reading was dropped entirely");
}

#[test]
fn ordinary_percentages_still_work() {
    // The clamps must not have broken the normal path.
    let reading = anthropic_rate_limit(&headers(&[(
        "anthropic-ratelimit-unified-used-percent",
        "82",
    )]))
    .expect("a plain percentage reads");
    assert!((reading.used_pct - 82.0).abs() < 0.01);

    let derived = anthropic_rate_limit(&headers(&[
        ("anthropic-ratelimit-unified-remaining", "250"),
        ("anthropic-ratelimit-unified-limit", "1000"),
    ]))
    .expect("remaining/limit reads");
    assert!((derived.used_pct - 75.0).abs() < 0.01);
}

#[test]
fn garbage_headers_produce_no_reading_rather_than_a_guess() {
    for value in ["", "  ", "eighty", "82%", "\0"] {
        let reading = anthropic_rate_limit(&headers(&[(
            "anthropic-ratelimit-unified-used-percent",
            value,
        )]));
        assert!(reading.is_none(), "{value:?} was read as a percentage");
    }
}
