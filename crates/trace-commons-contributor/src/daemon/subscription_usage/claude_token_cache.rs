//! The daemon's own copy of a Claude Code usage access token.
//!
//! Reading Claude Code's Keychain item can show a macOS dialog. After one
//! successful read, the access token, its expiry, a binding to the login
//! and account it came from, and the item revision it was read at are kept
//! in a Keychain item this app owns (the data-protection store under its
//! access group, which never prompts the signed app). Later Refreshes reuse
//! it until it expires, Anthropic rejects it, or the binding no longer
//! matches.
//!
//! Claude Code updates its item in place when it renews its own token
//! (`security add-generic-password -U`), which keeps the item's access list,
//! including an "Always Allow" grant, but moves its modification date. A
//! changed revision therefore does not discard a token that has not expired:
//! the caller makes one read that cannot show a dialog, replaces the token
//! only when the item now holds a different one, and otherwise keeps serving
//! the cached token (see `claude::load`).
//!
//! The refresh token is never stored or used, and tokens are never renewed
//! here: renewing would rotate Claude Code's own credentials.
//!
//! One fixed entry holds every cached login, so unenroll and the Claude
//! switch clear it with a single delete, including logins that have since
//! gone away.
//!
//! A build without the keychain entitlement (a locally built, ad-hoc signed
//! app) cannot reach the app's own store: the first store operation answers
//! `unentitled`, and from then on this process keeps the cache in memory
//! only. `unentitled()` lets the caller skip background reads of Claude
//! Code's item in such a build.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_contributor_platform::credential_store::MAX_SECRET_BYTES;
use trace_commons_contributor_platform::external_claude_credentials::{
    ItemRevision, SecretBytes, wipe,
};

/// A cached token is not used within this margin of its expiry.
const EXPIRY_MARGIN_SECONDS: i64 = 5 * 60;
/// A bounded number of logins, oldest dropped first.
const MAX_ENTRIES: usize = 8;
/// 2 added `revision` and took the item revision out of `binding`. A version
/// 1 document reads as empty: one more read of Claude Code's item.
const VERSION: u8 = 2;
/// The error a store returns when this build is not entitled to it.
pub(super) const UNENTITLED: &str = "unentitled";
/// How long an `UNENTITLED` answer is trusted before the OS store is asked
/// again, so a transient refusal in a signed build does not last until the
/// process restarts.
const REPROBE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Where the cache document lives. Production is the OS credential store;
/// test builds use memory only. A store this build cannot reach at all
/// answers `UNENTITLED`.
pub(super) trait TokenStore: Send + Sync {
    fn read(&self) -> Result<Option<SecretBytes>, &'static str>;
    fn write(&self, bytes: &[u8]) -> Result<(), &'static str>;
    fn clear(&self) -> Result<(), &'static str>;
}

#[derive(Serialize, Deserialize)]
struct Document {
    v: u8,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// A fingerprint of the login spec, never a path or account name.
    login: String,
    /// A fingerprint of the credential this token was read from: Keychain
    /// service, local account evidence and login spec. Not the item
    /// revision, which moves whenever Claude Code renews its own token.
    binding: String,
    /// Which item this token was read from: changes when Claude Code deletes
    /// and recreates its item, as a sign-out and sign-in do.
    item: String,
    /// The attribute-only content revision this token was last confirmed at.
    revision: String,
    /// Milliseconds since the epoch, from Claude Code's `expiresAt`.
    expires_at: i64,
    token: String,
}

impl Drop for Entry {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, so the String stays valid.
        wipe(unsafe { self.token.as_bytes_mut() });
    }
}

/// A cached token, the item it was read from and the content revision it
/// was last confirmed at.
pub(super) struct Cached {
    pub token: SecretBytes,
    pub revision: ItemRevision,
}

/// What the cache did while `store` was refusing this build, so the store's
/// own document can be brought up to date before it is used again.
#[derive(Default)]
struct Fallback {
    /// When `store` last answered `UNENTITLED`; `None` while it is usable.
    since: Option<Instant>,
    /// Everything was cleared (unenroll, the Claude switch turned off).
    cleared: bool,
    /// Logins forgotten (rejected, or their item gone or recreated).
    forgotten: BTreeSet<String>,
}

