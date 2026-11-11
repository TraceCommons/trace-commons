//! Bounded, read-only reader for the qualified OpenCode SQLite transcript schemas.
//! OpenCode 1.x sessions are read from `session`/`message`/`part`; OpenCode 2
//! sessions from `session_v2`/`session_message`, gated on the applied
//! migrations in `migration`. Account and credential tables are never queried.
//! SQLite may coordinate readers through an existing WAL shared-memory file.
use super::{SOURCE_OPENCODE, SessionRef, SessionTooLarge, SessionTranscript};
use anyhow::{Result, anyhow, bail};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags, Transaction};
use serde_json::{Map, Value, json};
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
    let mut conn = sql(match &immutable {
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
    sql(conn.progress_handler(
        1000,
        Some(move || counter.fetch_add(1, Ordering::Relaxed) + 1 >= WORK_BUDGET),
    ))?;
    sql(conn.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;"))?;
    Ok(Some(Reader {
        conn,
        work,
        snapshot,
    }))
}
/// Every migration id OpenCode 1.18.29 through 2.1.4 records in its
/// `migration` table, in order: the list in
/// `packages/core/src/database/migration.gen.ts` at anomalyco/opencode tag
/// v2.1.4. Its first 38 are 1.19.5's whole list; the last 12 are 2.0.0's to
/// 2.1.4's. A 2.x store that has applied any other migration has a schema
/// nobody checked, so it is refused rather than guessed at. Add ids only
/// after reading the new migrations' effect on `session_v2` and
/// `session_message`.
const KNOWN_MIGRATIONS: &[&str] = &[
    "20260127222353_familiar_lady_ursula",
    "20260211171708_add_project_commands",
    "20260213144116_wakeful_the_professor",
    "20260225215848_workspace",
    "20260227213759_add_session_workspace_id",
    "20260228203230_blue_harpoon",
    "20260303231226_add_workspace_fields",
    "20260309230000_move_org_to_state",
    "20260312043431_session_message_cursor",
    "20260323234822_events",
    "20260410174513_workspace-name",
    "20260413175956_chief_energizer",
    "20260423070820_add_icon_url_override",
    "20260427172553_slow_nightmare",
    "20260428004200_add_session_path",
    "20260501142318_next_venus",
    "20260504145000_add_sync_owner",
    "20260507164347_add_workspace_time",
    "20260510033149_session_usage",
    "20260511000411_data_migration_state",
    "20260511173437_session-metadata",
    "20260601010001_normalize_storage_paths",
    "20260601202201_amazing_prowler",
    "20260602002951_lowly_union_jack",
    "20260602182828_add_project_directories",
    "20260603001617_session_message_projection_indexes",
    "20260603040000_session_message_projection_order",
    "20260603141458_session_input_inbox",
    "20260603160727_jittery_ezekiel_stane",
    "20260604172448_event_sourced_session_input",
    "20260605003541_add_session_context_snapshot",
    "20260605042240_add_context_epoch_agent",
    "20260611035744_credential",
    "20260611192811_lush_chimera",
    "20260612174303_project_dir_strategy",
    "20260622142730_simplify_session_context_epoch",
    "20260622170816_reset_v2_session_state",
    "20260622202450_simplify_session_input",
    "20260804233008_loose_psylocke",
    "20260805200742_import_legacy_credentials",
    "20260808023530_workspace_domain",
    "20260811161259_execution_claim_attempts",
    "20260812181746_session_inbox",
    "20260812213948_worktree",
    "20260819222447_session_viewed_state",
    "20260823191254_nullable_workspace_binding",
    "20260910120000_clear_v1_session_permission",
    "20260923013825_project_time_active",
    "20261007190000_azure_cli_external_credential",
    "20261010180456_project_time_archived",
];
const MIGRATION_BUDGET: u64 = 4096;
const LEGACY_TABLES: [(&str, &[&str]); 3] = [
    (
        "session",
        &["id", "directory", "version", "time_created", "time_updated"],
    ),
    (
        "message",
        &["id", "session_id", "time_created", "time_updated", "data"],
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
        ],
    ),
];
/// The OpenCode 2 tables and the columns read from them
/// (`packages/core/src/session/sql.ts` at v2.1.4).
const V2_TABLES: [(&str, &[&str]); 3] = [
    (
        "session_v2",
        &[
            "id",
            "project_id",
            "directory",
            "version",
            "cost",
            "tokens_input",
            "tokens_output",
            "tokens_reasoning",
            "tokens_cache_read",
            "tokens_cache_write",
            "time_created",
            "time_updated",
        ],
    ),
    (
        "session_message",
        &[
            "id",
            "session_id",
            "type",
            "seq",
            "time_created",
            "time_updated",
            "data",
        ],
    ),
    ("migration", &["id"]),
];
/// Which session stores a database holds. OpenCode 1.x writes `session`,
/// `message` and `part`. OpenCode 2 writes `session_v2` and
/// `session_message`; upgrading a 1.x database leaves the 1.x tables in
/// place, frozen, and copies each session into `session_v2` under the same
/// id. 1.19.x already has a `session_message` table and a `migration`
/// table of its own, so only `session_v2` marks a 2.x store.
#[derive(Clone, Copy)]
struct Layout {
    legacy: bool,
    v2: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Store {
    Legacy,
    V2,
}
/// Whether `name` is a table; any other kind of object by that name is
/// refused.
fn table(tx: &Transaction<'_>, name: &str) -> Result<bool> {
    let mut stmt = sql(tx.prepare("SELECT type FROM sqlite_master WHERE name=?1"))?;
    let kinds = sql(stmt.query_map([name], |r| r.get::<_, String>(0)))?;
    let kinds = sql(kinds.collect::<rusqlite::Result<Vec<_>>>())?;
    match kinds.as_slice() {
        [] => Ok(false),
        [kind] if kind == "table" => Ok(true),
        _ => bail!("opencode-database-schema-unsupported"),
    }
}
fn columns(tx: &Transaction<'_>, table: &str, columns: &[&str]) -> Result<()> {
    let mut stmt = sql(tx.prepare(&format!("PRAGMA table_info({table})")))?;
    let rows = sql(stmt.query_map([], |r| r.get::<_, String>(1)))?;
    let names = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    if !columns.iter().all(|c| names.iter().any(|n| n == c)) {
        bail!("opencode-database-schema-unsupported");
    }
    Ok(())
}
fn schema(tx: &Transaction<'_>) -> Result<Layout> {
    let version: i64 = sql(tx.query_row("PRAGMA user_version", [], |r| r.get(0)))?;
    if version != 0 {
        bail!("opencode-database-schema-unsupported");
    }
    let legacy = table(tx, "session")?;
    if legacy {
        for (name, wanted) in LEGACY_TABLES {
            if !table(tx, name)? {
                bail!("opencode-database-schema-unsupported");
            }
            columns(tx, name, wanted)?;
        }
    }
    let v2 = table(tx, "session_v2")?;
    if v2 {
        for (name, wanted) in V2_TABLES {
            if !table(tx, name)? {
                bail!("opencode-database-schema-unsupported");
            }
            columns(tx, name, wanted)?;
        }
        // Gate the 2.x store on its schema, not on any version column:
        // every applied migration must be one of the pinned ones.
        let (count, untyped): (u64, u64) = sql(tx.query_row(
            "SELECT count(*), count(*) FILTER (WHERE typeof(id)<>'text' OR length(CAST(id AS BLOB))>256) FROM migration",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if count > MIGRATION_BUDGET || untyped > 0 {
            bail!("opencode-database-schema-unsupported");
        }
        let mut stmt = sql(tx.prepare("SELECT id FROM migration"))?;
        let ids = sql(stmt.query_map([], |r| r.get::<_, String>(0)))?;
        for id in ids {
            if !KNOWN_MIGRATIONS.contains(&sql(id)?.as_str()) {
                bail!("opencode-database-schema-unsupported");
            }
        }
    }
    if !legacy && !v2 {
        bail!("opencode-database-schema-unsupported");
    }
    Ok(Layout { legacy, v2 })
}
/// Where `id` is read from: `session_v2` whenever it holds the id, so a
/// session OpenCode 2 has copied is read once, from its copy.
fn store_of(tx: &Transaction<'_>, layout: Layout, id: &str) -> Result<Store> {
    if layout.v2 {
        let found: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_v2 WHERE id=?1)",
            [id],
            |r| r.get(0),
        ))?;
        if found || !layout.legacy {
            return Ok(Store::V2);
        }
    }
    Ok(Store::Legacy)
}
fn bounds(tx: &Transaction<'_>, id: &str, store: Store) -> Result<(u64, u64)> {
    // SQLite evaluates byte lengths without materializing the JSON in Rust. Include
    // identity/metadata and JSON framing overhead conservatively in the byte bound.
    let query = match store {
        Store::Legacy => "SELECT coalesce(sum(n),0), count(*) FROM (
         SELECT length(CAST(data AS BLOB))+length(CAST(id AS BLOB))*2+length(CAST(session_id AS BLOB))+128 AS n FROM message WHERE session_id=?1
         UNION ALL SELECT length(CAST(data AS BLOB))+length(CAST(id AS BLOB))*2+length(CAST(session_id AS BLOB))+length(CAST(message_id AS BLOB))+128 FROM part WHERE session_id=?1
         UNION ALL SELECT length(CAST(directory AS BLOB))+length(CAST(id AS BLOB))+length(CAST(version AS BLOB))+256 FROM session WHERE id=?1)",
        // Unsettled messages are counted too, though they are not read.
        Store::V2 => "SELECT coalesce(sum(n),0), count(*) FROM (
         SELECT length(CAST(data AS BLOB))+length(CAST(id AS BLOB))*2+length(CAST(type AS BLOB))*2+128 AS n FROM session_message WHERE session_id=?1
         UNION ALL SELECT length(CAST(directory AS BLOB))+length(CAST(id AS BLOB))+length(CAST(version AS BLOB))+length(CAST(project_id AS BLOB))+512 FROM session_v2 WHERE id=?1)",
    };
    let (bytes, count): (u64, u64) =
        sql(tx.query_row(query, [id], |r| Ok((r.get(0)?, r.get(1)?))))?;
    Ok((bytes, count.saturating_sub(1)))
}
fn reference(tx: &Transaction<'_>, root: &Path, id: &str, store: Store) -> Result<SessionRef> {
    let (bytes, _) = bounds(tx, id, store)?;
    let (metadata, times) = match store {
        Store::Legacy => (
            "SELECT length(CAST(directory AS BLOB))+length(CAST(version AS BLOB)) FROM session WHERE id=?1",
            "SELECT directory,time_created,(SELECT max(t) FROM (SELECT time_updated AS t FROM session WHERE id=?1 UNION ALL SELECT time_updated FROM message WHERE session_id=?1 UNION ALL SELECT time_updated FROM part WHERE session_id=?1)) FROM session WHERE id=?1",
        ),
        Store::V2 => (
            "SELECT length(CAST(directory AS BLOB))+length(CAST(version AS BLOB))+length(CAST(project_id AS BLOB)) FROM session_v2 WHERE id=?1",
            "SELECT directory,time_created,(SELECT max(t) FROM (SELECT time_updated AS t FROM session_v2 WHERE id=?1 UNION ALL SELECT time_updated FROM session_message WHERE session_id=?1)) FROM session_v2 WHERE id=?1",
        ),
    };
    let metadata_bytes: u64 = sql(tx.query_row(metadata, [id], |r| r.get(0)))?;
    if metadata_bytes > 65536 {
        bail!("opencode-database-metadata-too-large");
    }
    let (cwd, created, updated): (String, i64, i64) =
        sql(tx.query_row(times, [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))))?;
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
fn ids(tx: &Transaction<'_>, query: &str) -> Result<Vec<String>> {
    let mut stmt = sql(tx.prepare(query))?;
    let rows = sql(stmt.query_map([], |r| r.get::<_, String>(0)))?;
    sql(rows.collect::<rusqlite::Result<_>>())
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
        let layout = schema(&tx)?;
        // Legacy sessions OpenCode 2 has copied are listed from the copy.
        let legacy_only = if layout.v2 {
            " AND NOT EXISTS (SELECT 1 FROM session_v2 WHERE session_v2.id=session.id)"
        } else {
            ""
        };
        let mut count = 0u64;
        if layout.v2 {
            work.store(0, Ordering::Relaxed);
            count += sql(tx.query_row("SELECT count(*) FROM session_v2", [], |r| {
                r.get::<_, u64>(0)
            }))?;
        }
        if layout.legacy {
            work.store(0, Ordering::Relaxed);
            count += sql(tx.query_row(
                &format!("SELECT count(*) FROM session WHERE 1{legacy_only}"),
                [],
                |r| r.get::<_, u64>(0),
            ))?;
        }
        if count > SESSION_BUDGET as u64 {
            bail!("opencode-database-session-budget");
        }
        let mut found: Vec<(String, Store)> = Vec::new();
        const LISTABLE: &str = "typeof(id)='text' AND length(CAST(id AS BLOB))<=256";
        if layout.v2 {
            work.store(0, Ordering::Relaxed);
            let query = format!("SELECT id FROM session_v2 WHERE {LISTABLE}");
            found.extend(ids(&tx, &query)?.into_iter().map(|id| (id, Store::V2)));
        }
        if layout.legacy {
            work.store(0, Ordering::Relaxed);
            let query = format!("SELECT id FROM session WHERE {LISTABLE}{legacy_only}");
            found.extend(ids(&tx, &query)?.into_iter().map(|id| (id, Store::Legacy)));
        }
        found.sort();
        let mut refs = Vec::new();
        for (id, store) in found.iter().filter(|(id, _)| valid_id(id)) {
            // A fresh WORK_BUDGET for each session's queries.
            work.store(0, Ordering::Relaxed);
            if let Ok(found) = reference(&tx, root, id, *store) {
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
    let layout = schema(&tx)?;
    let store = store_of(&tx, layout, id)?;
    let (bytes, count) = bounds(&tx, id, store)?;
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
    let current = self::reference(&tx, root, id, store)?;
    match store {
        Store::Legacy => read_legacy(&tx, id, current),
        Store::V2 => read_v2(&tx, id, current),
    }
}
fn read_legacy(tx: &Transaction<'_>, id: &str, current: SessionRef) -> Result<SessionTranscript> {
    let version: String = sql(tx.query_row(
        "SELECT version FROM session WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    if !super::opencode::READABLE_VERSIONS.contains(&version.as_str()) {
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
/// Reassembles an OpenCode 2 session as the transfer envelope its own
/// `opencode export` writes (`{info, messages}`, core/src/session/transfer.ts
/// at v2.1.4) and hands it to the same parser exports go through.
fn read_v2(tx: &Transaction<'_>, id: &str, current: SessionRef) -> Result<SessionTranscript> {
    // Only the columns the transcript needs: title, metadata, revert and
    // permissions are never read. `info` is built as `fromRow` builds it
    // (core/src/session/info.ts), less those fields.
    let (version, project, cost, tokens, created, updated): (
        String,
        String,
        f64,
        [i64; 5],
        i64,
        i64,
    ) = sql(tx.query_row(
        "SELECT version,project_id,cost,tokens_input,tokens_output,tokens_reasoning,tokens_cache_read,tokens_cache_write,time_created,time_updated FROM session_v2 WHERE id=?1",
        [id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?],
                r.get(8)?,
                r.get(9)?,
            ))
        },
    ))?;
    // OpenCode reads messages in `seq` order (`SessionStore.messages`,
    // core/src/session/store.ts); `id` only makes a corrupt tie visible.
    let mut stmt = sql(tx.prepare(
        "SELECT id,type,seq,data FROM session_message WHERE session_id=?1 ORDER BY seq,id",
    ))?;
    let rows = sql(stmt.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    }))?;
    let mut messages = Vec::new();
    let mut previous = None;
    for row in rows {
        let (mid, kind, seq, data) = sql(row)?;
        if previous.is_some_and(|p| p >= seq) {
            bail!("opencode-database-ambiguous-order");
        }
        previous = Some(seq);
        // The row's data is the message less `id` and `type`, which are
        // columns; as in `rowInput` (core/src/session/history.ts) the data
        // is upgraded first and the columns then supply both.
        let Value::Object(data) = object(data, &[("type", &kind)])? else {
            unreachable!("object() returns an object");
        };
        let mut message = legacy_message(&kind, data);
        if message.get("id").is_some_and(|v| v.as_str() != Some(&mid)) || !valid_id(&mid) {
            bail!("opencode-database-ambiguous-id");
        }
        message.insert("id".into(), Value::String(mid));
        if settled(&kind, &message) {
            messages.push(Value::Object(message));
        }
    }
    let export = json!({
        "info": {
            "id": id,
            "projectID": project,
            "cost": cost,
            "tokens": {
                "input": tokens[0],
                "output": tokens[1],
                "reasoning": tokens[2],
                "cache": {"read": tokens[3], "write": tokens[4]},
            },
            "location": {"directory": current.cwd},
            "time": {"created": created, "updated": updated},
        },
        "messages": messages,
    });
    let bytes =
        serde_json::to_vec(&export).map_err(|_| anyhow!("opencode-database-export-failed"))?;
    let mut transcript = super::opencode::parse_export(&bytes)?;
    // A label only: forks inherit it, imports carry the importer's, and
    // development builds write `local`, so it never gates the read.
    transcript.agent_version = Some(version);
    Ok(transcript)
}
/// OpenCode's export keeps only settled messages: `isSettled` in
/// core/src/session/transfer.ts at v2.1.4. An assistant whose `time` is
/// not an object is kept, so the parser refuses it rather than it being
/// dropped unseen.
fn settled(kind: &str, message: &Map<String, Value>) -> bool {
    match kind {
        "assistant" => !message
            .get("time")
            .and_then(Value::as_object)
            .is_some_and(|time| !time.contains_key("completed")),
        "shell" | "compaction" => message.get("status").and_then(Value::as_str) != Some("running"),
        _ => true,
    }
}
/// Upgrades a message row written before OpenCode 2.1, as
/// `SessionLegacy.message` does (packages/schema/src/session-legacy.ts at
/// v2.1.4) before OpenCode decodes any stored message. The renamed fields
/// are provider-native data the parser never reads; upgrading them anyway
/// keeps the envelope the shape OpenCode itself would export.
fn legacy_message(kind: &str, data: Map<String, Value>) -> Map<String, Value> {
    match kind {
        "assistant" => legacy_content(rename(data, "providerState", "native")),
        "compaction" => provider_context(recent(rename(data, "providerState", "native"))),
        _ => data,
    }
}
fn legacy_content(mut data: Map<String, Value>) -> Map<String, Value> {
    if let Some(Value::Array(content)) = data.get_mut("content") {
        for part in content.iter_mut() {
            if let Value::Object(map) = part {
                let map = std::mem::take(map);
                *part = Value::Object(match map.get("type").and_then(Value::as_str) {
                    Some("text" | "reasoning") => rename(map, "state", "native"),
                    // A tool part's own `state` is its lifecycle status.
                    Some("tool") => tool_result(rename(map, "providerState", "native")),
                    _ => map,
                });
            }
        }
    }
    data
}
/// The kept text a summary compaction stored before `metadata.keepID`.
fn recent(mut data: Map<String, Value>) -> Map<String, Value> {
    match data.remove("recent") {
        Some(Value::String(text)) if !text.is_empty() => {
            let mut metadata = Map::new();
            metadata.insert("recent".into(), Value::String(text));
            if let Some(Value::Object(existing)) = data.remove("metadata") {
                metadata.extend(existing);
            }
            data.insert("metadata".into(), Value::Object(metadata));
            data
        }
        _ => data,
    }
}
/// The replacement window moves to `native.messages`, replacing `native`.
fn provider_context(mut data: Map<String, Value>) -> Map<String, Value> {
    let Some(Value::Object(mut context)) = data.remove("providerContext") else {
        return data;
    };
    let mut native = Map::new();
    if let Some(messages) = context.remove("messages") {
        native.insert(
            "messages".into(),
            match messages {
                Value::Array(items) => Value::Array(items.into_iter().map(media).collect()),
                other => other,
            },
        );
    }
    data.insert("native".into(), Value::Object(native));
    data
}
/// Before 2.0.15 a window's media part stored `mediaType` and base64 `data`
/// directly.
fn media(mut message: Value) -> Value {
    if let Some(Value::Array(content)) = message.get_mut("content") {
        for part in content.iter_mut() {
            let Value::Object(map) = part else { continue };
            if map.get("type").and_then(Value::as_str) != Some("media")
                || map.contains_key("media")
                || !map.get("data").is_some_and(Value::is_string)
            {
                continue;
            }
            let data = map.remove("data").expect("checked above");
            let mut source = Map::new();
            source.insert("type".into(), "base64".into());
            source.insert("data".into(), data);
            if let Some(media_type) = map.remove("mediaType") {
                source.insert("mediaType".into(), media_type);
            }
            map.insert("media".into(), json!({"source": source}));
        }
    }
    message
}
/// Result native data merges into the tool's single `native`.
fn tool_result(mut value: Map<String, Value>) -> Map<String, Value> {
    let Some(result) = value.remove("providerResultState") else {
        return value;
    };
    let Value::Object(result) = result else {
        return value;
    };
    let mut native = match value.remove("native") {
        Some(Value::Object(native)) => native,
        _ => Map::new(),
    };
    native.extend(result);
    value.insert("native".into(), Value::Object(native));
    value
}
/// The current name wins when a payload carries both.
fn rename(mut value: Map<String, Value>, from: &str, to: &str) -> Map<String, Value> {
    if let Some(moved) = value.remove(from) {
        if !value.contains_key(to) {
            value.insert(to.into(), moved);
        }
    }
    value
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
            "../../../trace-commons-contributor/tests/fixtures/opencode/native-1.19.0.sql"
        ))
        .unwrap();
        drop(db);
        let reference = discover(dir.path()).unwrap().remove(0);
        let actual = load(dir.path(), &reference).unwrap();
        let mut exported: Value = serde_json::from_slice(include_bytes!(
            "../../../trace-commons-contributor/tests/fixtures/opencode/completed.json"
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
    /// Patch releases whose storage code is unchanged from 1.19.0 read
    /// exactly like it; the next unchecked release still refuses.
    #[test]
    fn every_readable_release_reads_like_1_19_0_and_later_ones_refuse() {
        let dir = fixture();
        let reference = discover(dir.path()).unwrap().remove(0);
        let baseline = load(dir.path(), &reference).unwrap();
        for version in ["1.19.1", "1.19.2", "1.19.3", "1.19.4", "1.19.5"] {
            assert!(super::super::opencode::READABLE_VERSIONS.contains(&version));
            change(
                dir.path(),
                &format!("UPDATE session SET version='{version}'"),
            );
            let reference = discover(dir.path()).unwrap().remove(0);
            let transcript = load(dir.path(), &reference).unwrap();
            assert_eq!(transcript.agent_version.as_deref(), Some(version));
            assert_eq!(transcript.conversation_id, baseline.conversation_id);
            assert_eq!(transcript.events.len(), baseline.events.len());
        }
        for version in ["1.19.99", "1.20.0", "2.0.0", "2.1.4"] {
            change(
                dir.path(),
                &format!("UPDATE session SET version='{version}'"),
            );
            let reference = discover(dir.path()).unwrap().remove(0);
            assert!(
                load(dir.path(), &reference)
                    .unwrap_err()
                    .downcast_ref::<super::super::opencode::UnsupportedExportVersion>()
                    .is_some(),
                "{version}"
            );
        }
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
    /// OpenCode 2.1.4's `session_v2`, `session_message` and `migration`
    /// tables, as `packages/core/src/database/schema.gen.ts` and
    /// `migration.ts` create them at anomalyco/opencode v2.1.4 (the
    /// `project` foreign key is left out: no project rows are needed).
    const V2_DDL: &str = "CREATE TABLE `migration` (id TEXT PRIMARY KEY, time_completed INTEGER NOT NULL);
        CREATE TABLE `session_message` (`id` text PRIMARY KEY, `session_id` text NOT NULL, `type` text NOT NULL,
          `seq` integer NOT NULL, `time_created` integer NOT NULL, `time_updated` integer NOT NULL, `data` text NOT NULL,
          CONSTRAINT `fk_session_message_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE);
        CREATE TABLE `session_v2` (`id` text PRIMARY KEY, `project_id` text NOT NULL, `workspace_id` text, `parent_id` text,
          `fork_session_id` text, `fork_boundary` text, `slug` text NOT NULL, `directory` text NOT NULL, `path` text,
          `title` text, `version` text NOT NULL, `share_url` text, `summary_additions` integer, `summary_deletions` integer,
          `summary_files` integer, `summary_diffs` text, `metadata` text, `cost` real DEFAULT 0 NOT NULL,
          `tokens_input` integer DEFAULT 0 NOT NULL, `tokens_output` integer DEFAULT 0 NOT NULL,
          `tokens_reasoning` integer DEFAULT 0 NOT NULL, `tokens_cache_read` integer DEFAULT 0 NOT NULL,
          `tokens_cache_write` integer DEFAULT 0 NOT NULL, `revert` text, `permission` text, `agent` text, `model` text,
          `time_created` integer NOT NULL, `time_updated` integer NOT NULL, `time_idle` integer, `time_viewed` integer,
          `idle_outcome` text, `time_compacting` integer, `time_archived` integer, `time_suspended` integer,
          `resume_attempts` integer DEFAULT 0 NOT NULL);
        CREATE UNIQUE INDEX `session_message_session_seq_idx` ON `session_message` (`session_id`,`seq`);
        CREATE INDEX `session_message_session_type_seq_idx` ON `session_message` (`session_id`,`type`,`seq`);
        CREATE INDEX `session_message_session_time_created_id_idx` ON `session_message` (`session_id`,`time_created`,`id`);
        CREATE INDEX `session_message_time_created_idx` ON `session_message` (`time_created`);
        CREATE INDEX `session_v2_project_idx` ON `session_v2` (`project_id`);";
    const V2_ASSISTANT: &str = r#"{"agent":"build","model":{"id":"fixture-model","providerID":"trace-managed","variant":"default"},"content":[{"type":"reasoning","text":"Synthetic thought","time":{"created":1101,"completed":1102}},{"type":"text","text":"Synthetic answer"},{"type":"tool","id":"call_ok","name":"bash","time":{"created":1103,"ran":1104,"completed":1105},"state":{"status":"completed","input":{"command":"echo synthetic"},"content":[{"type":"text","text":"synthetic"}]}},{"type":"tool","id":"call_err","name":"bash","time":{"created":1106,"ran":1107,"completed":1108},"state":{"status":"error","input":{"command":"false"},"error":{"type":"Tool.Error","message":"synthetic failure"}}}],"finish":"stop","cost":0,"tokens":{"input":1,"output":2,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1100,"streamed":1150,"completed":1200}}"#;
    /// Writes a 2.1.4 session, `ses_fixture2x`, into the database at
    /// `root`. Message ids sort, and are inserted, in a different order
    /// from `seq`, and the `time_created` column runs backwards, so only
    /// `seq` gives the conversation's order.
    fn add_v2(root: &Path, migrations: &[&str]) {
        let db = Connection::open(root.join(DB)).unwrap();
        db.execute_batch(V2_DDL).unwrap();
        for id in migrations {
            db.execute("INSERT INTO migration VALUES(?1,0)", [id])
                .unwrap();
        }
        db.execute_batch(
            "INSERT INTO session_v2(id,project_id,slug,directory,version,cost,tokens_input,tokens_output,tokens_reasoning,tokens_cache_read,tokens_cache_write,model,title,time_created,time_updated)
             VALUES('ses_fixture2x','prj_synthetic','synthetic-slug','/synthetic/project2x','2.1.4',0.25,10,20,3,4,5,'{\"id\":\"fixture-model\",\"providerID\":\"trace-managed\"}','Synthetic title',1000,1300);
             INSERT INTO session_message VALUES('msg_b_idle','ses_fixture2x','idle',3,1,1300,'{\"outcome\":\"succeeded\",\"time\":{\"created\":1300}}');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_message VALUES('msg_a_assistant','ses_fixture2x','assistant',2,2,1200,?1)",
            [V2_ASSISTANT],
        )
        .unwrap();
        db.execute_batch("INSERT INTO session_message VALUES('msg_c_user','ses_fixture2x','user',1,3,1000,'{\"text\":\"Synthetic question\",\"files\":[],\"time\":{\"created\":1000}}');").unwrap();
    }
    fn fresh_v2() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        add_v2(dir.path(), KNOWN_MIGRATIONS);
        dir
    }
    /// The same conversation, as OpenCode 2.1.4's `opencode export` writes it.
    fn v2_export() -> Value {
        let mut assistant: Value = serde_json::from_str(V2_ASSISTANT).unwrap();
        assistant["id"] = "msg_a_assistant".into();
        assistant["type"] = "assistant".into();
        json!({"info":{"id":"ses_fixture2x","projectID":"prj_synthetic",
            "model":{"id":"fixture-model","providerID":"trace-managed","variant":"default"},
            "cost":0.25,"tokens":{"input":10,"output":20,"reasoning":3,"cache":{"read":4,"write":5}},
            "time":{"created":1000,"updated":1300},"title":"Synthetic title",
            "location":{"directory":"/synthetic/project2x"}},
          "messages":[
            {"id":"msg_c_user","type":"user","text":"Synthetic question","files":[],"time":{"created":1000}},
            assistant,
            {"id":"msg_b_idle","type":"idle","outcome":"succeeded","time":{"created":1300}}]})
    }
    fn assert_same_events(actual: &SessionTranscript, expected: &SessionTranscript) {
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
    }
    fn kinds(transcript: &SessionTranscript) -> Vec<super::super::SessionEventKind> {
        transcript.events.iter().map(|e| e.kind.clone()).collect()
    }

    #[test]
    fn the_pinned_migration_list_is_opencode_2_1_4s() {
        assert_eq!(KNOWN_MIGRATIONS.len(), 50);
        assert_eq!(KNOWN_MIGRATIONS[0], "20260127222353_familiar_lady_ursula");
        assert_eq!(
            KNOWN_MIGRATIONS[37],
            "20260622202450_simplify_session_input"
        );
        assert_eq!(KNOWN_MIGRATIONS[38], "20260804233008_loose_psylocke");
        assert_eq!(KNOWN_MIGRATIONS[49], "20261010180456_project_time_archived");
        let mut sorted = KNOWN_MIGRATIONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, KNOWN_MIGRATIONS);
    }

    #[test]
    fn a_fresh_2x_database_is_listed_and_read_without_changing_it() {
        let dir = fresh_v2();
        let before = std::fs::read(dir.path().join(DB)).unwrap();
        let refs = discover(dir.path()).unwrap();
        assert_eq!(refs.len(), 1);
        let reference = &refs[0];
        assert_eq!(reference.path, dir.path().join(DB).join("ses_fixture2x"));
        assert!(accepts_path(dir.path(), &reference.path));
        assert_eq!(reference.cwd.as_deref(), Some("/synthetic/project2x"));
        assert_eq!(reference.project.as_deref(), Some("project2x"));
        assert_eq!(reference.started_at.unwrap().timestamp_millis(), 1000);
        assert_eq!(
            reference.group_modified_at.unwrap().timestamp_millis(),
            1300
        );
        assert!(reference.size_bytes > V2_ASSISTANT.len() as u64);
        let transcript = load(dir.path(), reference).unwrap();
        use super::super::SessionEventKind::*;
        assert_eq!(
            kinds(&transcript),
            [
                User, Reasoning, Assistant, ToolCall, ToolResult, ToolCall, ToolResult, Opaque
            ]
        );
        assert_eq!(
            transcript.events[0].content.as_deref(),
            Some("Synthetic question")
        );
        assert_eq!(
            transcript.events[1].content.as_deref(),
            Some("Synthetic thought")
        );
        assert_eq!(
            transcript.events[2].content.as_deref(),
            Some("Synthetic answer")
        );
        assert_eq!(transcript.events[4].success, Some(true));
        assert_eq!(transcript.events[6].success, Some(false));
        assert_eq!(
            transcript.events[6].content.as_deref(),
            Some("synthetic failure")
        );
        assert_eq!(transcript.conversation_id.as_deref(), Some("ses_fixture2x"));
        assert_eq!(transcript.agent_version.as_deref(), Some("2.1.4"));
        assert_eq!(
            transcript.model.as_deref(),
            Some("trace-managed/fixture-model")
        );
        assert_eq!(transcript.cwd.as_deref(), Some("/synthetic/project2x"));
        assert_eq!(before, std::fs::read(dir.path().join(DB)).unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_2x_database_session_reads_like_its_export() {
        let dir = fresh_v2();
        let reference = discover(dir.path()).unwrap().remove(0);
        let actual = load(dir.path(), &reference).unwrap();
        let expected =
            super::super::opencode::parse_export(&serde_json::to_vec(&v2_export()).unwrap())
                .unwrap();
        assert_eq!(actual.conversation_id, expected.conversation_id);
        assert_eq!(actual.source, expected.source);
        assert_eq!(actual.source_session, expected.source_session);
        assert_eq!(actual.model, expected.model);
        assert_eq!(actual.project, expected.project);
        assert_eq!(actual.cwd, expected.cwd);
        assert_eq!(actual.started_at, expected.started_at);
        assert_same_events(&actual, &expected);
        // The export carries no producer version; the database row does.
        assert_eq!(expected.agent_version, None);
        assert_eq!(actual.agent_version.as_deref(), Some("2.1.4"));
        assert!(actual.attested_call.is_none() && actual.routing.is_empty());
    }

    /// OpenCode's export leaves out messages still being written
    /// (`isSettled`, core/src/session/transfer.ts); so does the reader.
    /// The unsettled assistant's content would refuse the session if read.
    #[test]
    fn unsettled_messages_are_left_out() {
        let dir = fresh_v2();
        change(
            dir.path(),
            r#"INSERT INTO session_message VALUES('msg_d_streaming','ses_fixture2x','assistant',4,1400,1400,'{"agent":"build","model":{"id":"fixture-model","providerID":"trace-managed"},"content":[{"type":"secret_future_part"}],"time":{"created":1400}}');
               INSERT INTO session_message VALUES('msg_e_shell','ses_fixture2x','shell',5,1500,1500,'{"shellID":"shl_x","command":"secret","status":"running","time":{"created":1500}}');
               INSERT INTO session_message VALUES('msg_f_compacting','ses_fixture2x','compaction',6,1600,1600,'{"status":"running","reason":"auto","summary":"","time":{"created":1600}}');
               INSERT INTO session_message VALUES('msg_g_compacted','ses_fixture2x','compaction',7,1700,1700,'{"status":"completed","reason":"auto","summary":"Synthetic summary","time":{"created":1700}}');"#,
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        let transcript = load(dir.path(), &reference).unwrap();
        assert_eq!(transcript.events.len(), 9);
        assert_eq!(
            transcript.events[8].kind,
            super::super::SessionEventKind::Opaque
        );
        assert_eq!(
            transcript.events[8].timestamp.unwrap().timestamp_millis(),
            1700
        );
        // Settled, the same assistant is read, and its unknown part refuses.
        change(
            dir.path(),
            r#"UPDATE session_message SET data=json_set(data,'$.time.completed',1450) WHERE id='msg_d_streaming'"#,
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode_unknown_part"
        );
    }

    #[test]
    fn messages_are_read_in_seq_order() {
        let dir = fresh_v2();
        let reference = discover(dir.path()).unwrap().remove(0);
        let first = load(dir.path(), &reference).unwrap();
        assert_eq!(first.events[0].kind, super::super::SessionEventKind::User);
        // Swapping the user and idle sequence numbers puts the idle first,
        // whose later time then makes the conversation run backwards.
        change(
            dir.path(),
            "UPDATE session_message SET seq=seq+10; UPDATE session_message SET seq=1 WHERE id='msg_b_idle';
             UPDATE session_message SET seq=3 WHERE id='msg_c_user';",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode_reordered_messages"
        );
    }

    /// An upgraded 1.x database keeps its legacy tables. OpenCode 2 copies
    /// each legacy session into `session_v2` under the same id; the copy is
    /// what is listed and read. A legacy session not yet copied is read
    /// from the legacy tables as before.
    #[test]
    fn an_upgraded_database_lists_each_session_once_and_prefers_session_v2() {
        let dir = fixture();
        add_v2(dir.path(), KNOWN_MIGRATIONS);
        change(
            dir.path(),
            r#"INSERT INTO session_v2(id,project_id,slug,directory,version,time_created,time_updated)
                 VALUES('ses_fixture','global','copied-slug','/synthetic/project','1.19.0',1000,2000);
               INSERT INTO session_message VALUES('msg_fixture','ses_fixture','user',0,1000,2000,'{"text":"Copied into session_v2","time":{"created":1000}}');
               INSERT INTO session VALUES('ses_legacyonly','/synthetic/legacy','1.19.5',1000,2000);
               INSERT INTO message VALUES('msg_legacyonly','ses_legacyonly',1000,2000,'{"role":"user","time":{"created":1000}}');
               INSERT INTO part VALUES('prt_legacyonly','msg_legacyonly','ses_legacyonly',1000,2000,'{"type":"text","text":"Legacy only"}');"#,
        );
        let before = std::fs::read(dir.path().join(DB)).unwrap();
        let refs = discover(dir.path()).unwrap();
        let ids: Vec<_> = refs
            .iter()
            .map(|r| r.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["ses_fixture", "ses_fixture2x", "ses_legacyonly"]);
        let copied = load(dir.path(), &refs[0]).unwrap();
        assert_eq!(copied.events.len(), 1);
        assert_eq!(
            copied.events[0].content.as_deref(),
            Some("Copied into session_v2")
        );
        assert_eq!(copied.agent_version.as_deref(), Some("1.19.0"));
        assert_eq!(load(dir.path(), &refs[1]).unwrap().events.len(), 8);
        let legacy = load(dir.path(), &refs[2]).unwrap();
        assert_eq!(legacy.events[0].content.as_deref(), Some("Legacy only"));
        assert_eq!(legacy.agent_version.as_deref(), Some("1.19.5"));
        assert_eq!(refs[2].cwd.as_deref(), Some("/synthetic/legacy"));
        assert_eq!(before, std::fs::read(dir.path().join(DB)).unwrap());
    }

    /// A session_v2 copy is read whatever its version column says: a copy
    /// of a session from a release the legacy reader refuses still reads.
    #[test]
    fn the_version_column_labels_but_does_not_gate_2x_sessions() {
        let dir = fresh_v2();
        for version in ["2.1.4", "2.0.0", "local", "99.0.0"] {
            change(
                dir.path(),
                &format!("UPDATE session_v2 SET version='{version}'"),
            );
            let reference = discover(dir.path()).unwrap().remove(0);
            let transcript = load(dir.path(), &reference).unwrap();
            assert_eq!(transcript.agent_version.as_deref(), Some(version));
        }
    }

    #[test]
    fn an_unknown_migration_refuses_the_2x_store() {
        for applied in [
            &["20261101000000_future_migration"][..],
            &[KNOWN_MIGRATIONS[0], "20260730195856_optional_session_title"][..],
        ] {
            let dir = fresh_v2();
            let reference = discover(dir.path()).unwrap().remove(0);
            for id in applied {
                change(
                    dir.path(),
                    &format!("INSERT OR IGNORE INTO migration VALUES('{id}',0)"),
                );
            }
            assert_eq!(
                discover(dir.path()).unwrap_err().to_string(),
                "opencode-database-schema-unsupported"
            );
            assert_eq!(
                load(dir.path(), &reference).unwrap_err().to_string(),
                "opencode-database-schema-unsupported"
            );
        }
        // In an upgraded database the refusal covers the legacy sessions
        // too: the whole database was last written by an unchecked release.
        let dir = fixture();
        let legacy = discover(dir.path()).unwrap().remove(0);
        add_v2(dir.path(), KNOWN_MIGRATIONS);
        assert_eq!(discover(dir.path()).unwrap().len(), 2);
        change(
            dir.path(),
            "INSERT INTO migration VALUES('20261101000000_future_migration',0)",
        );
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
        assert_eq!(
            load(dir.path(), &legacy).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
        // A subset of the known list (an older 2.x) is accepted.
        let dir = tempfile::tempdir().unwrap();
        add_v2(dir.path(), &KNOWN_MIGRATIONS[..40]);
        assert_eq!(discover(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_2x_store_without_its_migration_table_or_columns_refuses() {
        let dir = fresh_v2();
        change(dir.path(), "DROP TABLE migration");
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
        let dir = fresh_v2();
        change(
            dir.path(),
            "ALTER TABLE session_message DROP COLUMN time_updated",
        );
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
        let dir = fresh_v2();
        change(dir.path(), "INSERT INTO migration VALUES(7,0)");
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
    }

    /// A database with neither session table gets a schema label, not
    /// `unreadable`.
    #[test]
    fn a_database_without_session_tables_is_a_schema_refusal() {
        let dir = tempfile::tempdir().unwrap();
        Connection::open(dir.path().join(DB))
            .unwrap()
            .execute_batch("CREATE TABLE kv(key TEXT PRIMARY KEY, value TEXT)")
            .unwrap();
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-schema-unsupported"
        );
    }

    /// 1.19.x already has a `session_message` table and a `migration`
    /// table (its 38 migrations are the first 38 here). Without
    /// `session_v2` neither is consulted, so a pure 1.x database reads as
    /// before, whatever its migration rows say.
    #[test]
    fn a_1x_database_never_consults_session_message_or_migrations() {
        let dir = fixture();
        change(
            dir.path(),
            "CREATE TABLE migration(id TEXT PRIMARY KEY, time_completed INTEGER NOT NULL);
             INSERT INTO migration VALUES('20261101000000_future_migration',0);
             CREATE TABLE session_message(id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER, data TEXT);
             INSERT INTO session_message VALUES('msg_x','ses_fixture','user',0,'not json');",
        );
        let refs = discover(dir.path()).unwrap();
        assert_eq!(refs.len(), 1);
        let transcript = load(dir.path(), &refs[0]).unwrap();
        assert_eq!(transcript.events.len(), 1);
        assert_eq!(transcript.agent_version.as_deref(), Some("1.19.0"));
    }

    /// Rows written by 2.0.x carry pre-2.1 field names; they are upgraded
    /// as OpenCode's `SessionLegacy.message` (schema/src/session-legacy.ts)
    /// does before decoding.
    #[test]
    fn legacy_2_0_field_names_are_upgraded_like_opencode_does() {
        let upgrade = |kind: &str, data: Value| {
            Value::Object(legacy_message(kind, data.as_object().unwrap().clone()))
        };
        assert_eq!(
            upgrade(
                "assistant",
                json!({"providerState":{"a":1},"content":[
                    {"type":"text","text":"t","state":{"s":1}},
                    {"type":"reasoning","text":"r","state":{"s":2},"native":{"kept":true}},
                    {"type":"tool","id":"c","state":{"status":"completed"},"providerState":{"p":1},"providerResultState":{"r":2}},
                    {"type":"tool","id":"d","state":{"status":"error"},"native":{"n":0},"providerResultState":{"n":3}},
                    {"type":"tool","id":"e","state":{"status":"running"},"providerResultState":"not a record"},
                    "not a record"]})
            ),
            json!({"native":{"a":1},"content":[
                {"type":"text","text":"t","native":{"s":1}},
                {"type":"reasoning","text":"r","native":{"kept":true}},
                {"type":"tool","id":"c","state":{"status":"completed"},"native":{"p":1,"r":2}},
                {"type":"tool","id":"d","state":{"status":"error"},"native":{"n":3}},
                {"type":"tool","id":"e","state":{"status":"running"}},
                "not a record"]})
        );
        // The current name wins when both are present.
        assert_eq!(
            upgrade(
                "assistant",
                json!({"providerState":1,"native":2,"content":"x"})
            ),
            json!({"native":2,"content":"x"})
        );
        assert_eq!(
            upgrade(
                "compaction",
                json!({"status":"completed","recent":"kept text","metadata":{"keepID":"msg_k","recent":"newer"},
                    "native":{"dropped":true},
                    "providerContext":{"messages":[
                        {"content":[{"type":"media","mediaType":"image/png","data":"AAAA","name":"n"},{"type":"text","text":"x"}]},
                        {"content":[{"type":"media","media":{"kept":true},"data":"AAAA"}]}]}})
            ),
            json!({"status":"completed","metadata":{"recent":"newer","keepID":"msg_k"},
                "native":{"messages":[
                    {"content":[{"type":"media","name":"n","media":{"source":{"type":"base64","data":"AAAA","mediaType":"image/png"}}},{"type":"text","text":"x"}]},
                    {"content":[{"type":"media","media":{"kept":true},"data":"AAAA"}]}]}})
        );
        assert_eq!(
            upgrade(
                "compaction",
                json!({"recent":"","providerContext":"x","metadata":"m"})
            ),
            json!({"metadata":"m"})
        );
        assert_eq!(
            upgrade("compaction", json!({"recent":"t","providerContext":{}})),
            json!({"metadata":{"recent":"t"},"native":{}})
        );
        for kind in ["user", "idle", "shell"] {
            let data =
                json!({"providerState":1,"recent":"x","content":[{"type":"text","state":1}]});
            assert_eq!(upgrade(kind, data.clone()), data);
        }
    }

    #[test]
    fn a_2_0_shaped_session_reads_like_its_2_1_shape() {
        let dir = fresh_v2();
        let reference = discover(dir.path()).unwrap().remove(0);
        let current = load(dir.path(), &reference).unwrap();
        change(
            dir.path(),
            r#"UPDATE session_message SET data=json_set(json_remove(data,'$.content[0].native'),'$.providerState',json('{"step":1}'),'$.content[1].state',json('{"s":1}'),'$.content[2].providerResultState',json('{"r":1}')) WHERE id='msg_a_assistant'"#,
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        let legacy = load(dir.path(), &reference).unwrap();
        assert_same_events(&legacy, &current);
    }

    #[test]
    fn malformed_and_ambiguous_2x_rows_fail_closed() {
        for (statement, label) in [
            (
                "UPDATE session_message SET data='not json' WHERE id='msg_b_idle'",
                "opencode-database-malformed-json",
            ),
            (
                "UPDATE session_message SET data='[1]' WHERE id='msg_b_idle'",
                "opencode-database-malformed-json",
            ),
            (
                r#"UPDATE session_message SET data=json_set(data,'$.id','msg_other') WHERE id='msg_b_idle'"#,
                "opencode-database-ambiguous-id",
            ),
            (
                r#"UPDATE session_message SET data=json_set(data,'$.type','user') WHERE id='msg_b_idle'"#,
                "opencode-database-ambiguous-id",
            ),
            (
                "UPDATE session_message SET id='bad id!' WHERE id='msg_b_idle'",
                "opencode-database-ambiguous-id",
            ),
            (
                "UPDATE session_v2 SET tokens_input=-1",
                "opencode_invalid_v2_session",
            ),
            (
                "UPDATE session_v2 SET time_updated=500",
                "opencode_invalid_timestamp",
            ),
        ] {
            let dir = fresh_v2();
            let reference = discover(dir.path()).unwrap().remove(0);
            change(dir.path(), statement);
            assert_eq!(
                load(dir.path(), &reference).unwrap_err().to_string(),
                label,
                "{statement}"
            );
        }
    }

    #[test]
    fn duplicate_sequence_numbers_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        add_v2(dir.path(), KNOWN_MIGRATIONS);
        change(
            dir.path(),
            "DROP INDEX session_message_session_seq_idx; UPDATE session_message SET seq=1 WHERE id='msg_b_idle'",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode-database-ambiguous-order"
        );
    }

    #[test]
    fn budgets_bound_2x_sessions() {
        let dir = fresh_v2();
        change(
            dir.path(),
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000)
               INSERT INTO session_message SELECT 'msg_n'||x,'ses_fixture2x','idle',x+10,2000,2000,'{}' FROM n",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert_eq!(
            load(dir.path(), &reference).unwrap_err().to_string(),
            "opencode-database-record-budget"
        );
        let dir = fresh_v2();
        change(
            dir.path(),
            "UPDATE session_message SET data=zeroblob(16777217) WHERE id='msg_b_idle'",
        );
        let reference = discover(dir.path()).unwrap().remove(0);
        assert!(
            load(dir.path(), &reference)
                .unwrap_err()
                .downcast_ref::<SessionTooLarge>()
                .is_some()
        );
        let dir = fresh_v2();
        change(
            dir.path(),
            "UPDATE session_v2 SET directory='/'||hex(zeroblob(40000))",
        );
        // Oversized metadata leaves the session out of the listing.
        assert!(discover(dir.path()).unwrap().is_empty());
        let dir = fixture();
        add_v2(dir.path(), KNOWN_MIGRATIONS);
        change(
            dir.path(),
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<65535)
               INSERT INTO session_v2(id,project_id,slug,directory,version,time_created,time_updated)
               SELECT 'ses_n'||x,'global','s','/synthetic','2.1.4',1000,2000 FROM n",
        );
        // One legacy session plus 65,536 in session_v2 is over the budget.
        assert_eq!(
            discover(dir.path()).unwrap_err().to_string(),
            "opencode-database-session-budget"
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
