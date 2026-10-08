//! Counter rows for on-device token analytics: one [`TurnRecord`] per API
//! response and one [`ToolCallRecord`] per tool call, plus the keyed digests
//! that let analytics recognise a repeated message, file or argument set
//! without storing it.
//!
//! Nothing here reads files, keys or clocks. A host extracts the rows from
//! bytes the user selected and supplies the digest key through
//! [`DigestKeyStore`]; the rows themselves hold no bodies, paths, command
//! text, or raw message or session IDs.
//!
//! An unknown counter is `None`, never zero. A turn with any of its four
//! counters unknown is `usage_unknown` and contributes to no token figure.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// Turns kept per session. Past this the series is `truncated`, which counts
/// as partial coverage; nothing is cut silently.
pub const MAX_SERIES_TURNS: usize = 8_192;
/// Tool calls kept per session, with the same `truncated` rule.
pub const MAX_SERIES_TOOL_CALLS: usize = 32_768;
/// Longest file extension kept on a tool call. A longer one is dropped whole,
/// never cut, so no partial name survives.
pub const MAX_PATH_EXT_CHARS: usize = 8;

/// How a turn's cache write was reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheWrite {
    /// Claude reports the 5-minute and 1-hour writes separately.
    Split { m5: u32, h1: u32 },
    /// One total with no duration, as the proxy ledger records it. Known for
    /// token sums and cache share; its cache lifetime is unknown.
    TotalOnly(u32),
}

impl CacheWrite {
    pub fn total(self) -> u64 {
        match self {
            Self::Split { m5, h1 } => u64::from(m5) + u64::from(h1),
            Self::TotalOnly(total) => u64::from(total),
        }
    }

    /// Whether part of this write was declared to live for an hour. A
    /// `TotalOnly` write has no stated duration, so it is not.
    pub fn has_one_hour_write(self) -> bool {
        matches!(self, Self::Split { h1, .. } if h1 > 0)
    }
}

/// One API response's counters. `usage_unknown` is derived, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRecord {
    pub ordinal: u32,
    pub at: Option<DateTime<Utc>>,
    pub uncached_input: Option<u32>,
    pub cache_read: Option<u32>,
    pub cache_write: Option<CacheWrite>,
    pub output: Option<u32>,
    /// Index into the session's declared model labels; `None` is the
    /// "unknown label" bucket.
    pub model_label_ix: Option<u16>,
    /// Seconds since the previous turn. `None` when either side is undated.
    pub gap_secs: Option<u32>,
    /// Keyed digest of the provider's message ID, for counting a turn once
    /// across reimported snapshots. Never the ID itself.
    pub msg_key: Option<KeyedDigest>,
}

/// A turn whose four counters are all known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownTurnUsage {
    pub uncached_input: u32,
    pub cache_read: u32,
    pub cache_write: CacheWrite,
    pub output: u32,
}

impl KnownTurnUsage {
    /// Input sent this turn: uncached + cache read + cache write.
    pub fn context(&self) -> u64 {
        u64::from(self.uncached_input) + u64::from(self.cache_read) + self.cache_write.total()
    }

    /// Every token this turn: context + output.
    pub fn total(&self) -> u64 {
        self.context() + u64::from(self.output)
    }
}

impl TurnRecord {
    pub fn known_usage(&self) -> Option<KnownTurnUsage> {
        Some(KnownTurnUsage {
            uncached_input: self.uncached_input?,
            cache_read: self.cache_read?,
            cache_write: self.cache_write?,
            output: self.output?,
        })
    }

    pub fn usage_unknown(&self) -> bool {
        self.known_usage().is_none()
    }

    pub fn context(&self) -> Option<u64> {
        self.known_usage().map(|usage| usage.context())
    }
}

/// Seconds from `earlier` to `later`, or `None` when either is undated or the
/// clock ran backwards.
pub fn gap_between(earlier: Option<DateTime<Utc>>, later: Option<DateTime<Utc>>) -> Option<u32> {
    let seconds = (later? - earlier?).num_seconds();
    u32::try_from(seconds).ok()
}

