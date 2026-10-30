//! Codex `/goal` goals, read from `goals_1.sqlite` in Codex's home
//! directory for the Insights week answer (product owner, 2026-10-29).
//!
//! The database is opened read-only, never created and never written. Only
//! the non-text columns are selected: the goal's `objective` (and its
//! `goal_id`) are never read. Each goal is named by the keyed digest of its
//! thread ID under the counter key, the digest the counter rows hold for
//! their Codex session, so the raw thread ID is dropped as soon as it is
//! digested. No database, no table, or a database that cannot be read is
//! no goals: Insights shows none rather than an error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use trace_commons_protocol::insights_usage_series::{
    DigestKey, KeyedDigest, harness_session_digest,
};

use crate::insights::slash_goals::GoalStatus;
use crate::source::SOURCE_CODEX;

/// Codex's goals database, in its home directory.
pub const CODEX_GOALS_FILE: &str = "goals_1.sqlite";
/// A bound on what is read: one goal per thread, so far more than a person
/// sets in the weeks the store keeps.
const MAX_GOALS: usize = 4_096;
const MAX_DB_BYTES: u64 = 64 * 1024 * 1024;
const MAX_THREAD_ID_BYTES: usize = 128;

/// One Codex goal: status and counters only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexGoal {
    pub status: GoalStatus,
    pub tokens_used: Option<u64>,
    pub time_used_secs: Option<u64>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Codex's home: `CODEX_HOME` when set (non-empty and absolute), else
/// `~/.codex`, as the skill installer resolves it.
pub(crate) fn codex_goals_path() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))?;
    home.is_absolute().then(|| home.join(CODEX_GOALS_FILE))
}

/// The goals at `path`, by the keyed digest of each one's thread ID.
/// Empty when there is no database or it cannot be read.
pub(crate) fn read_codex_goals(path: &Path, key: &DigestKey) -> BTreeMap<KeyedDigest, CodexGoal> {
    read(path, key).unwrap_or_default()
}

/// Codex keeps the database in WAL mode. While Codex runs, the `-wal` and
/// `-shm` files beside it exist and a read-only connection reads through
/// them. When Codex has closed it, both are gone (or the log is empty) and
/// a read-only connection cannot make the `-shm` it needs, so the file is
/// opened `immutable`: with no log, every change is already in the main
/// file. `immutable` is never used beside a log with content, which it
/// would ignore.
///
/// `NOFOLLOW` refuses a symlinked ancestor too (macOS shows its temporary
/// directory through `/var`), so the directory is resolved first; the file
/// itself, already refused above when it is a link, keeps `NOFOLLOW`.
fn open_read_only(path: &Path) -> Option<Connection> {
    let dir = std::fs::canonicalize(path.parent()?).ok()?;
    let path = &dir.join(path.file_name()?);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let logged = match std::fs::symlink_metadata(PathBuf::from(wal)) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => return None,
        Ok(meta) => meta.len() > 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return None,
    };
    if logged {
        return Connection::open_with_flags(path, flags).ok();
    }
    let text = path.to_str()?;
    let escaped = text
        .replace('%', "%25")
        .replace('?', "%3f")
        .replace('#', "%23");
    Connection::open_with_flags(
        format!("file:{escaped}?immutable=1"),
        flags | OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()
}

