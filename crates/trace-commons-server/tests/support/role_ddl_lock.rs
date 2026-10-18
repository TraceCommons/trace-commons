// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The advisory lock every PostgreSQL suite takes before test-fixture role
//! DDL, shared by each suite that includes this file with `#[path]`, so they
//! hold one key without depending on each other.
//!
//! `GRANT` and `REVOKE` rewrite the ACL on the object's catalog row --
//! `pg_class.relacl` for a table or sequence, `pg_namespace.nspacl` for a
//! schema, `pg_proc.proacl` for a function -- and `ALTER ROLE` rewrites the
//! role's `pg_authid` row. libtest runs a suite's tests in parallel against
//! one database, so two of those in flight on the same object update the same
//! catalog tuple and PostgreSQL raises `XX000 tuple concurrently updated`. Two
//! `CREATE ROLE`s behind `IF NOT EXISTS` race the same way, into a duplicate
//! key on `pg_authid`. It surfaces in whichever test lost, on whatever PR
//! happened to be running: run 34381118381 against #825 (`trace_corpus_pg_rls`)
//! and #1430 (`trace_session_withdrawal_pg`), both changes that touched none of
//! it.
//!
//! The contended tuple belongs to the object, not to the grantee: two
//! concurrent `GRANT ... ON ALL TABLES` to two DIFFERENT roles raise it just as
//! two to the same role do, so a role per test does not help. Serialising the
//! DDL does, following the migration runner's advisory-lock precedent.

/// Two-int form with its own classid, so it cannot alias the one-arg
/// `pg_advisory_xact_lock(hashtext(tenant))` the audit-chain append takes.
const ROLE_DDL_LOCK_CLASSID: i32 = 0x726f_6c65u32 as i32; // "role"
const ROLE_DDL_LOCK_OBJID: i32 = 0x7273_6c73u32 as i32; // "rsls"

/// `ddl` with the role-DDL advisory lock taken as its first statement, for
/// `batch_execute`.
///
/// A multi-statement `batch_execute` runs as one implicit transaction, so the
/// `_xact_` lock is held across every statement and released when the batch
/// commits; no path can leak it. A batch that ends in `BEGIN` holds it until
/// that explicit transaction ends, because PostgreSQL folds the statements
/// before a `BEGIN` into the transaction it opens. A `COMMIT` partway through
/// would release it early, so `ddl` must not contain one.
///
/// Advisory locks are per database: this serialises the tests of one suite,
/// and suites pointed at one database together, which is where the race is.
pub fn with_role_ddl_lock(ddl: &str) -> String {
    format!("SELECT pg_advisory_xact_lock({ROLE_DDL_LOCK_CLASSID}, {ROLE_DDL_LOCK_OBJID});\n{ddl}")
}