/// The closed tool allow-list. Any other name is `Other`; names are matched
/// exactly as the harness spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Write,
    MultiEdit,
    NotebookEdit,
    Bash,
    Grep,
    Glob,
    Other,
}

impl ToolKind {
    pub fn from_tool_name(name: &str) -> Self {
        match name {
            "Read" => Self::Read,
            "Edit" => Self::Edit,
            "Write" => Self::Write,
            "MultiEdit" => Self::MultiEdit,
            "NotebookEdit" => Self::NotebookEdit,
            "Bash" => Self::Bash,
            "Grep" => Self::Grep,
            "Glob" => Self::Glob,
            _ => Self::Other,
        }
    }

    fn domain_byte(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::Edit => 2,
            Self::Write => 3,
            Self::MultiEdit => 4,
            Self::NotebookEdit => 5,
            Self::Bash => 6,
            Self::Grep => 7,
            Self::Glob => 8,
            Self::Other => 9,
        }
    }
}

/// One tool call, reduced to keyed digests and sizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRecord {
    pub turn_ordinal: u32,
    pub tool: ToolKind,
    pub args_key: KeyedDigest,
    pub path_key: Option<KeyedDigest>,
    pub path_ext: Option<String>,
    pub result_bytes: Option<u32>,
    pub success: Option<bool>,
    /// Whether a result was found for the call.
    pub paired: bool,
}

/// A session's rows, bounded by the caps above.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSeries {
    pub turns: Vec<TurnRecord>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeriesError {
    #[error("turn ordinals are not strictly increasing")]
    TurnOrder,
    #[error("a tool call names a missing turn or is out of turn order")]
    ToolCallTurn,
    #[error("a file extension is outside its bound")]
    PathExt,
    #[error("a gap is recorded without both turn dates")]
    Gap,
    #[error("the series exceeds its row caps")]
    TooManyRows,
}

impl UsageSeries {
    pub fn push_turn(&mut self, turn: TurnRecord) {
        if self.turns.len() >= MAX_SERIES_TURNS {
            self.truncated = true;
        } else {
            self.turns.push(turn);
        }
    }

    pub fn push_tool_call(&mut self, call: ToolCallRecord) {
        if self.tool_calls.len() >= MAX_SERIES_TOOL_CALLS {
            self.truncated = true;
        } else {
            self.tool_calls.push(call);
        }
    }

    pub fn validate(&self) -> Result<(), SeriesError> {
        if self.turns.len() > MAX_SERIES_TURNS || self.tool_calls.len() > MAX_SERIES_TOOL_CALLS {
            return Err(SeriesError::TooManyRows);
        }
        let mut previous: Option<&TurnRecord> = None;
        for turn in &self.turns {
            if let Some(previous) = previous {
                if turn.ordinal <= previous.ordinal {
                    return Err(SeriesError::TurnOrder);
                }
            }
            if turn.gap_secs.is_some()
                && (turn.at.is_none() || previous.is_none_or(|previous| previous.at.is_none()))
            {
                return Err(SeriesError::Gap);
            }
            previous = Some(turn);
        }
        let mut last_turn = None;
        for call in &self.tool_calls {
            if self
                .turns
                .binary_search_by_key(&call.turn_ordinal, |turn| turn.ordinal)
                .is_err()
                || last_turn.is_some_and(|last| call.turn_ordinal < last)
            {
                return Err(SeriesError::ToolCallTurn);
            }
            last_turn = Some(call.turn_ordinal);
            if let Some(ext) = &call.path_ext {
                if !is_valid_ext(ext) {
                    return Err(SeriesError::PathExt);
                }
            }
        }
        Ok(())
    }
}

/// An HMAC-SHA256 output under the host's digest key. Serialized as
/// lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyedDigest(pub [u8; 32]);

impl std::fmt::Debug for KeyedDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyedDigest({})", hex::encode(&self.0[..4]))
    }
}

impl Serialize for KeyedDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for KeyedDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&text, &mut bytes).map_err(serde::de::Error::custom)?;
        Ok(Self(bytes))
    }
}

/// The secret behind every [`KeyedDigest`]. Its `Debug` never prints it.
#[derive(Clone)]
pub struct DigestKey([u8; 32]);

