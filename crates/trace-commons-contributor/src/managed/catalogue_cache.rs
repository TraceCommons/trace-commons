//! One cache of NEAR AI's chat-model catalogue for the daemon.
//!
//! The picker, the background picker, a favorite's check and the session
//! rows' display names all read the same list. It is kept in memory for
//! [`TTL`] and a last-known-good copy lives on disk, so a cold start or an
//! offline machine still has names at once. A stale copy is served as it is
//! while one refresh runs in the background (stale-while-revalidate); a
//! reader waits on the network only when there is nothing at all to serve.
//!
//! The file holds what the public `/v1/models` list says: ids, display
//! names, tiers and token limits. No key, no account, no session.
//!
//! The cache has no clock and no network of its own: the caller passes the
//! time and does the fetch, so the rules here run without either.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::models::ChatModel;

/// How long a read catalogue is served without a refresh.
pub const TTL: Duration = Duration::from_secs(15 * 60);

/// The file's name in the state directory.
pub const FILE_NAME: &str = "managed-model-catalogue.json";

/// A catalogue file larger than this is not read.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    fetched_at_unix: u64,
    models: Vec<ChatModel>,
}

/// What a reader does with the cache.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    /// A fresh copy: use it.
    Fresh(Vec<ChatModel>),
    /// A stale copy: use it now. The caller owns the one refresh and must
    /// end it with [`CatalogueCache::store`] or [`CatalogueCache::refresh_failed`].
    Stale(Vec<ChatModel>),
    /// A stale copy while another caller's refresh is running: use it.
    StaleRefreshing(Vec<ChatModel>),
    /// Nothing to serve: the caller reads NEAR AI and ends with
    /// [`CatalogueCache::store`].
    Empty,
}

struct State {
    models: Option<Vec<ChatModel>>,
    fetched_at: Option<SystemTime>,
    refreshing: bool,
}

pub struct CatalogueCache {
    dir: Option<PathBuf>,
    state: Mutex<State>,
}

