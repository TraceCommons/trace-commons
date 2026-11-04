## What this changes

The native session rotation cap was computed in the database session's time zone. `crates/trace-commons-server/src/db/postgres.rs` (the rotation `UPDATE` in `validate_session`) capped the sliding expiry at `created_at + make_interval(days => 30)`. Adding a *days* interval to a `timestamptz` counts calendar days in the session's `TimeZone`, so in a zone that changes its clocks inside the 30-day window the cap lands an hour off: an hour late after a fall-back, an hour early after a spring-forward. The cap is meant to be 30 x 24 hours (`NATIVE_SESSION_MAX_LIFETIME_DAYS`, `account_native_auth.rs`), and the test computes it that way with `chrono::Duration::days(30)`.

The fix binds the cap in hours (`make_interval(hours => 30 * 24)`), which means the same instant in every zone. This is the same fix V143 applied to the referral earning promise. Under UTC the value is unchanged.

**The test was right. The bug was in production code.** `pg_native_rotation_stops_at_the_absolute_cap` started failing locally on 2026-11-01, when US daylight time ended, because Homebrew PostgreSQL takes the host's zone (America/Los_Angeles).

### Why two agents disagreed

The process `TZ` has no effect. tokio-postgres doesn't read `TZ` and sends no `TimeZone` unless the connection string sets `options`, so each session uses the database's default zone. Only the database zone matters, and only while a clock change falls inside the 30 days before the cap.

### Reproduction (2026-11-03, PostgreSQL 16.15, one fresh database per zone)

"Database zone" is set with `ALTER DATABASE ... SET timezone`. "Process TZ" is the test process's environment. Each cell covers that test across both process settings, unset and `TZ=UTC`.

| binary | test | database zone | process TZ unset | process TZ=UTC |
|---|---|---|---|---|
| before | `pg_native_rotation_stops_at_the_absolute_cap` | America/Los_Angeles | FAIL (cap +1h) | FAIL (cap +1h) |
| before | `pg_native_rotation_stops_at_the_absolute_cap` | UTC | pass | pass |
| before | new `pg_native_rotation_cap_ignores_the_database_time_zone` | America/Los_Angeles | FAIL (+1h) | FAIL (+1h) |
| before | new `pg_native_rotation_cap_ignores_the_database_time_zone` | UTC | FAIL (+1h) | FAIL (+1h) |
| after | both | America/Los_Angeles | pass | pass |
| after | both | UTC | pass | pass |

A failure from the old code looks like `left: 2026-11-03T07:26:42.166641Z  right: 2026-11-03T06:26:42.166641Z`.

### Regression test

`pg_native_rotation_cap_ignores_the_database_time_zone` connects with `options=-c TimeZone=<zone>`, where the zone is a POSIX rule computed on each run so that daylight time ended ten days ago. It therefore fails on the old code in any month and on a UTC CI database, not just in the 30 days after a real clock change. A control step first asserts that, in that session, `created_at + 30 days` and `created_at + 720 hours` differ by exactly one hour, so the test cannot pass because the zone had no clock change.

## Production impact

**Probably none on the deployed services.** I didn't connect to production. From the repo:

- The pilot uses either Cloud SQL through `deploy/pilot-gcp/systemd/cloud-sql-proxy.service`, whose default `timezone` flag is UTC, or `postgresql@16-main` on an Ubuntu GCE image, which `initdb` sets to the host zone, Etc/UTC. Staging (`deploy/staging-gcp`) uses the same two modes. Nothing under `deploy/` or `docs/operator/` sets a database `timezone` flag or a `TZ` in a unit.
- V143's header states the service's databases use UTC.

My confidence is high but this is inferred from the repo, not observed. Without this fix, a non-UTC database would cap the final extension of a native session created in the 30 days before a clock change an hour late (fall-back) or an hour early (spring-forward). The error is at most one hour on a 30-day cap and never touches the 12-hour sliding lifetime.

### Same pattern elsewhere, left unchanged

These are rolling windows, not stored caps compared against a Rust value, so an hour's drift near a clock change only blurs the window's edge:

- `postgres_referral_bonus.rs:430,452`: `last_seen_at >= / < clock_timestamp() - make_interval(days => $n)` (activity window)
- `postgres_referral.rs:725`: the same shape
- `V114__external_account_trust_evaluations.sql:418,425,428`: `transaction_timestamp() - interval '7 days'` (retention sweep)

There is no `chrono::Local` or `Local::now` in the server or ingest crates.

## Verification

Run on the merged head f19f35688 (origin/main merged in with a merge commit):

- [x] `cargo fmt --all -- --check`: clean
- [x] `RUSTFLAGS="-D warnings" cargo test -p trace-commons-ingest --lib --no-run`: builds
- [x] `cargo clippy -p trace-commons-server -p trace-commons-ingest --all-targets` with the repo allow-list: no warnings
- [x] `native_session_sliding_expiry_tests` module, on fresh `admission_test_*` databases at 127.0.0.1 with the login-resolver URL and `--test-threads=1`: **11 passed, 0 failed** in each of the four combinations (database America/Los_Angeles or UTC, process `TZ=America/Los_Angeles` or `TZ=UTC`)
- [ ] I have not run the whole `-p trace-commons-server -p trace-commons-ingest` suite locally yet. CI runs it on this head.

## Licensing

- [x] I license this contribution under **MIT OR Apache-2.0**, per
      [CONTRIBUTING.md](../CONTRIBUTING.md) — including for the AGPL-licensed
      server crates, which are distributed under AGPL-3.0-or-later regardless.
- [x] Any new `.rs` file in `trace-commons-server`, `trace-commons-ingest`,
      `trace-commons-gate-api`, or `trace-commons-gate-enclave` carries the
      copyright + SPDX header. (No new files.)
- [x] No permissive crate gained a dependency on an AGPL crate.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