impl DigestKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for DigestKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DigestKey(redacted)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DigestKeyError {
    /// A safe, label-only name for the control that failed.
    #[error("digest key store unavailable: {0}")]
    Unavailable(&'static str),
}

/// Custody of the digest key (owner decision D16, open; the recommended
/// backend is the OS keychain). The backend is the host's: this contract only
/// says that a key, once created, is returned unchanged until cleared.
pub trait DigestKeyStore {
    /// The stored key, or `None` when none exists yet. Never creates one.
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError>;
    /// The stored key, creating and storing one first when none exists.
    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError>;
    /// Forget the key. Every digest made under it stops matching new ones.
    fn clear(&self) -> Result<(), DigestKeyError>;
}

/// A key store that lives only in this process. For tests and previews; it
/// never touches a keychain or a file. The key it creates is its seed.
#[derive(Debug)]
pub struct InMemoryDigestKeyStore {
    seed: [u8; 32],
    key: Mutex<Option<[u8; 32]>>,
}

impl InMemoryDigestKeyStore {
    pub fn with_seed(seed: [u8; 32]) -> Self {
        Self {
            seed,
            key: Mutex::new(None),
        }
    }

    fn slot(&self) -> Result<std::sync::MutexGuard<'_, Option<[u8; 32]>>, DigestKeyError> {
        self.key
            .lock()
            .map_err(|_| DigestKeyError::Unavailable("in_memory_digest_key"))
    }
}

impl DigestKeyStore for InMemoryDigestKeyStore {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        Ok(self.slot()?.map(DigestKey))
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        let mut slot = self.slot()?;
        Ok(DigestKey(*slot.get_or_insert(self.seed)))
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        *self.slot()? = None;
        Ok(())
    }
}

const DOMAIN_MSG: &[u8] = b"trace-commons/insights/msg_key/v1\0";
const DOMAIN_ARGS: &[u8] = b"trace-commons/insights/args_key/v1\0";
const DOMAIN_PATH: &[u8] = b"trace-commons/insights/path_key/v1\0";
const DOMAIN_FINGERPRINT: &[u8] = b"trace-commons/insights/key_fingerprint/v1\0";
const DOMAIN_SESSION: &[u8] = b"trace-commons/insights/session_key/v1\0";
const DOMAIN_PROJECT: &[u8] = b"trace-commons/insights/project_digest/v1\0";

/// A keyed digest that names `key` without revealing it. Stored beside rows
/// so a reader can tell digests made under another key (a new device, a
/// cleared keychain) from digests of different messages, files or arguments.
pub fn key_fingerprint(key: &DigestKey) -> KeyedDigest {
    keyed(key, DOMAIN_FINGERPRINT, &[])
}

/// Keyed digest of a provider message ID.
pub fn msg_key(key: &DigestKey, message_id: &str) -> KeyedDigest {
    keyed(key, DOMAIN_MSG, &[message_id.as_bytes()])
}

/// Keyed digest of a tool name and its canonical JSON input.
pub fn args_key(
    key: &DigestKey,
    tool: ToolKind,
    input: &serde_json::Value,
) -> Result<KeyedDigest, serde_json::Error> {
    let canonical = crate::canonical_json::to_canonical_vec(input)?;
    Ok(keyed(
        key,
        DOMAIN_ARGS,
        &[&[tool.domain_byte()], &canonical],
    ))
}

/// Keyed digest of a file path after [`normalize_tool_path`].
pub fn path_key(key: &DigestKey, path: &str) -> KeyedDigest {
    keyed(key, DOMAIN_PATH, &[normalize_tool_path(path).as_bytes()])
}

/// Keyed digest naming one watched session: the harness that found it and
/// the address it was found at. A host keys its stored rows by this, so the
/// same session seen again replaces its row and the address is never stored.
/// The harness name is length-prefixed, so the two parts cannot run together.
pub fn session_key(key: &DigestKey, source: &str, address: &str) -> KeyedDigest {
    let length = (source.len() as u64).to_be_bytes();
    keyed(
        key,
        DOMAIN_SESSION,
        &[&length, source.as_bytes(), address.as_bytes()],
    )
}

