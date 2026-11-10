// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ingest service tested against the contributor's own client code.
//!
//! These tests lived in the `trace-commons-ingest` crate's unit-test module.
//! They are the only ingest tests that use `trace-commons-contributor`, and
//! keeping them there made the contributor, and the 84 packages only it
//! brings, a dependency of every build of the ingest test harness. Here they
//! build against the ingest library's `test-support` fixtures instead.
//!
//! Every test in this package needs a PostgreSQL database and returns early,
//! or is `#[ignore]`d, without one. They share the `tenant-a` fixture tenant
//! and process-global state with each other, so run them on one thread:
//!
//! ```text
//! cargo test -p trace-commons-ingest-contributor-tests --test it -- --test-threads=1
//! ```

mod near_ai_provisioning_tests;
// The module name is load-bearing. The `postgres-suites` CI job selects this
// suite with `cargo test -p trace-commons-ingest-contributor-tests --test it
// nearai_ceremony_pg_tests -- --ignored`, a substring match on the test
// path. A rename that does not change the workflow leaves these tests
// reporting `ignored` forever, which is the failure mode that step exists
// to prevent.
mod nearai_ceremony_pg_tests;
mod token_bundle_journey_test;
