//! Bounded, read-only reader for the qualified OpenCode SQLite transcript schema.
//! Only session/message/part data is read; account and credential tables are never queried.
//! SQLite may coordinate readers through an existing WAL shared-memory file.
use super::{SOURCE_OPENCODE, SessionRef, SessionTooLarge, SessionTranscript};
use anyhow::{Result, anyhow, bail};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags, Transaction};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

const BYTE_BUDGET: u64 = 16 * 1024 * 1024;
const RECORD_BUDGET: u64 = 100_000;
const SESSION_BUDGET: usize = 65_536;
const DB: &str = "opencode.db";

pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    env("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
        .join("opencode")
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn accepts_path(root: &Path, path: &Path) -> bool {
    path.parent() == Some(root.join(DB).as_path())
        && path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(valid_id)
}
fn sql<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|_| anyhow!("opencode-database-unreadable"))
}
fn checked_file(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(meta)),
        Ok(_) => bail!("opencode-database-unsafe-path"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("opencode-database-unreadable"),
    }
}
fn sidecars(root: &Path) -> Result<()> {
    let wal = checked_file(&root.join("opencode.db-wal"))?;
    let shm = checked_file(&root.join("opencode.db-shm"))?;
    if wal.is_some_and(|m| m.len() > 0) && shm.is_none() {
        bail!("opencode-database-wal-unavailable");
    }
    Ok(())
}
fn open(root: &Path) -> Result<Option<Connection>> {
    // Like the JSON reader, no claim is made against replacement of ancestor directories.
    let path = root.join(DB);
    let Some(before) = checked_file(&path)? else {
        return Ok(None);
    };
    // SQLite NOFOLLOW rejects symlinked ancestors as well as the final file.
    // Resolve the declared directory (e.g. macOS /var -> /private/var), but never
    // canonicalize the database itself: its final component must stay no-follow.
    let resolved_root = root
        .canonicalize()
        .map_err(|_| anyhow!("opencode-database-unreadable"))?;
    let resolved_path = resolved_root.join(DB);
    sidecars(&resolved_root)?;
    let conn = sql(Connection::open_with_flags(
        &resolved_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ))?;
    let after =
        checked_file(&resolved_path)?.ok_or_else(|| anyhow!("opencode-database-unsafe-path"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            bail!("opencode-database-unsafe-path");
        }
    }
    #[cfg(not(unix))]
    let _ = (before, after);
    sidecars(&resolved_root)?;
    sql(conn.busy_timeout(Duration::from_millis(100)))?;
    // Bound cumulative VM work, including aggregate scans, across this connection.
    // A lock timeout alone does not stop expensive queries on a large database.
    let mut progress_calls = 0_u32;
    conn.progress_handler(
        1000,
        Some(move || {
            progress_calls += 1;
            progress_calls >= 10_000
        }),
    );
    sql(conn.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;"))?;
    Ok(Some(conn))
}
fn schema(tx: &Transaction<'_>) -> Result<()> {
    let version: i64 = sql(tx.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
    if version != 0 {
        bail!("opencode-database-schema-unsupported");
    }
    for (table, columns) in [
        (
            "session",
            &["id", "directory", "version", "time_created", "time_updated"][..],
        ),
        (
            "message",
            &["id", "session_id", "time_created", "time_updated", "data"][..],
        ),
        (
            "part",
            &[
                "id",
                "session_id",
                "message_id",
                "time_created",
                "time_updated",
                "data",
            ][..],
        ),
    ] {
        let kind: String = sql(tx.query_row(
            "SELECT type FROM sqlite_master WHERE name=?1",
            [table],
            |r| r.get(0),
        ))?;
        if kind != "table" {
            bail!("opencode-database-schema-unsupported");
        }
        let mut stmt = sql(tx.prepare(&format!("PRAGMA table_info({table})")))?;
        let rows = sql(stmt.query_map([], |r| r.get::<_, String>(1)))?;
        let names = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        if !columns.iter().all(|c| names.iter().any(|n| n == c)) {
            bail!("opencode-database-schema-unsupported");
        }
    }
    Ok(())
}
fn bounds(tx: &Transaction<'_>, id: &str) -> Result<(u64, u64)> {
    // SQLite evaluates byte lengths without materializing the JSON in Rust. Include
    // identity/metadata and JSON framing overhead conservatively in the byte bound.
    let (bytes, count): (u64,u64) = sql(tx.query_row(
        "SELECT coalesce(sum(n),0), count(*) FROM (
         SELECT length(CAST(data AS BLOB))+length(CAST(id AS BLOB))*2+length(CAST(session_id AS BLOB))+128 AS n FROM message WHERE session_id=?1
         UNION ALL SELECT length(CAST(data AS BLOB))+length(CAST(id AS BLOB))*2+length(CAST(session_id AS BLOB))+length(CAST(message_id AS BLOB))+128 FROM part WHERE session_id=?1
         UNION ALL SELECT length(CAST(directory AS BLOB))+length(CAST(id AS BLOB))+length(CAST(version AS BLOB))+256 FROM session WHERE id=?1)", [id], |r| Ok((r.get(0)?,r.get(1)?))))?;
    Ok((bytes, count.saturating_sub(1)))
}
fn reference(tx: &Transaction<'_>, root: &Path, id: &str) -> Result<SessionRef> {
    let (bytes, _) = bounds(tx, id)?;
    let metadata_bytes: u64 = sql(tx.query_row("SELECT length(CAST(directory AS BLOB))+length(CAST(version AS BLOB)) FROM session WHERE id=?1", [id], |r|r.get(0)))?;
    if metadata_bytes > 65536 {
        bail!("opencode-database-metadata-too-large");
    }
    let (cwd, created, updated): (String, i64, i64) = sql(tx.query_row(
        "SELECT directory,time_created,(SELECT max(t) FROM (SELECT time_updated AS t FROM session WHERE id=?1 UNION ALL SELECT time_updated FROM message WHERE session_id=?1 UNION ALL SELECT time_updated FROM part WHERE session_id=?1)) FROM session WHERE id=?1",
        [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))))?;
    let started_at = DateTime::from_timestamp_millis(created)
        .filter(|_| created >= 0)
        .ok_or_else(|| anyhow!("opencode-database-invalid-timestamp"))?;
    let modified = DateTime::from_timestamp_millis(updated)
        .filter(|_| updated >= created)
        .ok_or_else(|| anyhow!("opencode-database-invalid-timestamp"))?;
    Ok(SessionRef {
        source: SOURCE_OPENCODE,
        declared_source: None,
        path: root.join(DB).join(id),
        project: Path::new(&cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned),
        cwd: Some(cwd),
        started_at: Some(started_at),
        size_bytes: bytes,
        group_modified_at: Some(modified),
        group_member_count: 0,
    })
}
pub fn discover(root: &Path) -> Result<Vec<SessionRef>> {
    let Some(mut conn) = open(root)? else {
        return Ok(Vec::new());
    };
    let tx = sql(conn.transaction())?;
    schema(&tx)?;
    let count: u64 = sql(tx.query_row("SELECT count(*) FROM session", [], |r| r.get(0)))?;
    if count > SESSION_BUDGET as u64 {
        bail!("opencode-database-session-budget");
    }
    let invalid: bool = sql(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session WHERE length(CAST(id AS BLOB))>256 OR id IS NULL)",
        [],
        |r| r.get(0),
    ))?;
    if invalid {
        bail!("opencode-database-invalid-id");
    }
    let mut stmt = sql(tx.prepare("SELECT id FROM session ORDER BY id"))?;
    let ids = sql(stmt.query_map([], |r| r.get::<_, String>(0)))?;
    let mut refs = Vec::new();
    for id in ids {
        let id = sql(id)?;
        if !valid_id(&id) {
            bail!("opencode-database-invalid-id");
        }
        refs.push(reference(&tx, root, &id)?);
    }
    Ok(refs)
}
fn object(data: String, identity: &[(&str, &str)]) -> Result<Value> {
    let mut value: Value =
        serde_json::from_str(&data).map_err(|_| anyhow!("opencode-database-malformed-json"))?;
    let map = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("opencode-database-malformed-json"))?;
    for (key, id) in identity {
        if !valid_id(id) || map.get(*key).is_some_and(|v| v.as_str() != Some(id)) {
            bail!("opencode-database-ambiguous-id");
        }
        map.insert((*key).into(), Value::String((*id).into()));
    }
    Ok(value)
}
pub fn load(root: &Path, reference: &SessionRef) -> Result<SessionTranscript> {
    if reference.source != SOURCE_OPENCODE || !accepts_path(root, &reference.path) {
        bail!("opencode-database-outside-root");
    }
    let id = reference
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("opencode-database-invalid-id"))?;
    let mut conn = open(root)?.ok_or_else(|| anyhow!("opencode-database-unavailable"))?;
    let tx = sql(conn.transaction())?;
    schema(&tx)?;
    let (bytes, count) = bounds(&tx, id)?;
    if count > RECORD_BUDGET {
        bail!("opencode-database-record-budget");
    }
    if bytes > BYTE_BUDGET {
        return Err(SessionTooLarge {
            label: "opencode-database-too-large",
            declared_bytes: bytes,
            budget_bytes: BYTE_BUDGET,
        }
        .into());
    }
    let current = self::reference(&tx, root, id)?;
    let version: String = sql(tx.query_row(
        "SELECT version FROM session WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    if !["1.18.29", "1.19.0"].contains(&version.as_str()) {
        return Err(super::opencode::UnsupportedExportVersion.into());
    }
    let mut messages = Vec::new();
    let mut index = HashMap::new();
    let mut stmt = sql(
        tx.prepare("SELECT id,data FROM message WHERE session_id=?1 ORDER BY time_created,id")
    )?;
    let rows = sql(stmt.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }))?;
    for row in rows {
        let (mid, data) = sql(row)?;
        let info = object(data, &[("id", &mid), ("sessionID", id)])?;
        if index.insert(mid, messages.len()).is_some() {
            bail!("opencode-database-ambiguous-id");
        }
        messages.push(json!({"info":info,"parts":[]}));
    }
    let mut stmt = sql(tx.prepare(
        "SELECT id,message_id,data FROM part WHERE session_id=?1 ORDER BY time_created,id",
    ))?;
    let rows = sql(stmt.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    }))?;
    for row in rows {
        let (pid, mid, data) = sql(row)?;
        let part = object(
            data,
            &[("id", &pid), ("messageID", &mid), ("sessionID", id)],
        )?;
        let slot = index
            .get(&mid)
            .ok_or_else(|| anyhow!("opencode-database-missing-message"))?;
        messages[*slot]["parts"]
            .as_array_mut()
            .expect("constructed array")
            .push(part);
    }
    let export = json!({"info":{"id":id,"version":version,"directory":current.cwd,"time":{"created":current.started_at.map(|t|t.timestamp_millis())}},"messages":messages});
    let bytes =
        serde_json::to_vec(&export).map_err(|_| anyhow!("opencode-database-export-failed"))?;
    super::opencode::parse_export(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let db = Connection::open(dir.path().join(DB)).unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,directory TEXT,version TEXT,time_created INTEGER,time_updated INTEGER);
            CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            CREATE TABLE part(id TEXT PRIMARY KEY,message_id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
            INSERT INTO session VALUES('ses_fixture','/synthetic/project','1.19.0',1000,2000);
            INSERT INTO message VALUES('msg_fixture','ses_fixture',1000,2000,'{\"role\":\"user\",\"time\":{\"created\":1000}}');
            INSERT INTO part VALUES('prt_fixture','msg_fixture','ses_fixture',1000,2000,'{\"type\":\"text\",\"text\":\"Synthetic test\"}');").unwrap();
        dir
    }
    fn change(root: &Path, statement: &str) {
        Connection::open(root.join(DB))
            .unwrap()
            .execute_batch(statement)
            .unwrap();
    }
    #[test]
    fn roundtrip_is_read_only_and_preserves_identity() {
        let dir = fixture();
        let before = std::fs::read(dir.path().join(DB)).unwrap();
        let refs = discover(dir.path()).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].group_member_count, 0);
        assert!(accepts_path(dir.path(), &refs[0].path));
        assert!(!accepts_path(
            dir.path(),
            &dir.path().join("opencode.db/../escape")
        ));
        let actual = load(dir.path(), &refs[0]).unwrap();
        let expected=super::super::opencode::parse_export(br#"{"info":{"id":"ses_fixture","directory":"/synthetic/project","version":"1.19.0","time":{"created":1000}},"messages":[{"info":{"id":"msg_fixture","sessionID":"ses_fixture","role":"user","time":{"created":1000}},"parts":[{"id":"prt_fixture","messageID":"msg_fixture","sessionID":"ses_fixture","type":"text","text":"Synthetic test"}]}]}"#).unwrap();
        assert_eq!(actual.conversation_id, expected.conversation_id);
        assert_eq!(actual.events.len(), 1);
        assert_eq!(actual.agent_version.as_deref(), Some("1.19.0"));
        assert_eq!(before, std::fs::read(dir.path().join(DB)).unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn updates_and_wal_commits_are_visible() {
        let dir = fixture();
        let initial = discover(dir.path()).unwrap().remove(0);
        let writer = Connection::open(dir.path().join(DB)).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; UPDATE part SET time_updated=3000,data='{\"type\":\"text\",\"text\":\"Longer synthetic test update\"}';").unwrap();
        let database_before = std::fs::read(dir.path().join(DB)).unwrap();
        let wal_before = std::fs::read(dir.path().join("opencode.db-wal")).unwrap();
        let updated = discover(dir.path()).unwrap().remove(0);
        assert!(updated.size_bytes > initial.size_bytes);
        assert!(updated.group_modified_at > initial.group_modified_at);
        assert_eq!(load(dir.path(), &updated).unwrap().events.len(), 1);
        assert_eq!(database_before, std::fs::read(dir.path().join(DB)).unwrap());
        assert_eq!(
            wal_before,
            std::fs::read(dir.path().join("opencode.db-wal")).unwrap()
        );
        assert!(
            open(dir.path())
                .unwrap()
                .unwrap()
                .execute("DELETE FROM session", [])
                .is_err()
        );
    }
    #[test]
    fn qualified_native_database_matches_export_events_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let db = Connection::open(dir.path().join(DB)).unwrap();
        db.execute_batch(include_str!(
            "../../tests/fixtures/opencode/native-1.19.0.sql"
        ))
        .unwrap();
        drop(db);
        let reference = discover(dir.path()).unwrap().remove(0);
        let actual = load(dir.path(), &reference).unwrap();
        let mut exported: Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/opencode/completed.json"
        ))
        .unwrap();
        // The real import relocates session.directory, while retaining message paths.
        exported["info"]["directory"] = "/private/tmp/tc-opencode-discovery-probe-ik984gy5".into();
        let expected =
            super::super::opencode::parse_export(&serde_json::to_vec(&exported).unwrap()).unwrap();
        assert_eq!(actual.conversation_id, expected.conversation_id);
        assert_eq!(actual.agent_version, expected.agent_version);
        assert_eq!(actual.source, expected.source);
        assert_eq!(actual.source_session, expected.source_session);
        assert_eq!(actual.model, expected.model);
        assert_eq!(actual.project, expected.project);
        assert_eq!(actual.cwd, expected.cwd);
        assert_eq!(actual.started_at, expected.started_at);
        assert_eq!(actual.events.len(), expected.events.len());
        for (actual, expected) in actual.events.iter().zip(&expected.events) {
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(actual.timestamp, expected.timestamp);
            assert_eq!(actual.content, expected.content);
            assert_eq!(actual.structured, expected.structured);
            assert_eq!(actual.tool_name, expected.tool_name);
            assert_eq!(actual.tool_call_id, expected.tool_call_id);
            assert_eq!(actual.token_counts, expected.token_counts);
            assert_eq!(actual.served_by, expected.served_by);
            assert_eq!(actual.success, expected.success);
        }
        assert!(actual.attested_call.is_none());
        assert!(actual.routing.is_empty());
    }
    #[test]
    fn sqlite_vm_work_budget_stops_expensive_queries() {
        let dir = fixture();
        let conn = open(dir.path()).unwrap().unwrap();
        let result = conn.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0));
        assert_eq!(
            sql(result).unwrap_err().to_string(),
            "opencode-database-unreadable"
        );
    }
    #[test]
    fn imported_older_session_retains_version_and_identity() {
        let dir = fixture();
        change(dir.path(), "UPDATE session SET version='1.18.29'");
        let reference = discover(dir.path()).unwrap().remove(0);
        let transcript = load(dir.path(), &reference).unwrap();
        assert_eq!(transcript.agent_version.as_deref(), Some("1.18.29"));
        assert_eq!(transcript.conversation_id.as_deref(), Some("ses_fixture"));
    }
    #[test]
    fn record_budget_and_ambiguous_identity_fail_closed() {
        let dir = fixture();
        change(
            dir.path(),
            "UPDATE part SET data='{\"id\":\"prt_other\",\"type\":\"text\",\"text\":\"Synthetic\"}'",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode-database-ambiguous-id"
        );
        change(
            dir.path(),
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO part SELECT 'prt_'||x,'msg_fixture','ses_fixture',1000,2000,'{}' FROM n",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode-database-record-budget"
        );
    }
    #[test]
    fn malformed_payload_is_not_read_by_discovery() {
        let dir = fixture();
        change(dir.path(), "UPDATE part SET data='not json'");
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode-database-malformed-json"
        );
    }
    #[test]
    fn refuses_unsupported_and_oversized_inputs() {
        let dir = fixture();
        change(dir.path(), "UPDATE session SET version='99.0.0'");
        let reference = discover(dir.path()).unwrap().remove(0);
        assert!(
            load(dir.path(), &reference)
                .unwrap_err()
                .downcast_ref::<super::super::opencode::UnsupportedExportVersion>()
                .is_some()
        );
        change(
            dir.path(),
            "UPDATE session SET version='1.19.0'; PRAGMA user_version=1",
        );
        assert!(discover(dir.path()).is_err());
        change(
            dir.path(),
            "PRAGMA user_version=0; UPDATE part SET data=zeroblob(16777217)",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert!(
            load(dir.path(), &reference)
                .unwrap_err()
                .downcast_ref::<SessionTooLarge>()
                .is_some()
        );
    }
    #[test]
    fn missing_database_and_conventional_root() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover(dir.path()).unwrap().is_empty());
        let home = dir.path().join("home");
        let data = dir.path().join("data");
        let fallback = home.join(".local/share/opencode");
        assert!(data.is_absolute());
        assert_eq!(conventional_root(&home, |_| None), fallback);
        assert_eq!(
            conventional_root(&home, |_| Some(data.to_string_lossy().into_owned())),
            data.join("opencode")
        );
        assert_eq!(
            conventional_root(&home, |_| Some("relative".into())),
            fallback
        );
    }
    #[cfg(unix)]
    #[test]
    fn refuses_database_and_sidecar_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = fixture();
        let alias = tempfile::tempdir().unwrap();
        let declared_root = alias.path().join("approved-root");
        symlink(dir.path(), &declared_root).unwrap();
        let reference = discover(&declared_root).unwrap().remove(0);
        assert!(reference.path.starts_with(&declared_root));
        assert!(load(&declared_root, &reference).is_ok());
        symlink(dir.path().join(DB), alias.path().join(DB)).unwrap();
        assert!(discover(alias.path()).is_err());
        for name in ["opencode.db-wal", "opencode.db-shm"] {
            symlink(dir.path().join(DB), dir.path().join(name)).unwrap();
            assert!(discover(dir.path()).is_err());
            std::fs::remove_file(dir.path().join(name)).unwrap();
        }
    }
}