/// Keyed digest of a project key (a normalized working directory). Stored in
/// place of the key itself; a display label is resolved from the watcher at
/// view time, never stored.
pub fn project_digest(key: &DigestKey, project_key: &str) -> KeyedDigest {
    keyed(key, DOMAIN_PROJECT, &[project_key.as_bytes()])
}

fn keyed(key: &DigestKey, domain: &[u8], parts: &[&[u8]]) -> KeyedDigest {
    let mut message = domain.to_vec();
    for part in parts {
        message.extend_from_slice(part);
    }
    KeyedDigest(hmac_sha256(&key.0, &message))
}

/// Lexical path normalization with no I/O: backslashes become `/`, empty and
/// `.` segments are dropped, a trailing `/` is dropped, and a leading `/` is
/// kept. Case is kept; `..` is kept as written.
pub fn normalize_tool_path(path: &str) -> String {
    let unified = path.replace('\\', "/");
    let absolute = unified.starts_with('/');
    let joined = unified
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// The lowercase extension of the last path segment, when it is 1 to
/// [`MAX_PATH_EXT_CHARS`] ASCII letters or digits and the name has a stem.
pub fn path_ext(path: &str) -> Option<String> {
    let normalized = normalize_tool_path(path);
    let name = normalized.rsplit('/').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let ext = ext.to_ascii_lowercase();
    is_valid_ext(&ext).then_some(ext)
}

fn is_valid_ext(ext: &str) -> bool {
    !ext.is_empty()
        && ext.len() <= MAX_PATH_EXT_CHARS
        && ext
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// HMAC-SHA256 (RFC 2104) over `sha2`, pinned by RFC 4231 vectors below.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block_key = [0u8; BLOCK];
    if key.len() > BLOCK {
        block_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block_key.map(|byte| byte ^ 0x36));
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block_key.map(|byte| byte ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn key(byte: u8) -> DigestKey {
        DigestKey::from_bytes([byte; 32])
    }

    fn turn(ordinal: u32) -> TurnRecord {
        TurnRecord {
            ordinal,
            at: None,
            uncached_input: Some(10),
            cache_read: Some(20),
            cache_write: Some(CacheWrite::Split { m5: 3, h1: 4 }),
            output: Some(5),
            model_label_ix: None,
            gap_secs: None,
            msg_key: None,
        }
    }

    fn read_call(turn_ordinal: u32) -> ToolCallRecord {
        ToolCallRecord {
            turn_ordinal,
            tool: ToolKind::Read,
            args_key: KeyedDigest([1; 32]),
            path_key: None,
            path_ext: None,
            result_bytes: Some(400),
            success: Some(true),
            paired: true,
        }
    }

    // RFC 4231 test case 1.
    #[test]
    fn hmac_sha256_matches_rfc_4231_case_1() {
        let mac = hmac_sha256(&[0x0b; 20], b"Hi There");
        assert_eq!(
            hex::encode(mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    // RFC 4231 test case 2: a key shorter than the block.
    #[test]
    fn hmac_sha256_matches_rfc_4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // RFC 4231 test case 6: a key longer than the block is hashed first.
    #[test]
    fn hmac_sha256_matches_rfc_4231_case_6() {
        let mac = hmac_sha256(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex::encode(mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn digest_key_debug_never_prints_the_key() {
        let rendered = format!("{:?}", key(0xab));
        assert!(!rendered.contains("ab"), "{rendered}");
        assert!(!rendered.contains("171"), "{rendered}");
    }

    #[test]
    fn the_three_digests_are_domain_separated() {
        let key = key(7);
        let msg = msg_key(&key, "same");
        let path = path_key(&key, "same");
        let args = args_key(&key, ToolKind::Other, &json!("same")).unwrap();
        assert_ne!(msg, path);
        assert_ne!(msg, args);
        assert_ne!(path, args);
    }

    /// A watched session and a watched folder are named by keyed digests in
    /// their own domains: never equal to a message, path or argument digest of
    /// the same text, and the session digest depends on the harness too.
    #[test]
    fn session_and_project_digests_are_domain_separated() {
        let key8 = || key(8);
        let key = key(7);
        let session = session_key(&key, "claude-code", "same");
        let project = project_digest(&key, "same");
        for other in [
            msg_key(&key, "same"),
            path_key(&key, "same"),
            args_key(&key, ToolKind::Other, &json!("same")).unwrap(),
            key_fingerprint(&key),
        ] {
            assert_ne!(session, other);
            assert_ne!(project, other);
        }
        assert_ne!(session, project);
        assert_ne!(session, session_key(&key, "codex", "same"));
        // The harness and the address cannot run into each other.
        assert_ne!(session_key(&key, "ab", "c"), session_key(&key, "a", "bc"));
        assert_eq!(session, session_key(&key, "claude-code", "same"));
        assert_ne!(session, session_key(&key8(), "claude-code", "same"));
        assert_ne!(project, project_digest(&key8(), "same"));
    }

    /// Rows made under different keys never match. The fingerprint lets a
    /// reader tell that apart from "different messages" without the key.
    #[test]
    fn key_fingerprint_names_the_key_without_revealing_it() {
        assert_eq!(key_fingerprint(&key(1)), key_fingerprint(&key(1)));
        assert_ne!(key_fingerprint(&key(1)), key_fingerprint(&key(2)));
        assert_ne!(key_fingerprint(&key(1)), msg_key(&key(1), ""));
        assert_ne!(key_fingerprint(&key(1)), path_key(&key(1), ""));
        assert_ne!(key_fingerprint(&key(1)).0, [1u8; 32]);
    }

    #[test]
    fn digests_depend_on_the_key() {
        assert_ne!(msg_key(&key(1), "m"), msg_key(&key(2), "m"));
        assert_eq!(msg_key(&key(1), "m"), msg_key(&key(1), "m"));
    }

    #[test]
    fn args_key_ignores_key_order() {
        let key = key(3);
        let one: serde_json::Value =
            serde_json::from_str(r#"{"file_path":"a.rs","limit":5}"#).unwrap();
        let two: serde_json::Value =
            serde_json::from_str(r#"{"limit":5,"file_path":"a.rs"}"#).unwrap();
        assert_eq!(
            args_key(&key, ToolKind::Read, &one).unwrap(),
            args_key(&key, ToolKind::Read, &two).unwrap()
        );
    }

    #[test]
    fn args_key_depends_on_the_tool() {
        let key = key(3);
        let input = json!({"pattern": "x"});
        assert_ne!(
            args_key(&key, ToolKind::Grep, &input).unwrap(),
            args_key(&key, ToolKind::Glob, &input).unwrap()
        );
    }

    #[test]
    fn path_key_uses_the_normalized_path() {
        let key = key(4);
        assert_eq!(
            path_key(&key, "src//lib.rs"),
            path_key(&key, "./src/lib.rs")
        );
        assert_eq!(path_key(&key, "src\\lib.rs"), path_key(&key, "src/lib.rs"));
        assert_ne!(path_key(&key, "src/lib.rs"), path_key(&key, "src/Lib.rs"));
    }

    #[test]
    fn normalize_tool_path_rules() {
        assert_eq!(normalize_tool_path("a//b/./c/"), "a/b/c");
        assert_eq!(normalize_tool_path("/abs//x"), "/abs/x");
        assert_eq!(normalize_tool_path("C:\\x\\y"), "C:/x/y");
        assert_eq!(normalize_tool_path("./"), "");
    }

    #[test]
    fn path_ext_keeps_a_short_ascii_extension_only() {
        assert_eq!(path_ext("src/lib.RS").as_deref(), Some("rs"));
        assert_eq!(path_ext("a/b.tar.gz").as_deref(), Some("gz"));
        assert_eq!(path_ext("Makefile"), None);
        assert_eq!(path_ext(".bashrc"), None);
        assert_eq!(path_ext("dir.d/file"), None);
        // Longer than the bound: dropped, not cut, so no partial name leaks.
        assert_eq!(path_ext("x.abcdefghi"), None);
        assert_eq!(path_ext("x.abcdefgh").as_deref(), Some("abcdefgh"));
        assert_eq!(path_ext("x.r s"), None);
        assert_eq!(path_ext("x."), None);
    }

    #[test]
    fn tool_kind_is_a_closed_allow_list() {
        assert_eq!(ToolKind::from_tool_name("Read"), ToolKind::Read);
        assert_eq!(ToolKind::from_tool_name("MultiEdit"), ToolKind::MultiEdit);
        assert_eq!(
            ToolKind::from_tool_name("NotebookEdit"),
            ToolKind::NotebookEdit
        );
        assert_eq!(ToolKind::from_tool_name("read"), ToolKind::Other);
        assert_eq!(ToolKind::from_tool_name("WebFetch"), ToolKind::Other);
    }

    #[test]
    fn cache_write_total_sums_the_split_without_overflow() {
        let split = CacheWrite::Split {
            m5: u32::MAX,
            h1: u32::MAX,
        };
        assert_eq!(split.total(), 2 * u64::from(u32::MAX));
        assert_eq!(CacheWrite::TotalOnly(9).total(), 9);
        assert!(!CacheWrite::TotalOnly(9).has_one_hour_write());
        assert!(CacheWrite::Split { m5: 0, h1: 1 }.has_one_hour_write());
        assert!(!CacheWrite::Split { m5: 1, h1: 0 }.has_one_hour_write());
    }

    #[test]
    fn a_turn_with_every_counter_known_has_usage() {
        let usage = turn(0).known_usage().unwrap();
        assert_eq!(usage.context(), 10 + 20 + 7);
        assert_eq!(usage.total(), 10 + 20 + 7 + 5);
        assert!(!turn(0).usage_unknown());
    }

    #[test]
    fn any_unknown_counter_makes_the_turn_unknown_never_zero() {
        let mut cases = Vec::new();
        let mut a = turn(0);
        a.uncached_input = None;
        cases.push(a);
        let mut b = turn(0);
        b.cache_read = None;
        cases.push(b);
        let mut c = turn(0);
        c.cache_write = None;
        cases.push(c);
        let mut d = turn(0);
        d.output = None;
        cases.push(d);
        for case in cases {
            assert!(case.usage_unknown());
            assert_eq!(case.known_usage(), None);
        }
    }

    #[test]
    fn context_does_not_overflow_u32() {
        let mut big = turn(0);
        big.uncached_input = Some(u32::MAX);
        big.cache_read = Some(u32::MAX);
        big.cache_write = Some(CacheWrite::TotalOnly(u32::MAX));
        big.output = Some(u32::MAX);
        let usage = big.known_usage().unwrap();
        assert_eq!(usage.context(), 3 * u64::from(u32::MAX));
        assert_eq!(usage.total(), 4 * u64::from(u32::MAX));
    }

    #[test]
    fn series_marks_itself_truncated_past_the_turn_cap() {
        let mut series = UsageSeries::default();
        for ordinal in 0..MAX_SERIES_TURNS as u32 {
            series.push_turn(turn(ordinal));
        }
        assert!(!series.truncated);
        series.push_turn(turn(MAX_SERIES_TURNS as u32));
        assert!(series.truncated);
        assert_eq!(series.turns.len(), MAX_SERIES_TURNS);
    }

    #[test]
    fn series_marks_itself_truncated_past_the_tool_call_cap() {
        let mut series = UsageSeries::default();
        series.push_turn(turn(0));
        for _ in 0..MAX_SERIES_TOOL_CALLS {
            series.push_tool_call(read_call(0));
        }
        assert!(!series.truncated);
        series.push_tool_call(read_call(0));
        assert!(series.truncated);
        assert_eq!(series.tool_calls.len(), MAX_SERIES_TOOL_CALLS);
    }

    #[test]
    fn caps_match_the_spec() {
        assert_eq!(MAX_SERIES_TURNS, 8_192);
        assert_eq!(MAX_SERIES_TOOL_CALLS, 32_768);
        assert_eq!(MAX_PATH_EXT_CHARS, 8);
    }

    #[test]
    fn validate_accepts_a_well_formed_series() {
        let mut series = UsageSeries::default();
        series.push_turn(turn(0));
        series.push_turn(turn(1));
        series.push_tool_call(read_call(1));
        assert_eq!(series.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_out_of_order_turns() {
        let series = UsageSeries {
            turns: vec![turn(1), turn(1)],
            tool_calls: vec![],
            truncated: false,
        };
        assert_eq!(series.validate(), Err(SeriesError::TurnOrder));
    }

    #[test]
    fn validate_rejects_a_tool_call_for_a_missing_turn() {
        let series = UsageSeries {
            turns: vec![turn(0)],
            tool_calls: vec![read_call(3)],
            truncated: false,
        };
        assert_eq!(series.validate(), Err(SeriesError::ToolCallTurn));
    }

    #[test]
    fn validate_rejects_tool_calls_out_of_turn_order() {
        let series = UsageSeries {
            turns: vec![turn(0), turn(1)],
            tool_calls: vec![read_call(1), read_call(0)],
            truncated: false,
        };
        assert_eq!(series.validate(), Err(SeriesError::ToolCallTurn));
    }

    #[test]
    fn validate_rejects_a_bad_extension() {
        let mut call = read_call(0);
        call.path_ext = Some("../etc".into());
        let series = UsageSeries {
            turns: vec![turn(0)],
            tool_calls: vec![call],
            truncated: false,
        };
        assert_eq!(series.validate(), Err(SeriesError::PathExt));
    }

    #[test]
    fn validate_rejects_a_gap_without_both_dates() {
        let mut second = turn(1);
        second.gap_secs = Some(30);
        let series = UsageSeries {
            turns: vec![turn(0), second],
            tool_calls: vec![],
            truncated: false,
        };
        assert_eq!(series.validate(), Err(SeriesError::Gap));
    }

    #[test]
    fn validate_rejects_too_many_rows() {
        let series = UsageSeries {
            turns: (0..=MAX_SERIES_TURNS as u32).map(turn).collect(),
            tool_calls: vec![],
            truncated: true,
        };
        assert_eq!(series.validate(), Err(SeriesError::TooManyRows));
    }

    #[test]
    fn gap_between_is_none_when_either_side_is_undated() {
        let at = Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap();
        let later = at + chrono::Duration::seconds(301);
        assert_eq!(gap_between(Some(at), Some(later)), Some(301));
        assert_eq!(gap_between(None, Some(later)), None);
        assert_eq!(gap_between(Some(at), None), None);
        // A clock that ran backwards is not a gap.
        assert_eq!(gap_between(Some(later), Some(at)), None);
    }

    #[test]
    fn rows_round_trip_with_hex_digests() {
        let mut call = read_call(0);
        call.path_key = Some(KeyedDigest([0xcd; 32]));
        call.path_ext = Some("rs".into());
        let value = serde_json::to_value(&call).unwrap();
        assert_eq!(value["tool"], "read");
        assert_eq!(value["path_key"], hex::encode([0xcd; 32]));
        let back: ToolCallRecord = serde_json::from_value(value).unwrap();
        assert_eq!(back, call);

        let mut t = turn(2);
        t.cache_write = Some(CacheWrite::TotalOnly(8));
        let value = serde_json::to_value(&t).unwrap();
        let back: TurnRecord = serde_json::from_value(value).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn a_short_digest_does_not_deserialize() {
        let bad: Result<KeyedDigest, _> = serde_json::from_value(json!("abcd"));
        assert!(bad.is_err());
    }

    #[test]
    fn in_memory_key_store_keeps_one_key_until_cleared() {
        let store = InMemoryDigestKeyStore::with_seed([9; 32]);
        let first = store.load_or_create().unwrap();
        let second = store.load_or_create().unwrap();
        assert_eq!(msg_key(&first, "x"), msg_key(&second, "x"));
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn in_memory_key_store_load_does_not_create() {
        let store = InMemoryDigestKeyStore::with_seed([9; 32]);
        assert!(store.load().unwrap().is_none());
        store.load_or_create().unwrap();
        assert!(store.load().unwrap().is_some());
    }

    #[test]
    fn in_memory_key_stores_with_different_seeds_differ() {
        let one = InMemoryDigestKeyStore::with_seed([1; 32])
            .load_or_create()
            .unwrap();
        let two = InMemoryDigestKeyStore::with_seed([2; 32])
            .load_or_create()
            .unwrap();
        assert_ne!(msg_key(&one, "x"), msg_key(&two, "x"));
    }
}
