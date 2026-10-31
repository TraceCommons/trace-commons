//! Content-free, bounded snapshots of preserved managed proxy ledgers.
use super::ironwire::{SummaryGroup, SummaryProofCounts, SummaryTotal, SummaryView};
use super::{ProofStatus, RoutedExchange};
use crate::config::ConfigStore;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const MAX_HOMES: usize = 512;
/// The columns read from a managed ledger, in the order the row reader
/// expects them.
macro_rules! exchange_columns {
    () => {
        "SELECT id, started_at, client_session_id, total_ms, facade, path, backend, requested_model, served_model, upstream_id, request_sha256, response_sha256, body_ref, rung, attempts, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, cost_usd, status, proof FROM exchanges"
    };
}
/// A ledger with no `failure` column: every row is a call.
const SELECT_ALL_EXCHANGES: &str = concat!(exchange_columns!(), " ORDER BY id LIMIT ?1");
/// A ledger with a `failure` column: a row carrying a code is not a call.
const SELECT_SERVED_EXCHANGES: &str = concat!(
    exchange_columns!(),
    " WHERE failure IS NULL ORDER BY id LIMIT ?1"
);
const MAX_ROWS: usize = 100_000;
const MAX_DB_BYTES: u64 = 512 * 1024 * 1024;
#[derive(Debug, Clone, Copy)]
pub(crate) struct ManagedHistoryError;
impl fmt::Display for ManagedHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed_history_unavailable")
    }
}
impl std::error::Error for ManagedHistoryError {}
impl From<rusqlite::Error> for ManagedHistoryError {
    fn from(_: rusqlite::Error) -> Self {
        Self
    }
}
impl From<std::io::Error> for ManagedHistoryError {
    fn from(_: std::io::Error) -> Self {
        Self
    }
}
pub(crate) struct ManagedHistory {
    entries: Vec<(PathBuf, RoutedExchange)>,
    from: DateTime<Utc>,
}
impl ManagedHistory {
    pub(crate) fn load(
        store: &ConfigStore,
        from: DateTime<Utc>,
    ) -> Result<Option<Self>, ManagedHistoryError> {
        Self::load_root(&store.dir().join("managed-routes"), from)
    }
    fn load_root(root: &Path, from: DateTime<Utc>) -> Result<Option<Self>, ManagedHistoryError> {
        match fs::symlink_metadata(root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ManagedHistoryError),
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => return Err(ManagedHistoryError),
            Ok(_) => {}
        }
        // SQLite NOFOLLOW rejects symlink ancestors too. macOS exposes its
        // temporary directory through /var -> /private/var; resolve the
        // already-validated root, while preserving NOFOLLOW for the managed
        // UUID directory and database appended below.
        let sqlite_root = fs::canonicalize(root)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut homes = Vec::new();
        for (n, item) in fs::read_dir(root)?.enumerate() {
            if n >= MAX_HOMES || Instant::now() >= deadline {
                return Err(ManagedHistoryError);
            }
            let item = item?;
            if item
                .file_name()
                .to_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .is_none()
            {
                continue;
            }
            let metadata = fs::symlink_metadata(item.path())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(ManagedHistoryError);
            }
            homes.push(item.path());
        }
        homes.sort();
        let mut entries = Vec::new();
        let mut found = false;
        let text_bytes = std::cell::Cell::new(0_usize);
        for home in homes {
            let path = home.join("ledger.sqlite");
            let metadata = match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                value => value?,
            };
            found = true;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_DB_BYTES
            {
                return Err(ManagedHistoryError);
            }
            for sidecar in ["ledger.sqlite-wal", "ledger.sqlite-shm"] {
                match fs::symlink_metadata(home.join(sidecar)) {
                    Ok(m)
                        if m.file_type().is_symlink() || !m.is_file() || m.len() > MAX_DB_BYTES =>
                    {
                        return Err(ManagedHistoryError);
                    }
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(ManagedHistoryError);
                    }
                    _ => {}
                }
            }
            let sqlite_path = sqlite_root
                .join(home.file_name().ok_or(ManagedHistoryError)?)
                .join("ledger.sqlite");
            let conn = Connection::open_with_flags(
                &sqlite_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )?;
            conn.busy_timeout(Duration::from_millis(50))?;
            conn.progress_handler(1000, Some(move || Instant::now() >= deadline));
            // Newer IronWire ledgers write a `failure` code for a request that
            // failed inside the proxy. Those are not calls. Older ledgers have
            // no such column and every row is read as before. Asked per home:
            // one machine can hold upgraded and older ledgers side by side.
            // `column_exists` matches the name case-insensitively, as SQLite
            // does.
            let query = if conn.column_exists(Some("main"), "exchanges", "failure")? {
                SELECT_SERVED_EXCHANGES
            } else {
                SELECT_ALL_EXCHANGES
            };
            let mut stmt = conn.prepare(query)?;
            let mut rows = stmt.query([
                i64::try_from(MAX_ROWS - entries.len() + 1).map_err(|_| ManagedHistoryError)?
            ])?;
            while let Some(row) = rows.next()? {
                if entries.len() >= MAX_ROWS || Instant::now() >= deadline {
                    return Err(ManagedHistoryError);
                }
                let text = |i| -> Result<Option<String>, ManagedHistoryError> {
                    match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Ok(None),
                        rusqlite::types::ValueRef::Text(bytes) if bytes.len() <= 4096 => {
                            let total = text_bytes
                                .get()
                                .checked_add(bytes.len())
                                .ok_or(ManagedHistoryError)?;
                            if total > 32 * 1024 * 1024 {
                                return Err(ManagedHistoryError);
                            }
                            text_bytes.set(total);
                            Ok(Some(
                                std::str::from_utf8(bytes)
                                    .map_err(|_| ManagedHistoryError)?
                                    .to_owned(),
                            ))
                        }
                        _ => Err(ManagedHistoryError),
                    }
                };
                let required = |i| text(i)?.ok_or(ManagedHistoryError);
                let timestamp = DateTime::parse_from_rfc3339(&required(1)?)
                    .map_err(|_| ManagedHistoryError)?
                    .with_timezone(&Utc);
                let cost: Option<f64> = row.get(19)?;
                if cost.is_some_and(|c| {
                    !c.is_finite() || c < 0.0 || c * 1_000_000.0 >= u64::MAX as f64
                }) {
                    return Err(ManagedHistoryError);
                }
                let exchange = RoutedExchange {
                    id: Some(row.get(0)?),
                    started_at: timestamp,
                    client_session_id: text(2)?,
                    total_ms: row.get(3)?,
                    facade: required(4)?,
                    path: text(5)?,
                    backend: required(6)?,
                    requested_model: text(7)?,
                    served_model: text(8)?,
                    upstream_id: text(9)?,
                    request_sha256: text(10)?,
                    response_sha256: text(11)?,
                    body_ref: text(12)?,
                    rung: required(13)?,
                    attempts: row.get(14)?,
                    input_tokens: row.get(15)?,
                    cache_read_tokens: row.get(16)?,
                    cache_write_tokens: row.get(17)?,
                    output_tokens: row.get(18)?,
                    cost_usd: cost,
                    status: row.get(20)?,
                    proof: text(21)?.as_deref().and_then(ProofStatus::parse),
                };
                entries.push((home.clone(), exchange));
            }
        }
        Ok(found.then_some(Self { entries, from }))
    }
    /// A snapshot of `rows` under each home, for tests elsewhere.
    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<(PathBuf, RoutedExchange)>) -> Self {
        Self {
            entries,
            from: DateTime::UNIX_EPOCH,
        }
    }
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<RoutedExchange> {
        self.rows_since(self.from)
    }
    /// Compare source-owned metadata without cloning it; window cutoffs are
    /// caller policy and do not represent changes to preserved history.
    pub(crate) fn same_rows(&self, other: &Self) -> bool {
        self.entries == other.entries
    }

    pub(crate) fn same_window(
        &self,
        other: &Self,
        from: DateTime<Utc>,
        other_from: DateTime<Utc>,
    ) -> bool {
        self.entries
            .iter()
            .filter(|(_, row)| row.started_at >= from)
            .eq(other
                .entries
                .iter()
                .filter(|(_, row)| row.started_at >= other_from))
    }

    /// Each managed route's rows by its home's name (the route's UUID),
    /// in id order, without copying them. Row ids are the home's own, so a
    /// fold keeps one cursor per home (`insights_week`'s ledger fold).
    pub(crate) fn rows_by_home(&self) -> BTreeMap<String, Vec<&RoutedExchange>> {
        let mut homes: BTreeMap<String, Vec<&RoutedExchange>> = BTreeMap::new();
        for (home, row) in &self.entries {
            if let Some(name) = home.file_name().and_then(|name| name.to_str()) {
                homes.entry(name.to_string()).or_default().push(row);
            }
        }
        homes
    }

    /// Check source membership without copying the snapshot's metadata.
    pub(crate) fn contains_row(&self, row: &RoutedExchange) -> bool {
        self.entries.iter().any(|(_, candidate)| candidate == row)
    }
    /// Apply the requested window before cloning any metadata.
    pub(crate) fn rows_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let from = from.max(self.from);
        self.entries
            .iter()
            .filter(|(_, row)| row.started_at >= from)
            .map(|(_, row)| row.clone())
            .collect()
    }
    /// Body capture consent must be checked by the caller before resolving.
    pub(crate) fn bodies_dir_for(&self, row: &RoutedExchange) -> Option<PathBuf> {
        let mut matches = self
            .entries
            .iter()
            .filter(|(_, candidate)| candidate == row);
        let (home, _) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        // Revalidate owning ancestors: an old snapshot must not follow a
        // replacement home symlink when capture is authorized later.
        for ancestor in [Some(home.as_path()), home.parent()].into_iter().flatten() {
            let metadata = fs::symlink_metadata(ancestor).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
        }
        let bodies = home.join("bodies");
        let metadata = fs::symlink_metadata(&bodies).ok()?;
        (metadata.is_dir() && !metadata.file_type().is_symlink()).then_some(bodies)
    }
    pub(crate) fn summary(&self, since: DateTime<Utc>) -> Result<SummaryView, ManagedHistoryError> {
        if since < self.from {
            return Err(ManagedHistoryError);
        }
        let mut summary = SummaryView {
            enabled: true,
            // A historical ledger does not establish the current receipt setting.
            receipts: false,
            since,
            groups: Vec::new(),
            routed: empty_total(),
            outside: empty_total(),
            unknown: empty_total(),
        };
        let mut groups = BTreeMap::new();
        for (_, row) in self
            .entries
            .iter()
            .filter(|(_, row)| row.started_at >= since)
        {
            let route = match row.proof {
                None => "unknown",
                Some(ProofStatus::Outside) => "outside",
                Some(_) => "routed",
            };
            let model = row
                .served_model
                .clone()
                .or_else(|| row.requested_model.clone());
            let group = groups
                .entry((model.clone(), row.backend.clone(), route))
                .or_insert_with(|| SummaryGroup {
                    group_id: String::new(),
                    model,
                    backend: row.backend.clone(),
                    route: route.into(),
                    work_kind: None,
                    total: empty_total(),
                });
            add(&mut group.total, row)?;
            add(
                match route {
                    "routed" => &mut summary.routed,
                    "outside" => &mut summary.outside,
                    _ => &mut summary.unknown,
                },
                row,
            )?;
        }
        if groups.len() > 4096 {
            return Err(ManagedHistoryError);
        }
        summary.groups = groups.into_values().collect();
        summary.groups.sort_by(|a, b| {
            b.total
                .calls
                .cmp(&a.total.calls)
                .then_with(|| a.backend.cmp(&b.backend))
                .then_with(|| a.model.cmp(&b.model))
                .then_with(|| route_rank(&a.route).cmp(&route_rank(&b.route)))
        });
        Ok(summary)
    }
}
fn route_rank(route: &str) -> u8 {
    match route {
        "routed" => 0,
        "outside" => 1,
        _ => 2,
    }
}
fn empty_total() -> SummaryTotal {
    SummaryTotal {
        calls: 0,
        priced_calls: 0,
        cost_usd: 0.0,
        proof: SummaryProofCounts {
            verified: 0,
            gateway_only: 0,
            unattested: 0,
            pending: 0,
            unavailable: 0,
            failed: 0,
            outside: 0,
            unrecorded: 0,
        },
    }
}
fn add(total: &mut SummaryTotal, row: &RoutedExchange) -> Result<(), ManagedHistoryError> {
    total.calls += 1;
    if let Some(cost) = row.cost_usd {
        total.priced_calls += 1;
        total.cost_usd += cost;
    }
    if !total.cost_usd.is_finite() || total.cost_usd * 1_000_000.0 >= u64::MAX as f64 {
        return Err(ManagedHistoryError);
    }
    let slot = match row.proof {
        Some(ProofStatus::Verified) => &mut total.proof.verified,
        Some(ProofStatus::GatewayOnly) => &mut total.proof.gateway_only,
        Some(ProofStatus::Unattested) => &mut total.proof.unattested,
        Some(ProofStatus::Pending) => &mut total.proof.pending,
        Some(ProofStatus::Unavailable) => &mut total.proof.unavailable,
        Some(ProofStatus::Failed) => &mut total.proof.failed,
        Some(ProofStatus::Outside) => &mut total.proof.outside,
        None => &mut total.proof.unrecorded,
    };
    *slot += 1;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture(root: &Path, proof: &str) -> PathBuf {
        let home = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(home.join("bodies")).unwrap();
        let conn = Connection::open(home.join("ledger.sqlite")).unwrap();
        // There is deliberately no error, prompt or response column: the reader
        // must project only metadata, and must never open the body files.
        conn.execute_batch("CREATE TABLE exchanges (id INTEGER PRIMARY KEY, started_at TEXT, client_session_id TEXT, total_ms INTEGER, facade TEXT, path TEXT, backend TEXT, requested_model TEXT, served_model TEXT, upstream_id TEXT, request_sha256 TEXT, response_sha256 TEXT, body_ref TEXT, rung TEXT, attempts INTEGER, input_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, output_tokens INTEGER, cost_usd REAL, status INTEGER, proof TEXT);").unwrap();
        conn.execute("INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, cost_usd, status, proof, body_ref) VALUES (1, '2026-10-24T00:00:00Z', 'openai', 'nearai', 'native', 1, 0.25, 200, ?1, 'opaque')", [proof]).unwrap();
        fs::write(home.join("bodies/opaque"), b"not a body document").unwrap();
        home
    }
    #[test]
    fn homes_preserve_row_identity_and_proof_counts_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let a = fixture(temp.path(), "verified");
        let b = fixture(temp.path(), "gateway_only");
        let c = fixture(temp.path(), "unattested");
        let d = fixture(temp.path(), "new_future_label");
        let originals: Vec<_> = [&a, &b, &c, &d]
            .into_iter()
            .map(|home| {
                (
                    home.join("ledger.sqlite"),
                    fs::read(home.join("ledger.sqlite")).unwrap(),
                )
            })
            .collect();
        let start = DateTime::UNIX_EPOCH;
        let first = ManagedHistory::load_root(temp.path(), start)
            .unwrap()
            .unwrap();
        let rows = first.rows();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.id == Some(1)));
        let summary = first.summary(start).unwrap();
        assert_eq!(summary.routed.calls, 3);
        assert_eq!(summary.routed.proof.verified, 1);
        assert_eq!(summary.routed.proof.gateway_only, 1);
        assert_eq!(summary.routed.proof.unattested, 1);
        assert_eq!(summary.unknown.proof.unrecorded, 1);
        assert_eq!(summary.routed.cost_usd, 0.75);
        assert_eq!(summary.routed.priced_calls, 3);
        for row in &rows {
            assert!(
                [&a, &b, &c, &d]
                    .iter()
                    .any(|home| first.bodies_dir_for(row) == Some(home.join("bodies")))
            );
        }
        for (path, bytes) in originals {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(
            ManagedHistory::load_root(temp.path(), start)
                .unwrap()
                .unwrap()
                .rows(),
            rows
        );
    }
    /// Each failure code with a status IronWire writes for it, so the filter
    /// is shown to key on `failure`, not on the status.
    const FAILURE_ROWS: [(&str, i64); 7] = [
        ("panic", 500),
        ("body_rejected", 413),
        ("body_rejected", 400),
        ("invalid_json", 400),
        ("unknown_login", 404),
        ("client_disconnected", 499),
        ("unrecorded", 500),
    ];
    fn insert_failure_rows(conn: &Connection) {
        for (i, (code, status)) in FAILURE_ROWS.iter().enumerate() {
            conn.execute(
                "INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, status, failure) VALUES (?1, '2026-10-24T00:00:00Z', 'openai', 'none', 'none', 0, ?2, ?3)",
                rusqlite::params![i as i64 + 2, status, code],
            )
            .unwrap();
        }
    }
    #[test]
    fn upgraded_and_older_ledgers_are_each_read_by_their_own_schema() {
        let temp = tempfile::tempdir().unwrap();
        // The upgraded home sorts first, so a column check cached from the
        // first home would make the older one fail with `no such column`.
        let homes = [
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
        ]
        .map(|name| {
            let home = fixture(temp.path(), "verified");
            let named = temp.path().join(name);
            fs::rename(&home, &named).unwrap();
            named
        });
        let upgraded = Connection::open(homes[0].join("ledger.sqlite")).unwrap();
        // Declared in another case: SQLite names are case-insensitive.
        upgraded
            .execute_batch("ALTER TABLE exchanges ADD COLUMN Failure TEXT;")
            .unwrap();
        insert_failure_rows(&upgraded);
        drop(upgraded);
        let older = Connection::open(homes[1].join("ledger.sqlite")).unwrap();
        older
            .execute("INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, status) VALUES (7, '2026-10-24T00:00:00Z', 'openai', 'nearai', 'native', 1, 499)", [])
            .unwrap();
        drop(older);
        let start = DateTime::UNIX_EPOCH;
        let history = ManagedHistory::load_root(temp.path(), start)
            .unwrap()
            .unwrap();
        let by_home = history.rows_by_home();
        let ids = |name: &str| -> Vec<Option<i64>> {
            by_home
                .get(name)
                .map(|rows| rows.iter().map(|r| r.id).collect())
                .unwrap_or_default()
        };
        assert_eq!(ids("00000000-0000-4000-8000-000000000001"), [Some(1)]);
        // The older ledger has no failure column; its 499 row is a call.
        assert_eq!(
            ids("00000000-0000-4000-8000-000000000002"),
            [Some(1), Some(7)]
        );
    }
    #[test]
    fn failed_request_rows_are_not_counted_as_calls() {
        let temp = tempfile::tempdir().unwrap();
        let home = fixture(temp.path(), "verified");
        let conn = Connection::open(home.join("ledger.sqlite")).unwrap();
        conn.execute_batch("ALTER TABLE exchanges ADD COLUMN failure TEXT;")
            .unwrap();
        insert_failure_rows(&conn);
        // A refusal carries no failure code and stays a counted call.
        conn.execute("INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, status) VALUES (100, '2026-10-24T00:00:00Z', 'openai', 'nearai', 'native', 1, 503)", [])
            .unwrap();
        drop(conn);
        let start = DateTime::UNIX_EPOCH;
        let history = ManagedHistory::load_root(temp.path(), start)
            .unwrap()
            .unwrap();
        let ids: Vec<_> = history.rows().iter().map(|r| r.id).collect();
        assert_eq!(ids, [Some(1), Some(100)]);
        let summary = history.summary(start).unwrap();
        let calls = summary.routed.calls + summary.outside.calls + summary.unknown.calls;
        assert_eq!(calls, 2);
        assert_eq!(summary.routed.calls, 1);
        assert_eq!(summary.unknown.calls, 1);
    }
    #[test]
    fn ledger_without_a_failure_column_counts_every_row() {
        let temp = tempfile::tempdir().unwrap();
        let home = fixture(temp.path(), "verified");
        let conn = Connection::open(home.join("ledger.sqlite")).unwrap();
        conn.execute("INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, status) VALUES (2, '2026-10-24T00:00:00Z', 'openai', 'nearai', 'native', 1, 503)", [])
            .unwrap();
        drop(conn);
        let start = DateTime::UNIX_EPOCH;
        let history = ManagedHistory::load_root(temp.path(), start)
            .unwrap()
            .unwrap();
        assert_eq!(history.rows().len(), 2);
        let summary = history.summary(start).unwrap();
        assert_eq!(summary.routed.calls + summary.unknown.calls, 2);
    }
    #[test]
    fn invalid_cost_refuses_instead_of_fabricating_zero() {
        let temp = tempfile::tempdir().unwrap();
        let home = fixture(temp.path(), "verified");
        let conn = Connection::open(home.join("ledger.sqlite")).unwrap();
        for cost in [-0.1, f64::INFINITY, u64::MAX as f64] {
            conn.execute("UPDATE exchanges SET cost_usd = ?1", [cost])
                .unwrap();
            assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        }
    }

    #[test]
    fn exact_duplicate_rows_refuse_body_ownership() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path(), "verified");
        fixture(temp.path(), "verified");
        let snapshot = ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH)
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshot.summary(DateTime::UNIX_EPOCH).unwrap().routed.calls,
            2
        );
        assert!(snapshot.bodies_dir_for(&snapshot.rows()[0]).is_none());
    }
    #[test]
    fn malformed_and_oversize_ledgers_refuse_without_repair() {
        let temp = tempfile::tempdir().unwrap();
        let home = fixture(temp.path(), "verified");
        let path = home.join("ledger.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE exchanges SET started_at = 'broken'", [])
            .unwrap();
        drop(conn);
        assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_DB_BYTES + 1)
            .unwrap();
        assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_DB_BYTES + 1);
    }
    #[cfg(unix)]
    #[test]
    fn ancestor_alias_is_readable_but_managed_root_symlink_refuses() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let root = real.join("managed-routes");
        fs::create_dir_all(&root).unwrap();
        fixture(&root, "verified");
        let alias = temp.path().join("alias");
        symlink(&real, &alias).unwrap();
        let history =
            ManagedHistory::load_root(&alias.join("managed-routes"), DateTime::UNIX_EPOCH)
                .unwrap()
                .unwrap();
        assert_eq!(history.rows().len(), 1);
        let linked_root = temp.path().join("linked-managed-routes");
        symlink(&root, &linked_root).unwrap();
        assert!(ManagedHistory::load_root(&linked_root, DateTime::UNIX_EPOCH).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_homes_and_databases_refuse() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(
            outside.path(),
            temp.path().join(uuid::Uuid::new_v4().to_string()),
        )
        .unwrap();
        assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        fs::remove_file(
            fs::read_dir(temp.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        let home = fixture(temp.path(), "verified");
        fs::remove_file(home.join("ledger.sqlite")).unwrap();
        symlink(outside.path().join("absent"), home.join("ledger.sqlite")).unwrap();
        assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        assert!(!outside.path().join("absent").exists());
    }
    #[test]
    fn row_and_directory_overflow_refuse() {
        let temp = tempfile::tempdir().unwrap();
        let home = fixture(temp.path(), "verified");
        let conn = Connection::open(home.join("ledger.sqlite")).unwrap();
        conn.execute_batch("WITH RECURSIVE n(x) AS (SELECT 2 UNION ALL SELECT x+1 FROM n WHERE x < 100001) INSERT INTO exchanges (id, started_at, facade, backend, rung, attempts, status) SELECT x, '2026-10-24T00:00:00Z', 'openai', 'nearai', 'native', 1, 200 FROM n;").unwrap();
        assert!(ManagedHistory::load_root(temp.path(), DateTime::UNIX_EPOCH).is_err());
        let empty = tempfile::tempdir().unwrap();
        for i in 0..=MAX_HOMES {
            fs::create_dir(empty.path().join(i.to_string())).unwrap();
        }
        assert!(ManagedHistory::load_root(empty.path(), DateTime::UNIX_EPOCH).is_err());
    }
}