impl CatalogueCache {
    /// A cache backed by `dir`, loading the last-known-good copy from it.
    /// A missing, oversized, malformed or foreign-version file is ignored.
    pub fn open(dir: Option<&Path>) -> Self {
        let loaded = dir.and_then(load);
        let (models, fetched_at) = match loaded {
            Some((models, at)) => (Some(models), Some(at)),
            None => (None, None),
        };
        Self {
            dir: dir.map(Path::to_path_buf),
            state: Mutex::new(State {
                models,
                fetched_at,
                refreshing: false,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What is held, fresh or stale, with no refresh and no network.
    pub fn peek(&self) -> Option<Vec<ChatModel>> {
        self.state().models.clone()
    }

    /// The copy to serve at `now`, and whether this caller must refresh it.
    /// At most one stale reader is handed the refresh at a time.
    pub fn lookup(&self, now: SystemTime) -> Lookup {
        let mut state = self.state();
        let Some(models) = state.models.clone() else {
            return Lookup::Empty;
        };
        let fresh = state
            .fetched_at
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age < TTL);
        if fresh {
            Lookup::Fresh(models)
        } else if state.refreshing {
            Lookup::StaleRefreshing(models)
        } else {
            state.refreshing = true;
            Lookup::Stale(models)
        }
    }

    /// Keep a catalogue read at `now`, in memory and on disk. A disk
    /// failure leaves the memory copy in place.
    pub fn store(&self, models: Vec<ChatModel>, now: SystemTime) {
        {
            let mut state = self.state();
            state.models = Some(models.clone());
            state.fetched_at = Some(now);
            state.refreshing = false;
        }
        if let Some(dir) = &self.dir {
            let stored = Stored {
                version: VERSION,
                fetched_at_unix: now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                models,
            };
            if let Ok(body) = serde_json::to_vec(&stored) {
                let _ = crate::config::write_atomic_0600(dir, &dir.join(FILE_NAME), &body);
            }
        }
    }

    /// A refresh ended without an answer: keep what is held, and let the
    /// next stale reader try again.
    pub fn refresh_failed(&self) {
        self.state().refreshing = false;
    }
}

fn load(dir: &Path) -> Option<(Vec<ChatModel>, SystemTime)> {
    let path = dir.join(FILE_NAME);
    if std::fs::metadata(&path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let stored: Stored = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    if stored.version != VERSION {
        return None;
    }
    // The file is ours, but an id still has to be safe to name in a tool's
    // config before anything uses it.
    let models: Vec<ChatModel> = stored
        .models
        .into_iter()
        .filter(|m| super::opencode::valid_model(&m.id))
        .collect();
    if models.is_empty() {
        return None;
    }
    Some((
        models,
        UNIX_EPOCH + Duration::from_secs(stored.fetched_at_unix),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, name: Option<&str>) -> ChatModel {
        ChatModel {
            id: id.into(),
            name: name.map(str::to_owned),
            ..Default::default()
        }
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn empty_cache_has_nothing_and_peek_never_fetches() {
        let cache = CatalogueCache::open(None);
        assert_eq!(cache.peek(), None);
        assert_eq!(cache.lookup(at(1_000)), Lookup::Empty);
    }

    #[test]
    fn a_stored_catalogue_is_served_fresh_within_the_ttl() {
        let cache = CatalogueCache::open(None);
        let models = vec![model("a/b", Some("A B"))];
        cache.store(models.clone(), at(1_000));
        assert_eq!(
            cache.lookup(at(1_000 + TTL.as_secs() - 1)),
            Lookup::Fresh(models.clone())
        );
        assert_eq!(cache.peek(), Some(models));
    }

    #[test]
    fn a_stale_catalogue_is_served_and_one_reader_owns_the_refresh() {
        let cache = CatalogueCache::open(None);
        let old = vec![model("a/b", None)];
        cache.store(old.clone(), at(1_000));
        let later = at(1_000 + TTL.as_secs());
        assert_eq!(cache.lookup(later), Lookup::Stale(old.clone()));
        // A second reader while the refresh runs is served, not handed it.
        assert_eq!(cache.lookup(later), Lookup::StaleRefreshing(old.clone()));
        // The refresh lands: fresh again, new content.
        let new = vec![model("a/c", Some("A C"))];
        cache.store(new.clone(), later);
        assert_eq!(cache.lookup(later), Lookup::Fresh(new));
    }

    #[test]
    fn a_failed_refresh_keeps_the_old_copy_and_allows_another_try() {
        let cache = CatalogueCache::open(None);
        let old = vec![model("a/b", None)];
        cache.store(old.clone(), at(1_000));
        let later = at(1_000 + TTL.as_secs());
        assert_eq!(cache.lookup(later), Lookup::Stale(old.clone()));
        cache.refresh_failed();
        assert_eq!(cache.peek(), Some(old.clone()));
        assert_eq!(cache.lookup(later), Lookup::Stale(old));
    }

    #[test]
    fn a_cold_start_reads_the_disk_copy_as_stale_so_names_are_there_at_once() {
        let dir = dir();
        let models = vec![model("a/b", Some("A B")), model("c/d", None)];
        CatalogueCache::open(Some(dir.path())).store(models.clone(), at(1_000));
        let cold = CatalogueCache::open(Some(dir.path()));
        // Offline or not, peek answers with no network.
        assert_eq!(cold.peek(), Some(models.clone()));
        // Its age comes from the file: past the TTL it is stale, so a
        // refresh is due, but it is still served meanwhile.
        assert_eq!(
            cold.lookup(at(1_000 + TTL.as_secs() + 1)),
            Lookup::Stale(models)
        );
    }

    #[test]
    fn the_file_holds_only_catalogue_fields_and_is_private() {
        let dir = dir();
        let cache = CatalogueCache::open(Some(dir.path()));
        cache.store(
            vec![ChatModel {
                id: "a/b".into(),
                name: Some("A B".into()),
                context_length: Some(1000),
                ..Default::default()
            }],
            at(1_000),
        );
        let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: std::collections::BTreeSet<_> =
            value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["fetched_at_unix", "models", "version"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        assert_eq!(value["models"][0]["id"], "a/b");
        assert_eq!(value["models"][0]["context_length"], 1000);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Nothing else is left beside it.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn unreadable_oversized_foreign_and_unsafe_files_are_ignored() {
        let dir = dir();
        let path = dir.path().join(FILE_NAME);
        for body in [
            "not json".to_owned(),
            r#"{"version":2,"fetched_at_unix":1,"models":[{"id":"a/b"}]}"#.to_owned(),
            r#"{"version":1,"fetched_at_unix":1,"models":[]}"#.to_owned(),
            r#"{"version":1,"fetched_at_unix":1,"models":[{"id":"bad id; rm"}]}"#.to_owned(),
            " ".repeat(MAX_FILE_BYTES as usize + 1),
        ] {
            std::fs::write(&path, body).unwrap();
            assert_eq!(CatalogueCache::open(Some(dir.path())).peek(), None);
        }
    }
}
