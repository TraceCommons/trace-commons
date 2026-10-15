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
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, SystemTime},
};

const BYTE_BUDGET: u64 = 16 * 1024 * 1024;
const RECORD_BUDGET: u64 = 100_000;
const SESSION_BUDGET: usize = 65_536;
const DB: &str = "opencode.db";
/// Progress callbacks (one per 1000 VM instructions) allowed per session.
const WORK_BUDGET: u32 = 10_000;

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
/// Whether either sidecar exists. Errors on a non-file sidecar, and on a
/// non-empty WAL without its shared-memory file.
fn sidecars(root: &Path) -> Result<bool> {
    let wal = checked_file(&root.join("opencode.db-wal"))?;
    let shm = checked_file(&root.join("opencode.db-shm"))?;
    if wal.as_ref().is_some_and(|m| m.len() > 0) && shm.is_none() {
        bail!("opencode-database-wal-unavailable");
    }
    Ok(wal.is_some() || shm.is_some())
}
/// `path` as an SQLite URI opened `immutable=1`; None where the path has no
/// plain URI form (a Windows UNC share), which then opens normally.
fn immutable_uri(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    #[cfg(windows)]
    let text = {
        let text = text.strip_prefix(r"\\?\").unwrap_or(text);
        if text.starts_with(r"\\") || text.starts_with("UNC\\") {
            return None;
        }
        format!("/{}", text.replace('\\', "/"))
    };
    let mut uri = String::from("file:");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/:-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?immutable=1");
    Some(uri)
}
/// An open, read-only OpenCode database.
struct Reader {
    conn: Connection,
    /// Progress callbacks used by the current session's queries.
    work: Arc<AtomicU32>,
    /// For an `immutable` open: the database file's size and modification
    /// time when opened, which must be unchanged once reading is done.
    snapshot: Option<(PathBuf, u64, SystemTime)>,
}
impl Reader {
    /// An `immutable` read assumed nobody changed the file underneath it. A
    /// checkpoint by an OpenCode that started meanwhile would, so the read
    /// is refused and retried on a later pass.
    fn finish(self) -> Result<()> {
        drop(self.conn);
        if let Some((path, len, modified)) = self.snapshot {
            let now = checked_file(&path)?.ok_or_else(|| anyhow!("opencode-database-busy"))?;
            if now.len() != len || now.modified().ok() != Some(modified) {
                bail!("opencode-database-busy");
            }
        }
        Ok(())
    }
}
fn open(root: &Path) -> Result<Option<Reader>> {
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
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    // With no sidecars the database is fully checkpointed and nobody has it
    // open in WAL mode. A plain read-only open of a WAL database would then
    // create `-wal` and `-shm` in the person's folder and leave them there;
    // `immutable` creates nothing and takes no locks, and `finish` refuses
    // the read if the file changed meanwhile.
    let immutable = if sidecars(&resolved_root)? {
        None
    } else {
        immutable_uri(&resolved_path)
    };
    let conn = sql(match &immutable {
        Some(uri) => Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI),
        None => Connection::open_with_flags(&resolved_path, flags),
    })?;
    let after =
        checked_file(&resolved_path)?.ok_or_else(|| anyhow!("opencode-database-unsafe-path"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            bail!("opencode-database-unsafe-path");
        }
    }
    let snapshot = match (&immutable, after.modified()) {
        (None, _) => None,
        (Some(_), Ok(modified)) => Some((resolved_path.clone(), after.len(), modified)),
        (Some(_), Err(_)) => bail!("opencode-database-unreadable"),
    };
    #[cfg(not(unix))]
    let _ = (before, after);
    sidecars(&resolved_root)?;
    sql(conn.busy_timeout(Duration::from_millis(100)))?;
    // Bound VM work, including aggregate scans, per session: discovery
    // starts a fresh budget for each one, so a large store is listed in
    // full while no single session's queries can run unbounded. A lock
    // timeout alone does not stop expensive queries on a large database.
    let work = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&work);
    conn.progress_handler(
        1000,
        Some(move || counter.fetch_add(1, Ordering::Relaxed) + 1 >= WORK_BUDGET),
    );
    sql(conn.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;"))?;
    Ok(Some(Reader {
        conn,
        work,
        snapshot,
    }))
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
/// Lists the database's sessions. A session that cannot be described (an
/// invalid id, a bad timestamp, oversized metadata, or queries over the
/// work budget) is left out rather than hiding every other session; it
/// cannot be loaded either, so nothing about it is sent.
pub fn discover(root: &Path) -> Result<Vec<SessionRef>> {
    let Some(mut reader) = open(root)? else {
        return Ok(Vec::new());
    };
    let work = Arc::clone(&reader.work);
    let refs = {
        let tx = sql(reader.conn.transaction())?;
        schema(&tx)?;
        let count: u64 = sql(tx.query_row("SELECT count(*) FROM session", [], |r| r.get(0)))?;
        if count > SESSION_BUDGET as u64 {
            bail!("opencode-database-session-budget");
        }
        let ids: Vec<String> = {
            let mut stmt = sql(tx.prepare(
                "SELECT id FROM session WHERE typeof(id)='text' AND length(CAST(id AS BLOB))<=256 ORDER BY id",
            ))?;
            let rows = sql(stmt.query_map([], |r| r.get::<_, String>(0)))?;
            sql(rows.collect::<rusqlite::Result<_>>())?
        };
        let mut refs = Vec::new();
        for id in ids.iter().filter(|id| valid_id(id)) {
            // A fresh WORK_BUDGET for each session's queries.
            work.store(0, Ordering::Relaxed);
            if let Ok(found) = reference(&tx, root, id) {
                refs.push(found);
            }
        }
        refs
    };
    reader.finish()?;
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
    let mut reader = open(root)?.ok_or_else(|| anyhow!("opencode-database-unavailable"))?;
    let transcript = read_session(&mut reader.conn, root, id)?;
    reader.finish()?;
    Ok(transcript)
}
fn read_session(conn: &mut Connection, root: &Path, id: &str) -> Result<SessionTranscript> {
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
                .conn
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
        let conn = open(dir.path()).unwrap().unwrap().conn;
        let result = conn.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0));
        assert_eq!(
            sql(result).unwrap_err().to_string(),
            "opencode-database-unreadable"
        );
    }
    /// OpenCode leaves a cleanly closed WAL database with no sidecars. A
    /// read-only open must not create `-wal`/`-shm` in the person's folder.
    #[test]
    fn a_cleanly_closed_wal_database_gets_no_sidecars() {
        let dir = fixture();
        change(dir.path(), "PRAGMA journal_mode=WAL;");
        for name in ["opencode.db-wal", "opencode.db-shm"] {
            let _ = std::fs::remove_file(dir.path().join(name));
        }
        let before = std::fs::read(dir.path().join(DB)).unwrap();
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(load(dir.path(), &reference).unwrap().events.len(), 1);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(DB)]);
        assert_eq!(before, std::fs::read(dir.path().join(DB)).unwrap());
    }
    /// An `immutable` read whose database changed underneath it (an
    /// OpenCode that started and checkpointed meanwhile) is refused.
    #[test]
    fn an_immutable_read_of_a_changed_file_is_refused() {
        let dir = fixture();
        let reader = open(dir.path()).unwrap().unwrap();
        assert!(reader.snapshot.is_some());
        std::thread::sleep(Duration::from_millis(20));
        change(
            dir.path(),
            "INSERT INTO session VALUES('ses_later','/synthetic/project','1.19.0',1000,2000)",
        );
        assert_eq!(
            reader.finish().unwrap_err().to_string(),
            "opencode-database-busy"
        );
    }
    /// The VM-work cap bounds one session's queries, not the whole listing:
    /// a store whose total exceeds it is still listed in full.
    #[test]
    fn the_work_budget_is_per_session_not_per_listing() {
        let dir = fixture();
        change(
            dir.path(),
            "CREATE INDEX part_session ON part(session_id);
             CREATE INDEX message_session ON message(session_id);
             WITH RECURSIVE s(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM s WHERE x<400)
               INSERT INTO session SELECT 'ses_'||x,'/synthetic/project','1.19.0',1000,2000 FROM s;
             WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<400000)
               INSERT INTO part SELECT 'prt_'||x,'msg_fixture','ses_'||(x%400+1),1000,2000,'{}' FROM n;",
        );
        assert_eq!(discover(dir.path()).unwrap().len(), 401);
    }
    /// One malformed session is left out; the rest are still listed.
    #[test]
    fn a_malformed_session_does_not_hide_the_others() {
        let dir = fixture();
        change(
            dir.path(),
            "INSERT INTO session VALUES('ses_bad','/synthetic/project','1.19.0',-5,2000);
             INSERT INTO session VALUES('bad id!','/synthetic/project','1.19.0',1000,2000);",
        );
        let refs = discover(dir.path()).unwrap();
        assert_eq!(refs.len(), 1);
        assert!(refs[0].path.ends_with("ses_fixture"));
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