pub(super) struct TokenCache {
    store: Box<dyn TokenStore>,
    /// Where the cache lives once `store` has answered `UNENTITLED`.
    memory: MemoryStore,
    /// The in-memory mode, if `store` has refused this build. Never held
    /// across a store call.
    fallback: Mutex<Fallback>,
    reprobe_after: Duration,
    /// Per login, the item revision whose silent check last ended without
    /// an answer. Memory only: it only spares repeating that check.
    unanswered: Mutex<BTreeMap<String, ItemRevision>>,
    /// Serializes read-modify-write of the single entry.
    lock: Mutex<()>,
    /// Bumped by `clear`. A write that started under an older epoch is
    /// dropped, so a Refresh finishing after unenroll cannot recreate it.
    epoch: AtomicU64,
}

impl TokenCache {
    pub(super) fn new(store: Box<dyn TokenStore>) -> Self {
        Self {
            store,
            memory: MemoryStore::default(),
            fallback: Mutex::new(Fallback::default()),
            reprobe_after: REPROBE_AFTER,
            unanswered: Mutex::new(BTreeMap::new()),
            lock: Mutex::new(()),
            epoch: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(super) fn with_reprobe_after(mut self, after: Duration) -> Self {
        self.reprobe_after = after;
        self
    }

    /// Test only: make the next store operation re-probe the OS store.
    #[cfg(test)]
    fn expire_window(&self) {
        let mut fallback = self.fallback.lock().unwrap();
        if fallback.since.is_some() {
            fallback.since =
                Instant::now().checked_sub(self.reprobe_after + Duration::from_secs(1));
            assert!(fallback.since.is_some());
        }
    }

    /// True while the OS store's last answer was that this build is not
    /// entitled to it. Known only after the first store operation, which
    /// `lookup` makes. Never waits on a store call.
    pub(super) fn unentitled(&self) -> bool {
        self.fallback
            .lock()
            .map_or(true, |fallback| fallback.since.is_some())
    }

    fn refused_now(&self) {
        if let Ok(mut fallback) = self.fallback.lock() {
            fallback.since = Some(Instant::now());
        }
    }

    /// The OS store, or memory once that store has refused this build. A
    /// refusal is trusted for `reprobe_after`. The next operation after that
    /// first brings the OS store up to date (`reconcile`); only once that
    /// succeeds is the store used again. Callers hold `self.lock`, so store
    /// operations and the fallback's bookkeeping never interleave.
    fn with_store<T>(
        &self,
        operation: impl Fn(&dyn TokenStore) -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let since = self
            .fallback
            .lock()
            .map_or(Some(Instant::now()), |fallback| fallback.since);
        if let Some(at) = since {
            if at.elapsed() < self.reprobe_after {
                return operation(&self.memory);
            }
            // Never read the store's document before the clears and forgets
            // made in memory are applied to it.
            if self.reconcile().is_err() {
                self.refused_now();
                return operation(&self.memory);
            }
            if let Ok(mut fallback) = self.fallback.lock() {
                *fallback = Fallback::default();
            }
            let _ = self.memory.clear();
        }
        match operation(self.store.as_ref()) {
            Err(UNENTITLED) => {
                self.refused_now();
                operation(&self.memory)
            }
            other => other,
        }
    }

    /// Apply what happened in memory to the OS store's document: a clear
    /// empties it, a forgotten login is removed, and a token remembered in
    /// memory replaces the store's entry for that login.
    fn reconcile(&self) -> Result<(), &'static str> {
        let (cleared, forgotten) = {
            let fallback = self.fallback.lock().map_err(|_| "unavailable")?;
            (fallback.cleared, fallback.forgotten.clone())
        };
        let mut document = if cleared {
            empty_document()
        } else {
            parse(self.store.read()?)
        };
        document.entries.retain(|e| !forgotten.contains(&e.login));
        let carried = parse(self.memory.read()?);
        for entry in carried.entries {
            document.entries.retain(|e| e.login != entry.login);
            document.entries.push(entry);
        }
        bound(&mut document);
        if document.entries.is_empty() {
            return self.store.clear();
        }
        let bytes = SecretBytes::new(serde_json::to_vec(&document).map_err(|_| "unavailable")?);
        self.store.write(&bytes)
    }

    /// Record a forget or clear made while in memory, for `reconcile`.
    fn tombstone(&self, login: Option<&str>) {
        let Ok(mut fallback) = self.fallback.lock() else {
            return;
        };
        if fallback.since.is_none() {
            return;
        }
        match login {
            Some(login) => {
                fallback.forgotten.insert(login.into());
            }
            None => {
                fallback.cleared = true;
                fallback.forgotten.clear();
            }
        }
    }

    /// Whether `login`'s silent check already ended without an answer at
    /// exactly `revision`.
    pub(super) fn unanswered(&self, login: &str, revision: &ItemRevision) -> bool {
        self.unanswered
            .lock()
            .is_ok_and(|map| map.get(login) == Some(revision))
    }

    /// Note that `login`'s silent check at `revision` found no answer.
    pub(super) fn note_unanswered(&self, login: &str, revision: &ItemRevision) {
        if let Ok(mut map) = self.unanswered.lock() {
            map.insert(login.into(), revision.clone());
        }
    }

    fn answered(&self, login: Option<&str>) {
        if let Ok(mut map) = self.unanswered.lock() {
            match login {
                Some(login) => {
                    map.remove(login);
                }
                None => map.clear(),
            }
        }
    }

    /// The cache this build uses. Test builds never reach an OS store.
    pub(super) fn for_build() -> Self {
        if cfg!(any(test, feature = "test-credential-store")) {
            return Self::new(Box::new(MemoryStore::default()));
        }
        Self::new(os_store())
    }

    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn document(&self) -> Document {
        self.with_store(|store| store.read())
            .map(parse)
            .unwrap_or_else(|_| empty_document())
    }

    fn save(&self, document: &Document) {
        if document.entries.is_empty() {
            let _ = self.with_store(|store| store.clear());
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(document) {
            let bytes = SecretBytes::new(bytes);
            let _ = self.with_store(|store| store.write(&bytes));
        }
    }

    /// The cached token for `login`, if it was read from the credential
    /// `binding` names and is not within the margin of expiry, with the item
    /// revision it was last confirmed at. A stale or mismatched entry is
    /// removed. A changed revision is the caller's to judge, not a miss.
    pub(super) fn lookup(&self, login: &str, binding: &str, now: DateTime<Utc>) -> Option<Cached> {
        let _guard = self.lock.lock().ok()?;
        let mut document = self.document();
        let index = document.entries.iter().position(|e| e.login == login)?;
        let entry = &document.entries[index];
        let usable = entry.binding == binding
            && entry.expires_at - EXPIRY_MARGIN_SECONDS * 1000 > now.timestamp_millis();
        if usable {
            return Some(Cached {
                token: SecretBytes::new(entry.token.as_bytes().to_vec()),
                revision: ItemRevision {
                    item: entry.item.clone(),
                    content: entry.revision.clone(),
                },
            });
        }
        document.entries.remove(index);
        self.save(&document);
        None
    }

    /// Keep `token` for `login`. Best effort: a failure here never fails the
    /// Refresh that produced the token. Skipped when the cache was cleared
    /// since `epoch`, or when the token would not fit the store's bound.
    pub(super) fn remember(
        &self,
        epoch: u64,
        login: &str,
        binding: &str,
        revision: &ItemRevision,
        expires_at: i64,
        token: &[u8],
    ) {
        let Ok(token) = std::str::from_utf8(token) else {
            return;
        };
        self.answered(Some(login));
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        if self.epoch() != epoch {
            return;
        }
        let mut document = self.document();
        document.entries.retain(|e| e.login != login);
        document.entries.push(Entry {
            login: login.into(),
            binding: binding.into(),
            item: revision.item.clone(),
            revision: revision.content.clone(),
            expires_at,
            token: token.into(),
        });
        bound(&mut document);
        if document.entries.last().is_none_or(|e| e.login != login) {
            return;
        }
        self.save(&document);
    }

    /// Record that `login`'s cached token is still the one the item holds at
    /// `revision`, so later loads do not read the item again. A no-op when
    /// the entry is gone, bound elsewhere, read from another item, or the
    /// cache was cleared since `epoch`.
    pub(super) fn confirm(&self, epoch: u64, login: &str, binding: &str, revision: &ItemRevision) {
        self.answered(Some(login));
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        if self.epoch() != epoch {
            return;
        }
        let mut document = self.document();
        let Some(entry) = document
            .entries
            .iter_mut()
            .find(|e| e.login == login && e.binding == binding && e.item == revision.item)
        else {
            return;
        };
        if entry.revision == revision.content {
            return;
        }
        entry.revision = revision.content.clone();
        self.save(&document);
    }

    /// Remove `login`'s entry: Anthropic rejected it, or Claude Code's item
    /// is gone or was recreated.
    pub(super) fn forget(&self, login: &str) {
        self.answered(Some(login));
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        let mut document = self.document();
        let before = document.entries.len();
        document.entries.retain(|e| e.login != login);
        if document.entries.len() != before {
            self.save(&document);
        }
        // In memory mode the store may still hold this login's entry. Last,
        // and whether or not anything was removed, so a refusal that first
        // arrives on the write above is covered too.
        self.tombstone(Some(login));
    }

    /// Invalidate writes already in flight without touching the store, so a
    /// caller that must not block can do this at once and `clear` later.
    pub(super) fn begin_clear(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Remove every cached token: unenroll, or the Claude switch turned off.
    pub(super) fn clear(&self) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.answered(None);
        let _ = self.with_store(|store| store.clear());
        // In memory mode the store still holds what it had.
        self.tombstone(None);
    }
}

fn empty_document() -> Document {
    Document {
        v: VERSION,
        entries: Vec::new(),
    }
}

/// A stored document, or an empty one when absent or unreadable.
fn parse(bytes: Option<SecretBytes>) -> Document {
    bytes
        .and_then(|bytes| serde_json::from_slice::<Document>(&bytes).ok())
        .filter(|doc| doc.v == VERSION)
        .unwrap_or_else(empty_document)
}

/// Keep at most `MAX_ENTRIES` logins, and drop the oldest until the document
/// fits the store's bound. The newest entry is last; it is dropped only when
/// it cannot fit on its own.
fn bound(document: &mut Document) {
    while document.entries.len() > MAX_ENTRIES {
        document.entries.remove(0);
    }
    while !document.entries.is_empty()
        && serde_json::to_vec(&*document).map_or(usize::MAX, |b| SecretBytes::new(b).len())
            > MAX_SECRET_BYTES
    {
        document.entries.remove(0);
    }
}

#[derive(Default)]
pub(super) struct MemoryStore(Mutex<Option<Vec<u8>>>);

impl Drop for MemoryStore {
    fn drop(&mut self) {
        if let Ok(slot) = self.0.get_mut() {
            if let Some(bytes) = slot.as_mut() {
                wipe(bytes);
            }
        }
    }
}

impl TokenStore for MemoryStore {
    fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
        let slot = self.0.lock().map_err(|_| "unavailable")?;
        Ok(slot.as_ref().map(|bytes| SecretBytes::new(bytes.clone())))
    }
    fn write(&self, bytes: &[u8]) -> Result<(), &'static str> {
        let mut slot = self.0.lock().map_err(|_| "unavailable")?;
        if let Some(old) = slot.as_mut() {
            wipe(old);
        }
        *slot = Some(bytes.to_vec());
        Ok(())
    }
    fn clear(&self) -> Result<(), &'static str> {
        let mut slot = self.0.lock().map_err(|_| "unavailable")?;
        if let Some(old) = slot.as_mut() {
            wipe(old);
        }
        *slot = None;
        Ok(())
    }
}