fn read(path: &Path, key: &DigestKey) -> Option<BTreeMap<KeyedDigest, CodexGoal>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_DB_BYTES {
        return None;
    }
    let conn = open_read_only(path)?;
    conn.busy_timeout(Duration::from_millis(50)).ok()?;
    // Never `objective` or `goal_id`: the text stays in Codex's file.
    let mut stmt = conn
        .prepare(
            "SELECT thread_id, status, tokens_used, time_used_seconds, updated_at_ms \
             FROM thread_goals LIMIT ?1",
        )
        .ok()?;
    let mut rows = stmt.query([MAX_GOALS as i64]).ok()?;
    let mut goals = BTreeMap::new();
    while let Some(row) = rows.next().ok()? {
        let Ok(rusqlite::types::ValueRef::Text(thread)) = row.get_ref(0) else {
            continue;
        };
        if thread.len() > MAX_THREAD_ID_BYTES {
            continue;
        }
        let Ok(thread) = std::str::from_utf8(thread) else {
            continue;
        };
        let status = match row.get_ref(1) {
            Ok(rusqlite::types::ValueRef::Text(status)) if status.len() <= 32 => {
                GoalStatus::from_codex(std::str::from_utf8(status).unwrap_or(""))
            }
            _ => GoalStatus::Unknown,
        };
        let count = |i| {
            row.get::<_, Option<i64>>(i)
                .ok()
                .flatten()
                .and_then(|n| u64::try_from(n).ok())
        };
        let updated_at = row
            .get::<_, Option<i64>>(4)
            .ok()
            .flatten()
            .and_then(DateTime::<Utc>::from_timestamp_millis);
        goals.insert(
            harness_session_digest(key, SOURCE_CODEX, thread),
            CodexGoal {
                status,
                tokens_used: count(2),
                time_used_secs: count(3),
                updated_at,
            },
        );
    }
    Some(goals)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const OBJECTIVE: &str = "SECRET-CODEX-OBJECTIVE-TEXT";

    /// A `goals_1.sqlite` in WAL mode, as Codex writes it, with one goal
    /// per `(thread_id, status, tokens, secs, updated_at_ms)`.
    pub(crate) fn write_goals_db(path: &Path, goals: &[(&str, &str, i64, i64, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", "wal").unwrap();
        conn.execute_batch(
            "CREATE TABLE thread_goals(thread_id TEXT PRIMARY KEY NOT NULL, goal_id TEXT NOT NULL, \
             objective TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('active','paused',\
             'blocked','usage_limited','budget_limited','complete')), token_budget INTEGER, \
             tokens_used INTEGER NOT NULL DEFAULT 0, time_used_seconds INTEGER NOT NULL DEFAULT 0, \
             created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);",
        )
        .unwrap();
        for (thread, status, tokens, secs, updated) in goals {
            conn.execute(
                "INSERT INTO thread_goals VALUES (?1, 'goal-id', ?2, ?3, NULL, ?4, ?5, ?6, ?6)",
                rusqlite::params![thread, OBJECTIVE, status, tokens, secs, updated],
            )
            .unwrap();
        }
    }

    fn key() -> DigestKey {
        DigestKey::from_bytes([7; 32])
    }

    #[test]
    fn goals_are_read_by_thread_digest_without_their_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CODEX_GOALS_FILE);
        write_goals_db(
            &path,
            &[
                ("019e-a", "complete", 900, 60, 1_761_700_000_000),
                ("019e-b", "paused", 10, 5, 1_761_700_100_000),
            ],
        );
        let before = std::fs::read_dir(dir.path()).unwrap().count();
        let goals = read_codex_goals(&path, &key());
        assert_eq!(goals.len(), 2);
        let a = goals[&harness_session_digest(&key(), SOURCE_CODEX, "019e-a")];
        assert_eq!(a.status, GoalStatus::Complete);
        assert_eq!(a.tokens_used, Some(900));
        assert_eq!(a.time_used_secs, Some(60));
        assert_eq!(a.updated_at.unwrap().timestamp_millis(), 1_761_700_000_000);
        assert!(!format!("{goals:?}").contains("SECRET"));
        // Read-only: nothing is created beside the database.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
    }

    /// While Codex runs, its changes sit in the log beside the file: they
    /// are read through it, never missed.
    #[test]
    fn goals_still_in_the_write_ahead_log_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CODEX_GOALS_FILE);
        write_goals_db(&path, &[("019e-a", "active", 1, 1, 1)]);
        let codex = Connection::open(&path).unwrap();
        codex.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        codex
            .execute(
                "UPDATE thread_goals SET status = 'complete', tokens_used = 77 WHERE thread_id = '019e-a'",
                [],
            )
            .unwrap();
        let wal = dir.path().join(format!("{CODEX_GOALS_FILE}-wal"));
        assert!(
            std::fs::metadata(&wal).unwrap().len() > 0,
            "the change is in the log"
        );
        let goals = read_codex_goals(&path, &key());
        let a = goals[&harness_session_digest(&key(), SOURCE_CODEX, "019e-a")];
        assert_eq!(a.status, GoalStatus::Complete);
        assert_eq!(a.tokens_used, Some(77));
        drop(codex);
    }

    #[test]
    fn no_database_or_no_table_is_no_goals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CODEX_GOALS_FILE);
        assert!(read_codex_goals(&path, &key()).is_empty());
        assert!(!path.exists(), "a missing database is never created");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE other(x INTEGER);")
            .unwrap();
        assert!(read_codex_goals(&path, &key()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.sqlite");
        write_goals_db(&real, &[("t", "complete", 1, 1, 1)]);
        let link = dir.path().join(CODEX_GOALS_FILE);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_codex_goals(&link, &key()).is_empty());
    }
}
