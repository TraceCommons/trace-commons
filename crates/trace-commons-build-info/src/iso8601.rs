// Formats seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
//
// This file is compiled twice on purpose, so the comments here are ordinary
// line comments rather than inner doc comments: `build.rs` `include!`s it,
// because that is where the timestamp is resolved, and the library declares it
// as a test-only module so the arithmetic is covered by unit tests instead of
// only by whatever date the build happened on.

/// Format seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Hand-rolled rather than pulled from chrono: a build-dependency here is paid
/// for on every clean build of the workspace, and this is the entirety of what
/// that dependency would be used for. The date arithmetic is Howard Hinnant's
/// civil-from-days, which is exact across the proleptic Gregorian calendar.
pub fn format_iso8601_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}