/// The app's own data-protection Keychain item on macOS. Elsewhere Claude's
/// credential is a file, which never prompts, so nothing is cached.
fn os_store() -> Box<dyn TokenStore> {
    #[cfg(target_os = "macos")]
    {
        Box::new(OsStore)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NoStore)
    }
}

#[cfg(target_os = "macos")]
struct OsStore;

#[cfg(target_os = "macos")]
use super::super::credential_store::CredentialError;

/// Only the entitlement answer is kept; every other failure is a label.
#[cfg(target_os = "macos")]
fn store_error(error: CredentialError) -> &'static str {
    match error {
        CredentialError::Unentitled => UNENTITLED,
        _ => "unavailable",
    }
}

#[cfg(target_os = "macos")]
impl OsStore {
    fn backend() -> Result<super::super::os_secret_store::OsSecretBackend, &'static str> {
        super::super::os_secret_store::OsSecretBackend::claude_usage_cache(
            &crate::channel::KEYCHAIN,
        )
        .map_err(store_error)
    }
}

#[cfg(target_os = "macos")]
impl TokenStore for OsStore {
    fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
        match Self::backend()?.read_claude_usage_cache() {
            Ok(bytes) => Ok(Some(SecretBytes::new(bytes))),
            Err(CredentialError::NoEntry) => Ok(None),
            Err(error) => Err(store_error(error)),
        }
    }
    fn write(&self, bytes: &[u8]) -> Result<(), &'static str> {
        Self::backend()?
            .write_claude_usage_cache(bytes)
            .map_err(store_error)
    }
    fn clear(&self) -> Result<(), &'static str> {
        match Self::backend()?.delete_claude_usage_cache() {
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct NoStore;

#[cfg(not(target_os = "macos"))]
impl TokenStore for NoStore {
    fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
        Ok(None)
    }
    fn write(&self, _bytes: &[u8]) -> Result<(), &'static str> {
        Ok(())
    }
    fn clear(&self) -> Result<(), &'static str> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 28, 12, 0, 0).unwrap()
    }
    fn later(minutes: i64) -> i64 {
        (now() + chrono::Duration::minutes(minutes)).timestamp_millis()
    }
    fn cache() -> TokenCache {
        TokenCache::new(Box::new(MemoryStore::default()))
    }
    fn rev(content: &str) -> ItemRevision {
        ItemRevision {
            item: "item".into(),
            content: content.into(),
        }
    }

    #[test]
    fn a_remembered_token_is_reused_only_under_the_same_binding() {
        let cache = cache();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind-1",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        assert_eq!(
            cache
                .lookup("login-a", "bind-1", now())
                .unwrap()
                .token
                .as_ref(),
            b"token-a"
        );
        // A switched account or login: a miss, and the entry is gone.
        assert!(cache.lookup("login-a", "bind-2", now()).is_none());
        assert!(cache.lookup("login-a", "bind-1", now()).is_none());
    }

    #[test]
    fn an_expiring_token_is_not_reused_and_is_removed() {
        let cache = cache();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(4),
            b"token-a",
        );
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(6),
            b"token-a",
        );
        assert!(cache.lookup("login-a", "bind", now()).is_some());
        assert!(
            cache
                .lookup("login-a", "bind", now() + chrono::Duration::minutes(2))
                .is_none()
        );
    }

    #[test]
    fn logins_are_separate_and_forget_removes_only_one() {
        let cache = cache();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        cache.forget("login-a");
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert_eq!(
            cache
                .lookup("login-b", "bind", now())
                .unwrap()
                .token
                .as_ref(),
            b"token-b"
        );
    }

    #[test]
    fn clear_removes_everything_and_drops_writes_started_before_it() {
        let cache = cache();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        let started = cache.epoch();
        cache.clear();
        assert!(cache.store.read().unwrap().is_none());
        cache.remember(
            started,
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        assert!(cache.store.read().unwrap().is_none());
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        assert!(cache.lookup("login-b", "bind", now()).is_some());
    }

    #[test]
    fn the_document_stays_within_the_store_bound_and_never_holds_a_refresh_token() {
        let cache = cache();
        let token = vec![b'x'; 200];
        for login in 0..20 {
            cache.remember(
                cache.epoch(),
                &format!("login-{login}"),
                &"b".repeat(64),
                &rev("rev"),
                later(60),
                &token,
            );
        }
        let stored = cache.store.read().unwrap().unwrap();
        assert!(stored.len() <= MAX_SECRET_BYTES);
        let doc: Document = serde_json::from_slice(&stored).unwrap();
        assert!(doc.entries.len() <= MAX_ENTRIES);
        assert_eq!(doc.entries.last().unwrap().login, "login-19");
        let text = String::from_utf8_lossy(&stored);
        assert!(!text.contains("refresh"));
        // A token that cannot fit on its own is simply not cached.
        let huge = vec![b'y'; MAX_SECRET_BYTES];
        cache.remember(
            cache.epoch(),
            "login-huge",
            "b",
            &rev("rev"),
            later(60),
            &huge,
        );
        assert!(cache.lookup("login-huge", "b", now()).is_none());
    }

    #[test]
    fn a_moved_item_revision_is_reported_not_discarded_and_confirm_records_it() {
        let cache = cache();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev-1"),
            later(60),
            b"token-a",
        );
        // The caller sees the revision the token was confirmed at; a lookup
        // never removes an entry because the item moved since.
        let cached = cache.lookup("login-a", "bind", now()).unwrap();
        assert_eq!(cached.revision, rev("rev-1"));
        assert_eq!(cached.token.as_ref(), b"token-a");
        cache.confirm(cache.epoch(), "login-a", "bind", &rev("rev-2"));
        let cached = cache.lookup("login-a", "bind", now()).unwrap();
        assert_eq!(cached.revision, rev("rev-2"));
        assert_eq!(cached.token.as_ref(), b"token-a");
        // Not under another binding, not for another item, and not after a
        // clear.
        cache.confirm(cache.epoch(), "login-a", "other", &rev("rev-3"));
        let recreated = ItemRevision {
            item: "recreated".into(),
            content: "rev-3".into(),
        };
        cache.confirm(cache.epoch(), "login-a", "bind", &recreated);
        assert_eq!(
            cache.lookup("login-a", "bind", now()).unwrap().revision,
            rev("rev-2")
        );
        let started = cache.epoch();
        cache.begin_clear();
        cache.confirm(started, "login-a", "bind", &rev("rev-4"));
        assert_eq!(
            cache.lookup("login-a", "bind", now()).unwrap().revision,
            rev("rev-2")
        );
    }

    /// The OS store a build without the keychain entitlement sees.
    #[derive(Default)]
    struct UnentitledStore {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl TokenStore for UnentitledStore {
        fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(UNENTITLED)
        }
        fn write(&self, _bytes: &[u8]) -> Result<(), &'static str> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(UNENTITLED)
        }
        fn clear(&self) -> Result<(), &'static str> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(UNENTITLED)
        }
    }

    #[test]
    fn an_unentitled_build_keeps_the_cache_in_memory_and_says_so() {
        let store = UnentitledStore::default();
        let calls = std::sync::Arc::clone(&store.calls);
        let cache = TokenCache::new(Box::new(store));
        assert!(!cache.unentitled());
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(cache.unentitled());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        assert_eq!(
            cache
                .lookup("login-a", "bind", now())
                .unwrap()
                .token
                .as_ref(),
            b"token-a"
        );
        // Once refused, the OS store is not asked again in this process.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.clear();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Refuses the first `refusals` operations as unentitled, then behaves
    /// as a working store.
    struct RecoveringStore {
        refusals: std::sync::Arc<Mutex<usize>>,
        inner: std::sync::Arc<MemoryStore>,
    }
    impl RecoveringStore {
        fn refused(&self) -> bool {
            let mut left = self.refusals.lock().unwrap();
            let refused = *left > 0;
            *left = left.saturating_sub(1);
            refused
        }
    }
    impl TokenStore for RecoveringStore {
        fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
            if self.refused() {
                return Err(UNENTITLED);
            }
            self.inner.read()
        }
        fn write(&self, bytes: &[u8]) -> Result<(), &'static str> {
            if self.refused() {
                return Err(UNENTITLED);
            }
            self.inner.write(bytes)
        }
        fn clear(&self) -> Result<(), &'static str> {
            if self.refused() {
                return Err(UNENTITLED);
            }
            self.inner.clear()
        }
    }

    #[test]
    fn an_unentitled_answer_is_asked_again_after_the_window() {
        let recovering = |refusals| RecoveringStore {
            refusals: std::sync::Arc::new(Mutex::new(refusals)),
            inner: std::sync::Arc::new(MemoryStore::default()),
        };
        // A transient refusal: once the window has passed, the OS store is
        // asked again, and its answer ends the in-memory mode.
        let cache = TokenCache::new(Box::new(recovering(1))).with_reprobe_after(Duration::ZERO);
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(cache.unentitled());
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        assert!(!cache.unentitled());
        assert!(cache.lookup("login-a", "bind", now()).is_some());
        // The in-memory copy was not left behind.
        assert!(cache.memory.read().unwrap().is_none());

        // Within the window the refusal stands and memory is used.
        let cache = TokenCache::new(Box::new(recovering(1)));
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        assert!(cache.unentitled());
        assert!(cache.lookup("login-a", "bind", now()).is_some());
    }

    /// A cache over a working store that can be made to refuse later, with
    /// handles to the refusal count and the store's own contents.
    fn interruptible() -> (
        TokenCache,
        std::sync::Arc<Mutex<usize>>,
        std::sync::Arc<MemoryStore>,
    ) {
        let refusals = std::sync::Arc::new(Mutex::new(0));
        let inner = std::sync::Arc::new(MemoryStore::default());
        let cache = TokenCache::new(Box::new(RecoveringStore {
            refusals: std::sync::Arc::clone(&refusals),
            inner: std::sync::Arc::clone(&inner),
        }))
        .with_reprobe_after(Duration::from_secs(600));
        (cache, refusals, inner)
    }
    fn stored_logins(inner: &MemoryStore) -> Vec<String> {
        inner
            .read()
            .unwrap()
            .map(|bytes| serde_json::from_slice::<Document>(&bytes).unwrap())
            .map(|doc| doc.entries.iter().map(|e| e.login.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_token_forgotten_during_the_memory_window_stays_forgotten_after_the_store_returns() {
        let (cache, refusals, inner) = interruptible();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        // The store refuses once: the cache moves to memory, where neither
        // token is.
        *refusals.lock().unwrap() = 1;
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(cache.unentitled());
        // Anthropic rejected login-a's token: forgotten while in memory.
        cache.forget("login-a");
        // The store answers again: login-a must not come back from it.
        cache.expire_window();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(!cache.unentitled());
        assert_eq!(stored_logins(&inner), ["login-b"]);
        assert!(cache.lookup("login-b", "bind", now()).is_some());
    }

    /// Answers reads, and refuses writes while `refuse_writes` is set.
    struct WriteRefusingStore {
        refuse_writes: std::sync::Arc<std::sync::atomic::AtomicBool>,
        inner: std::sync::Arc<MemoryStore>,
    }
    impl TokenStore for WriteRefusingStore {
        fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
            self.inner.read()
        }
        fn write(&self, bytes: &[u8]) -> Result<(), &'static str> {
            if self.refuse_writes.load(Ordering::SeqCst) {
                return Err(UNENTITLED);
            }
            self.inner.write(bytes)
        }
        fn clear(&self) -> Result<(), &'static str> {
            if self.refuse_writes.load(Ordering::SeqCst) {
                return Err(UNENTITLED);
            }
            self.inner.clear()
        }
    }

    #[test]
    fn a_forget_whose_write_is_the_first_refusal_stays_forgotten() {
        let refuse_writes = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let inner = std::sync::Arc::new(MemoryStore::default());
        let cache = TokenCache::new(Box::new(WriteRefusingStore {
            refuse_writes: std::sync::Arc::clone(&refuse_writes),
            inner: std::sync::Arc::clone(&inner),
        }))
        .with_reprobe_after(Duration::from_secs(600));
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        // The store still reads, then refuses the forget's write.
        refuse_writes.store(true, Ordering::SeqCst);
        cache.forget("login-a");
        assert!(cache.unentitled());
        refuse_writes.store(false, Ordering::SeqCst);
        cache.expire_window();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert_eq!(stored_logins(&inner), ["login-b"]);
    }

    #[test]
    fn a_clear_during_the_memory_window_reaches_the_store_when_it_returns() {
        let (cache, refusals, inner) = interruptible();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        *refusals.lock().unwrap() = 1;
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        // Unenroll, or the Claude switch turned off, while in memory.
        cache.clear();
        cache.expire_window();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(!cache.unentitled());
        assert!(inner.read().unwrap().is_none());

        // A token remembered after that clear is kept, and the cleared
        // ones still do not return.
        let (cache, refusals, inner) = interruptible();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        *refusals.lock().unwrap() = 1;
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.clear();
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        cache.expire_window();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert!(cache.lookup("login-b", "bind", now()).is_some());
        assert_eq!(stored_logins(&inner), ["login-b"]);
    }

    #[test]
    fn a_token_read_during_the_memory_window_is_carried_into_the_store() {
        let (cache, refusals, inner) = interruptible();
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        *refusals.lock().unwrap() = 1;
        assert!(cache.lookup("login-b", "bind", now()).is_none());
        // A pressed Refresh read login-b's token while in memory.
        cache.remember(
            cache.epoch(),
            "login-b",
            "bind",
            &rev("rev"),
            later(60),
            b"token-b",
        );
        cache.expire_window();
        assert_eq!(
            cache
                .lookup("login-b", "bind", now())
                .unwrap()
                .token
                .as_ref(),
            b"token-b"
        );
        assert!(cache.lookup("login-a", "bind", now()).is_some());
        let mut logins = stored_logins(&inner);
        logins.sort();
        assert_eq!(logins, ["login-a", "login-b"]);
        assert!(cache.memory.read().unwrap().is_none());
    }

    /// Blocks in `read` until released, to stand in for a stalled Keychain.
    struct StalledStore {
        entered: std::sync::mpsc::SyncSender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl TokenStore for StalledStore {
        fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
            Ok(None)
        }
        fn write(&self, _bytes: &[u8]) -> Result<(), &'static str> {
            Ok(())
        }
        fn clear(&self) -> Result<(), &'static str> {
            Ok(())
        }
    }

    #[test]
    fn asking_whether_the_build_is_unentitled_never_waits_on_the_store() {
        let (entered_tx, entered) = std::sync::mpsc::sync_channel(1);
        let (release, release_rx) = std::sync::mpsc::channel();
        let cache = std::sync::Arc::new(TokenCache::new(Box::new(StalledStore {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        })));
        let reader = std::sync::Arc::clone(&cache);
        let stalled = std::thread::spawn(move || {
            reader.lookup("login-a", "bind", now());
        });
        entered.recv().unwrap();
        let asker = std::sync::Arc::clone(&cache);
        let (answer_tx, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = answer_tx.send(asker.unentitled());
        });
        let answered = answer.recv_timeout(Duration::from_secs(5));
        release.send(()).unwrap();
        stalled.join().unwrap();
        assert_eq!(answered, Ok(false));
    }

    #[test]
    fn an_unanswered_check_is_remembered_until_the_item_moves_or_the_entry_changes() {
        let cache = cache();
        cache.note_unanswered("login-a", &rev("rev-2"));
        assert!(cache.unanswered("login-a", &rev("rev-2")));
        assert!(!cache.unanswered("login-a", &rev("rev-3")));
        assert!(!cache.unanswered("login-b", &rev("rev-2")));
        cache.forget("login-a");
        assert!(!cache.unanswered("login-a", &rev("rev-2")));
        cache.note_unanswered("login-a", &rev("rev-2"));
        cache.clear();
        assert!(!cache.unanswered("login-a", &rev("rev-2")));
    }

    #[test]
    fn an_unreadable_document_is_a_miss_not_an_error() {
        let cache = cache();
        cache.store.write(b"not json").unwrap();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.remember(
            cache.epoch(),
            "login-a",
            "bind",
            &rev("rev"),
            later(60),
            b"token-a",
        );
        assert!(cache.lookup("login-a", "bind", now()).is_some());
    }
}
