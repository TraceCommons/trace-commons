//! The daemon's own copy of a Claude Code usage access token.
//!
//! Reading Claude Code's Keychain item can show a macOS dialog, and Claude
//! Code rotating that item resets any "Always Allow" grant. After one
//! successful read, the access token, its expiry and a binding to the exact
//! credential it came from are kept in a Keychain item this app owns (the
//! data-protection store under its access group, which never prompts the
//! signed app). Later Refreshes reuse it until it expires, Anthropic rejects
//! it, or the binding no longer matches.
//!
//! The refresh token is never stored or used, and tokens are never renewed
//! here: renewing would rotate Claude Code's own credentials.
//!
//! One fixed entry holds every cached login, so unenroll and the Claude
//! switch clear it with a single delete, including logins that have since
//! gone away.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_contributor_platform::credential_store::MAX_SECRET_BYTES;
use trace_commons_contributor_platform::external_claude_credentials::{SecretBytes, wipe};

/// A cached token is not used within this margin of its expiry.
const EXPIRY_MARGIN_SECONDS: i64 = 5 * 60;
/// A bounded number of logins, oldest dropped first.
const MAX_ENTRIES: usize = 8;
const VERSION: u8 = 1;

/// Where the cache document lives. Production is the OS credential store;
/// test builds use memory only.
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
    /// service, item revision, local account evidence and login spec.
    binding: String,
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

pub(super) struct TokenCache {
    store: Box<dyn TokenStore>,
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
            lock: Mutex::new(()),
            epoch: AtomicU64::new(0),
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
        let empty = || Document {
            v: VERSION,
            entries: Vec::new(),
        };
        match self.store.read() {
            Ok(Some(bytes)) => serde_json::from_slice::<Document>(&bytes)
                .ok()
                .filter(|doc| doc.v == VERSION)
                .unwrap_or_else(empty),
            _ => empty(),
        }
    }

    fn save(&self, document: &Document) {
        if document.entries.is_empty() {
            let _ = self.store.clear();
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(document) {
            let bytes = SecretBytes::new(bytes);
            let _ = self.store.write(&bytes);
        }
    }

    /// The cached token for `login`, if it was read from the credential
    /// `binding` names and is not within the margin of expiry. A stale or
    /// mismatched entry is removed.
    pub(super) fn lookup(
        &self,
        login: &str,
        binding: &str,
        now: DateTime<Utc>,
    ) -> Option<SecretBytes> {
        let _guard = self.lock.lock().ok()?;
        let mut document = self.document();
        let index = document.entries.iter().position(|e| e.login == login)?;
        let entry = &document.entries[index];
        let usable = entry.binding == binding
            && entry.expires_at - EXPIRY_MARGIN_SECONDS * 1000 > now.timestamp_millis();
        if usable {
            return Some(SecretBytes::new(entry.token.as_bytes().to_vec()));
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
        expires_at: i64,
        token: &[u8],
    ) {
        let Ok(token) = std::str::from_utf8(token) else {
            return;
        };
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
            expires_at,
            token: token.into(),
        });
        while document.entries.len() > MAX_ENTRIES {
            document.entries.remove(0);
        }
        // Drop the oldest logins until the document fits; never the new one.
        while serde_json::to_vec(&document).map_or(usize::MAX, |b| SecretBytes::new(b).len())
            > MAX_SECRET_BYTES
        {
            if document.entries.len() <= 1 {
                return;
            }
            document.entries.remove(0);
        }
        self.save(&document);
    }

    /// Remove `login`'s entry: Anthropic rejected it, or the credential it
    /// was read from changed during the Refresh.
    pub(super) fn forget(&self, login: &str) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        let mut document = self.document();
        let before = document.entries.len();
        document.entries.retain(|e| e.login != login);
        if document.entries.len() != before {
            self.save(&document);
        }
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
        let _ = self.store.clear();
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
impl OsStore {
    fn backend() -> Result<super::super::os_secret_store::OsSecretBackend, &'static str> {
        super::super::os_secret_store::OsSecretBackend::claude_usage_cache(
            &crate::channel::KEYCHAIN,
        )
        .map_err(|_| "unavailable")
    }
}

#[cfg(target_os = "macos")]
impl TokenStore for OsStore {
    fn read(&self) -> Result<Option<SecretBytes>, &'static str> {
        use super::super::credential_store::CredentialError;
        match Self::backend()?.read_claude_usage_cache() {
            Ok(bytes) => Ok(Some(SecretBytes::new(bytes))),
            Err(CredentialError::NoEntry) => Ok(None),
            Err(_) => Err("unavailable"),
        }
    }
    fn write(&self, bytes: &[u8]) -> Result<(), &'static str> {
        Self::backend()?
            .write_claude_usage_cache(bytes)
            .map_err(|_| "unavailable")
    }
    fn clear(&self) -> Result<(), &'static str> {
        use super::super::credential_store::CredentialError;
        match Self::backend()?.delete_claude_usage_cache() {
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(_) => Err("unavailable"),
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

    #[test]
    fn a_remembered_token_is_reused_only_under_the_same_binding() {
        let cache = cache();
        cache.remember(cache.epoch(), "login-a", "bind-1", later(60), b"token-a");
        assert_eq!(
            cache.lookup("login-a", "bind-1", now()).unwrap().as_ref(),
            b"token-a"
        );
        // A rotated item or switched account: a miss, and the entry is gone.
        assert!(cache.lookup("login-a", "bind-2", now()).is_none());
        assert!(cache.lookup("login-a", "bind-1", now()).is_none());
    }

    #[test]
    fn an_expiring_token_is_not_reused_and_is_removed() {
        let cache = cache();
        cache.remember(cache.epoch(), "login-a", "bind", later(4), b"token-a");
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.remember(cache.epoch(), "login-a", "bind", later(6), b"token-a");
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
        cache.remember(cache.epoch(), "login-a", "bind", later(60), b"token-a");
        cache.remember(cache.epoch(), "login-b", "bind", later(60), b"token-b");
        cache.forget("login-a");
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        assert_eq!(
            cache.lookup("login-b", "bind", now()).unwrap().as_ref(),
            b"token-b"
        );
    }

    #[test]
    fn clear_removes_everything_and_drops_writes_started_before_it() {
        let cache = cache();
        cache.remember(cache.epoch(), "login-a", "bind", later(60), b"token-a");
        let started = cache.epoch();
        cache.clear();
        assert!(cache.store.read().unwrap().is_none());
        cache.remember(started, "login-b", "bind", later(60), b"token-b");
        assert!(cache.store.read().unwrap().is_none());
        cache.remember(cache.epoch(), "login-b", "bind", later(60), b"token-b");
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
        cache.remember(cache.epoch(), "login-huge", "b", later(60), &huge);
        assert!(cache.lookup("login-huge", "b", now()).is_none());
    }

    #[test]
    fn an_unreadable_document_is_a_miss_not_an_error() {
        let cache = cache();
        cache.store.write(b"not json").unwrap();
        assert!(cache.lookup("login-a", "bind", now()).is_none());
        cache.remember(cache.epoch(), "login-a", "bind", later(60), b"token-a");
        assert!(cache.lookup("login-a", "bind", now()).is_some());
    }
}
