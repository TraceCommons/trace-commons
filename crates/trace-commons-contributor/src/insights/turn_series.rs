//! Claude Code per-turn counter rows, extracted from the exact bytes the user
//! selected and saved with the analysis.
//!
//! One [`TurnRecord`] per API response (a Claude message ID) and one
//! [`ToolCallRecord`] per `tool_use` block. Counters are read only through
//! `source::claude_code::claude_stated_usage`, the same rule the pricing path
//! uses. Tool calls come from the bytes, never from a provider's input.
//!
//! What is kept: counts, sizes, dates, declared model labels, and keyed
//! digests. What is not: bodies, paths, command text, and raw message, tool
//! or session IDs. Those are read into local maps for pairing and
//! deduplication and dropped before return. Anyone holding both a saved
//! store and its digest key can confirm a guessed path or command; a store
//! copied without the key reveals none (owner decision D16, open).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, DigestKey, DigestKeyError, DigestKeyStore, InMemoryDigestKeyStore, KeyedDigest,
    MAX_SERIES_TOOL_CALLS, MAX_SERIES_TURNS, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
    args_key, gap_between, key_fingerprint, msg_key, path_ext, path_key,
};

use super::analytics_constants::DEDUPE_TURNS_BY_MSG_KEY;
use super::analytics_constants::{DIGEST_KEY_CUSTODY, DigestKeyCustody};
use super::models::MAX_DECLARED_MODELS;
use super::usage_evidence::safe_model;
use crate::source::claude_code::claude_stated_usage;

pub const TURN_SERIES_SCHEMA_VERSION: u32 = 1;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// The saved per-turn rows of one Claude Code snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeTurnSeriesEvidence {
    pub schema_version: u32,
    /// Binds the rows to the exact imported bytes.
    pub source_digest: String,
    /// Names the digest key the rows were made under, without revealing it.
    /// Digests under different fingerprints never match each other.
    pub key_fingerprint: KeyedDigest,
    /// Declared model labels, in first-seen order. `model_label_ix` indexes
    /// here. Labels past the bound leave their turns' index `None`.
    pub model_labels: Vec<String>,
    pub series: UsageSeries,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights_turn_series_invalid")
}

impl ClaudeTurnSeriesEvidence {
    pub fn validate(&self) -> Result<()> {
        let mut labels = BTreeSet::new();
        if self.schema_version != TURN_SERIES_SCHEMA_VERSION
            || self.source_digest.len() != 64
            || !self
                .source_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.model_labels.len() > MAX_DECLARED_MODELS
            || !self
                .model_labels
                .iter()
                .all(|label| safe_model(label) && labels.insert(label))
        {
            return Err(invalid());
        }
        self.series.validate().map_err(|_| invalid())?;
        for turn in &self.series.turns {
            if turn
                .model_label_ix
                .is_some_and(|ix| usize::from(ix) >= self.model_labels.len())
                || (!DEDUPE_TURNS_BY_MSG_KEY && turn.msg_key.is_some())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub fn validate_binding(&self, source_digest: &str) -> Result<()> {
        self.validate()?;
        if self.source_digest != source_digest {
            return Err(invalid());
        }
        Ok(())
    }
}

/// One message's counters while its records are read.
struct TurnDraft {
    at: Option<DateTime<Utc>>,
    model: Option<String>,
    counters: Option<[u32; 5]>,
    /// A record of this turn was unstated, regressed, or changed model.
    unknown: bool,
    msg_key: Option<KeyedDigest>,
}

/// Extract the rows. Fails with a fixed label on bytes that are not JSONL
/// records; never includes source content in the error.
pub fn extract_claude_turn_series(
    bytes: &[u8],
    key: &DigestKey,
) -> Result<ClaudeTurnSeriesEvidence> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(invalid());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut drafts: Vec<TurnDraft> = Vec::new();
    let mut calls: Vec<ToolCallRecord> = Vec::new();
    // Local only: message ID -> draft, tool-use ID -> call. Dropped at return.
    let mut turn_by_message: BTreeMap<String, usize> = BTreeMap::new();
    let mut call_by_tool_use: BTreeMap<String, usize> = BTreeMap::new();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).map_err(|_| invalid())?;
        let kind = row
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        match kind {
            "assistant" => {
                let ordinal = assistant_turn(&row, key, &mut drafts, &mut turn_by_message)?;
                if let Some(Value::Array(blocks)) = row.pointer("/message/content") {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                            continue;
                        }
                        let id = block.get("id").and_then(Value::as_str);
                        if id.is_some_and(|id| call_by_tool_use.contains_key(id)) {
                            continue;
                        }
                        let call = tool_call(ordinal, block, key)?;
                        if let Some(id) = id {
                            call_by_tool_use.insert(id.to_owned(), calls.len());
                        }
                        calls.push(call);
                    }
                }
            }
            "user" => {
                let Some(Value::Array(blocks)) = row.pointer("/message/content") else {
                    continue;
                };
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(&index) = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|id| call_by_tool_use.get(id))
                    else {
                        continue;
                    };
                    let call = &mut calls[index];
                    if call.paired {
                        continue;
                    }
                    call.paired = true;
                    call.success = block.get("is_error").and_then(Value::as_bool).map(|e| !e);
                    call.result_bytes = result_bytes(block.get("content"));
                }
            }
            _ => {}
        }
    }

    let mut model_labels: Vec<String> = Vec::new();
    let mut series = UsageSeries::default();
    let mut previous_at = None;
    for (ordinal, draft) in drafts.into_iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| invalid())?;
        let model_label_ix = draft.model.and_then(|model| {
            let ix = match model_labels.iter().position(|label| *label == model) {
                Some(ix) => ix,
                None if model_labels.len() < MAX_DECLARED_MODELS => {
                    model_labels.push(model);
                    model_labels.len() - 1
                }
                None => return None,
            };
            u16::try_from(ix).ok()
        });
        let counters = draft.counters.filter(|_| !draft.unknown);
        series.push_turn(TurnRecord {
            ordinal,
            at: draft.at,
            uncached_input: counters.map(|c| c[0]),
            cache_read: counters.map(|c| c[1]),
            cache_write: counters.map(|c| CacheWrite::Split { m5: c[3], h1: c[4] }),
            output: counters.map(|c| c[2]),
            model_label_ix,
            gap_secs: if ordinal == 0 {
                None
            } else {
                gap_between(previous_at, draft.at)
            },
            msg_key: draft.msg_key,
        });
        previous_at = draft.at;
    }
    // A later record of an earlier message can add a call after a later
    // turn's calls; rows are kept in turn order. The sort is stable.
    calls.sort_by_key(|call| call.turn_ordinal);
    let kept_turns = series.turns.len();
    for call in calls {
        if usize::try_from(call.turn_ordinal).map_err(|_| invalid())? < kept_turns {
            series.push_tool_call(call);
        } else {
            series.truncated = true;
        }
    }
    debug_assert!(series.turns.len() <= MAX_SERIES_TURNS);
    debug_assert!(series.tool_calls.len() <= MAX_SERIES_TOOL_CALLS);

    let evidence = ClaudeTurnSeriesEvidence {
        schema_version: TURN_SERIES_SCHEMA_VERSION,
        source_digest: format!("{:x}", Sha256::digest(bytes)),
        key_fingerprint: key_fingerprint(key),
        model_labels,
        series,
    };
    evidence.validate()?;
    Ok(evidence)
}

/// Fold one assistant record into its message's turn, returning the turn's
/// ordinal. A record with no usable message ID is a turn of its own whose
/// counters are unknown: without the ID it cannot be told from a replay.
fn assistant_turn(
    row: &Value,
    key: &DigestKey,
    drafts: &mut Vec<TurnDraft>,
    turn_by_message: &mut BTreeMap<String, usize>,
) -> Result<u32> {
    let at = row
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let model = row
        .pointer("/message/model")
        .and_then(Value::as_str)
        .filter(|model| safe_model(model))
        .map(str::to_owned);
    let counters = row
        .pointer("/message/usage")
        .and_then(claude_stated_usage)
        .map(|stated| {
            [
                stated.input,
                stated.cache_read,
                stated.output,
                stated.cache_write_5m,
                stated.cache_write_1h,
            ]
        });
    let id = row
        .pointer("/message/id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 256);
    let index = match id.and_then(|id| turn_by_message.get(id).copied()) {
        Some(index) => {
            let draft = &mut drafts[index];
            match (draft.counters, counters) {
                (Some(previous), Some(current))
                    if current
                        .iter()
                        .zip(previous)
                        .all(|(now, before)| *now >= before)
                        && draft.model == model =>
                {
                    draft.counters = Some(current);
                }
                _ => draft.unknown = true,
            }
            index
        }
        None => {
            let index = drafts.len();
            drafts.push(TurnDraft {
                at,
                model,
                counters,
                unknown: id.is_none() || counters.is_none(),
                msg_key: id
                    .filter(|_| DEDUPE_TURNS_BY_MSG_KEY)
                    .map(|id| msg_key(key, id)),
            });
            if let Some(id) = id {
                turn_by_message.insert(id.to_owned(), index);
            }
            index
        }
    };
    u32::try_from(index).map_err(|_| invalid())
}

fn tool_call(turn_ordinal: u32, block: &Value, key: &DigestKey) -> Result<ToolCallRecord> {
    let tool = ToolKind::from_tool_name(block.get("name").and_then(Value::as_str).unwrap_or(""));
    let input = block.get("input").cloned().unwrap_or(Value::Null);
    let file = input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty());
    Ok(ToolCallRecord {
        turn_ordinal,
        tool,
        args_key: args_key(key, tool, &input).map_err(|_| invalid())?,
        path_key: file.map(|path| path_key(key, path)),
        path_ext: file.and_then(path_ext),
        result_bytes: None,
        success: None,
        paired: false,
    })
}

/// The byte length of a tool result's text. Unknown when a block is not text
/// (an image, say), or the content is absent or of another shape.
fn result_bytes(content: Option<&Value>) -> Option<u32> {
    let total = match content? {
        Value::String(text) => text.len(),
        Value::Array(blocks) => {
            let mut total = 0usize;
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("text") {
                    return None;
                }
                total = total.checked_add(block.get("text")?.as_str()?.len())?;
            }
            total
        }
        _ => return None,
    };
    u32::try_from(total).ok()
}

/// The rows to save with a Claude Code snapshot, or `None` when the key store
/// cannot produce a key or the rows cannot be extracted. Never a series under
/// a substitute key: an unknown series counts as `no_usage_counters`.
pub fn saved_turn_series(
    bytes: &[u8],
    keys: &dyn DigestKeyStore,
) -> Option<ClaudeTurnSeriesEvidence> {
    let key = keys.load_or_create().ok()?;
    extract_claude_turn_series(bytes, &key).ok()
}

/// The key store an Insights store uses outside tests, chosen by
/// [`DIGEST_KEY_CUSTODY`] (owner decision D16, open).
pub fn default_digest_key_store(store_dir: PathBuf) -> Box<dyn DigestKeyStore + Send + Sync> {
    if cfg!(test) {
        return Box::new(InMemoryDigestKeyStore::with_seed([0x5a; 32]));
    }
    match DIGEST_KEY_CUSTODY {
        DigestKeyCustody::OsKeychain => Box::new(OsDigestKeyStore),
        DigestKeyCustody::KeyFileInStore => Box::new(FileDigestKeyStore::new(store_dir)),
    }
}

/// A test build never reaches a keychain or writes a key file outside a
/// test's own directory: a past test leak left thousands of orphaned items.
pub(crate) fn refuse_in_test_build() -> Result<(), DigestKeyError> {
    if cfg!(any(test, feature = "test-credential-store")) {
        Err(DigestKeyError::Unavailable(
            "insights_digest_key_test_build",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn new_key_bytes() -> Result<[u8; 32], DigestKeyError> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_random"))?;
    Ok(bytes)
}

pub(crate) fn decode_key(stored: &[u8]) -> Result<DigestKey, DigestKeyError> {
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(stored, &mut bytes)
        .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_invalid"))?;
    Ok(DigestKey::from_bytes(bytes))
}

/// The digest key in the OS keychain: macOS Keychain, Windows Credential
/// Manager, Secret Service on Linux. One fixed entry per OS user.
#[derive(Debug, Clone, Copy)]
pub struct OsDigestKeyStore;

impl OsDigestKeyStore {
    fn backend() -> Result<crate::daemon::os_secret_store::OsSecretBackend, DigestKeyError> {
        refuse_in_test_build()?;
        crate::daemon::os_secret_store::OsSecretBackend::insights_digest_key()
            .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_keychain"))
    }
}

impl DigestKeyStore for OsDigestKeyStore {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        use crate::daemon::credential_store::CredentialError;
        match Self::backend()?.read_insights_digest_key() {
            Ok(stored) => decode_key(&stored).map(Some),
            Err(CredentialError::NoEntry) => Ok(None),
            Err(_) => Err(DigestKeyError::Unavailable("insights_digest_key_keychain")),
        }
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        if let Some(key) = self.load()? {
            return Ok(key);
        }
        let bytes = new_key_bytes()?;
        Self::backend()?
            .write_insights_digest_key(hex::encode(bytes).as_bytes())
            .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_keychain"))?;
        // Read back: a concurrent creator may have written its key last, and
        // the stored one is the key every later digest is made under.
        self.load()?
            .ok_or(DigestKeyError::Unavailable("insights_digest_key_keychain"))
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        use crate::daemon::credential_store::CredentialError;
        match Self::backend()?.delete_insights_digest_key() {
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(_) => Err(DigestKeyError::Unavailable("insights_digest_key_keychain")),
        }
    }
}

/// The alternative custody under D16: a 0600 key file inside the store
/// directory. Weaker, since a copied store directory carries its key. It
/// touches only the directory it is given, so tests exercise it in a
/// temporary directory.
#[derive(Debug, Clone)]
pub struct FileDigestKeyStore {
    dir: PathBuf,
}

impl FileDigestKeyStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

const KEY_FILE_NAME: &str = "digest-key";

impl DigestKeyStore for FileDigestKeyStore {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        let path = self.dir.join(KEY_FILE_NAME);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(DigestKeyError::Unavailable("insights_digest_key_file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DigestKeyError::Unavailable("insights_digest_key_file")),
        }
        let stored = std::fs::read(&path)
            .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_file"))?;
        decode_key(&stored).map(Some)
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        if let Some(key) = self.load()? {
            return Ok(key);
        }
        let bytes = new_key_bytes()?;
        crate::config::write_atomic_0600(
            &self.dir,
            &self.dir.join(KEY_FILE_NAME),
            hex::encode(bytes).as_bytes(),
        )
        .map_err(|_| DigestKeyError::Unavailable("insights_digest_key_file"))?;
        self.load()?
            .ok_or(DigestKeyError::Unavailable("insights_digest_key_file"))
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        match std::fs::remove_file(self.dir.join(KEY_FILE_NAME)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(DigestKeyError::Unavailable("insights_digest_key_file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use trace_commons_protocol::insights_usage_series::{
        CacheWrite, DigestKeyStore, InMemoryDigestKeyStore, ToolKind, key_fingerprint, msg_key,
        path_key,
    };

    fn fixture() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl"),
        )
        .unwrap()
    }

    fn key() -> DigestKey {
        InMemoryDigestKeyStore::with_seed([7; 32])
            .load_or_create()
            .unwrap()
    }

    fn jsonl(rows: &[Value]) -> Vec<u8> {
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    #[test]
    fn fixture_session_yields_one_turn_per_message_with_stated_counters() {
        let bytes = fixture();
        let evidence = extract_claude_turn_series(&bytes, &key()).unwrap();
        evidence.validate().unwrap();
        evidence
            .validate_binding(&format!("{:x}", Sha256::digest(&bytes)))
            .unwrap();
        assert_eq!(evidence.schema_version, TURN_SERIES_SCHEMA_VERSION);
        assert_eq!(evidence.key_fingerprint, key_fingerprint(&key()));
        assert_eq!(
            evidence.model_labels,
            ["claude-fixture-model", "claude-other-model"]
        );
        let turns = &evidence.series.turns;
        assert_eq!(turns.len(), 6);
        assert!(!evidence.series.truncated);
        assert_eq!(
            turns.iter().map(|t| t.ordinal).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );

        // The replayed record of the first message keeps its latest counters
        // and its first timestamp.
        let first = &turns[0];
        assert_eq!(first.uncached_input, Some(3));
        assert_eq!(first.cache_read, Some(0));
        assert_eq!(
            first.cache_write,
            Some(CacheWrite::Split { m5: 12_000, h1: 0 })
        );
        assert_eq!(first.output, Some(40));
        assert_eq!(first.at.unwrap().to_rfc3339(), "2026-09-14T09:00:02+00:00");
        assert_eq!(first.gap_secs, None);
        assert_eq!(first.model_label_ix, Some(0));
        assert_eq!(first.msg_key, Some(msg_key(&key(), "msg_PRIVATE_01")));

        assert_eq!(
            turns[1].cache_write,
            Some(CacheWrite::Split { m5: 300, h1: 200 })
        );
        assert_eq!(turns[1].gap_secs, Some(8));
        assert_eq!(turns[2].gap_secs, Some(350));

        // An older record with writes but no split, and a priority-tier
        // record: unknown, never zero.
        for unknown in [&turns[3], &turns[4]] {
            assert!(unknown.usage_unknown());
            assert_eq!(unknown.uncached_input, None);
            assert_eq!(unknown.cache_write, None);
            assert!(unknown.at.is_some());
        }
        // An undated turn has no gap on either side.
        assert_eq!(turns[5].at, None);
        assert_eq!(turns[5].gap_secs, None);
        assert_eq!(turns[5].model_label_ix, Some(1));
        assert_eq!(turns[5].known_usage().unwrap().total(), 4 + 12_600 + 30);
    }

    #[test]
    fn fixture_tool_calls_are_digests_and_sizes_only() {
        let evidence = extract_claude_turn_series(&fixture(), &key()).unwrap();
        let calls = &evidence.series.tool_calls;
        assert_eq!(
            calls
                .iter()
                .map(|c| (c.turn_ordinal, c.tool))
                .collect::<Vec<_>>(),
            [
                (0, ToolKind::Read),
                (1, ToolKind::Edit),
                (2, ToolKind::Bash),
                (3, ToolKind::Other),
                (4, ToolKind::Grep),
            ]
        );
        let file = path_key(
            &key(),
            "/Users/PRIVATE_USER/code/PRIVATE_REPO/src/PRIVATE_FILE.rs",
        );
        assert_eq!(calls[0].path_key, Some(file));
        assert_eq!(calls[1].path_key, Some(file));
        assert_eq!(calls[0].path_ext.as_deref(), Some("rs"));
        assert_eq!(calls[2].path_key, None);
        assert_eq!(calls[4].path_key, None, "only file_path is a file");

        assert_eq!(
            calls[0].result_bytes,
            Some("PRIVATE_FILE_CONTENTS fn main() {}".len() as u32)
        );
        assert_eq!(calls[1].result_bytes, Some("PRIVATE_EDIT_OK".len() as u32));
        assert_eq!(calls[0].success, None, "no is_error is not a verdict");
        assert_eq!(calls[2].success, Some(false));
        assert!(calls[..3].iter().all(|c| c.paired));
        assert!(!calls[3].paired && !calls[4].paired);
        assert_eq!(calls[3].result_bytes, None);
        assert_ne!(calls[0].args_key, calls[1].args_key);
    }

    #[test]
    fn serialized_series_holds_no_readable_private_text() {
        let evidence = extract_claude_turn_series(&fixture(), &key()).unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(!json.contains("PRIVATE"), "{json}");
        assert!(!json.contains("/Users"));
        assert!(!json.contains("cargo"));
        assert!(!json.contains("toolu_"));
        // The field is named `msg_key`; the ID itself never appears.
        assert!(!json.contains("msg_PRIVATE"));
    }

    /// The parity fixture: every assistant record, alone, through both the
    /// adapter's pricing path (`served_by`) and this extractor. Both must
    /// agree that it is known, and on the cache counters when it is.
    #[test]
    fn both_claude_extractors_agree_record_by_record() {
        let bytes = fixture();
        let text = std::str::from_utf8(&bytes).unwrap();
        let mut compared = 0;
        let mut unknown = 0;
        for line in text.lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            if row["type"] != "assistant" {
                continue;
            }
            let single = format!("{line}\n");
            let events =
                crate::source::claude_code::parse_selected_file_bytes(single.as_bytes()).unwrap();
            let served = events.iter().find_map(|event| event.served_by.clone());
            let series = extract_claude_turn_series(single.as_bytes(), &key()).unwrap();
            let turn = &series.series.turns[0];
            match (&served, turn.known_usage()) {
                (Some(served), Some(usage)) => {
                    assert_eq!(usage.cache_read, served.cache_read_tokens);
                    assert_eq!(
                        usage.cache_write,
                        CacheWrite::Split {
                            m5: served.cache_write_5m_tokens,
                            h1: served.cache_write_1h_tokens
                        }
                    );
                }
                (None, None) => unknown += 1,
                (served, usage) => panic!("extractors disagree: {served:?} vs {usage:?}"),
            }
            compared += 1;
        }
        assert_eq!(compared, 7);
        assert_eq!(unknown, 2, "the no-split and priority records");
    }

    #[test]
    fn a_message_whose_repeat_regresses_is_unknown() {
        let usage = |output: u64| {
            json!({"input_tokens":1,"output_tokens":output,"cache_read_input_tokens":0,
                "cache_creation_input_tokens":0})
        };
        let record = |output: u64| {
            json!({"type":"assistant","message":{"id":"m","model":"claude-x",
                "content":[],"usage":usage(output)}})
        };
        let evidence = extract_claude_turn_series(&jsonl(&[record(5), record(3)]), &key()).unwrap();
        assert_eq!(evidence.series.turns.len(), 1);
        assert!(evidence.series.turns[0].usage_unknown());
        let rising = extract_claude_turn_series(&jsonl(&[record(3), record(5)]), &key()).unwrap();
        assert_eq!(rising.series.turns[0].output, Some(5));
    }

    #[test]
    fn a_record_without_a_message_id_is_its_own_unknown_turn() {
        let evidence = extract_claude_turn_series(
            &jsonl(&[
                json!({"type":"assistant","message":{"model":"claude-x","content":[
                {"type":"tool_use","id":"t","name":"Read","input":{"file_path":"a.md"}}],
                "usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,
                    "cache_creation_input_tokens":0}}}),
            ]),
            &key(),
        )
        .unwrap();
        let turn = &evidence.series.turns[0];
        assert!(turn.usage_unknown());
        assert_eq!(turn.msg_key, None);
        assert_eq!(evidence.series.tool_calls.len(), 1);
    }

    #[test]
    fn rows_past_the_caps_mark_the_series_truncated() {
        let mut rows = Vec::new();
        for i in 0..(MAX_SERIES_TURNS + 2) {
            rows.push(json!({"type":"assistant","message":{"id":format!("m{i}"),
                "content":[{"type":"tool_use","id":format!("t{i}"),"name":"Glob","input":{}}],
                "usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,
                    "cache_creation_input_tokens":0}}}));
        }
        let evidence = extract_claude_turn_series(&jsonl(&rows), &key()).unwrap();
        assert!(evidence.series.truncated);
        assert_eq!(evidence.series.turns.len(), MAX_SERIES_TURNS);
        assert_eq!(evidence.series.tool_calls.len(), MAX_SERIES_TURNS);
        evidence.validate().unwrap();
    }

    #[test]
    fn validation_rejects_rows_a_reader_cannot_trust() {
        let valid = extract_claude_turn_series(&fixture(), &key()).unwrap();

        let mut schema = valid.clone();
        schema.schema_version = TURN_SERIES_SCHEMA_VERSION + 1;
        assert!(schema.validate().is_err());

        let mut label = valid.clone();
        label.series.turns[0].model_label_ix = Some(9);
        assert!(label.validate().is_err());

        let mut unsafe_label = valid.clone();
        unsafe_label.model_labels[0] = "/Users/someone".into();
        assert!(unsafe_label.validate().is_err());

        let mut order = valid.clone();
        order.series.turns.swap(0, 1);
        assert!(order.validate().is_err());

        let mut digest = valid.clone();
        digest.source_digest = "not-a-digest".into();
        assert!(digest.validate().is_err());

        assert!(valid.validate_binding(&"0".repeat(64)).is_err());
    }

    #[test]
    fn unparseable_bytes_fail_with_a_fixed_label() {
        assert_eq!(
            extract_claude_turn_series(b"{PRIVATE", &key())
                .unwrap_err()
                .to_string(),
            "insights_turn_series_invalid"
        );
    }

    /// Custody is the host's (D16). A store that cannot produce a key yields
    /// no series, never a series under some other key.
    #[test]
    fn an_unavailable_key_store_yields_no_series() {
        struct Refusing;
        impl DigestKeyStore for Refusing {
            fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
                Err(DigestKeyError::Unavailable("refusing"))
            }
            fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
                Err(DigestKeyError::Unavailable("refusing"))
            }
            fn clear(&self) -> Result<(), DigestKeyError> {
                Ok(())
            }
        }
        assert!(saved_turn_series(&fixture(), &Refusing).is_none());
        assert!(
            saved_turn_series(&fixture(), &InMemoryDigestKeyStore::with_seed([7; 32])).is_some()
        );
    }

    #[test]
    fn the_key_file_store_creates_once_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileDigestKeyStore::new(dir.path().to_path_buf());
        assert!(store.load().unwrap().is_none());
        let created = store.load_or_create().unwrap();
        let again = store.load_or_create().unwrap();
        assert_eq!(key_fingerprint(&created), key_fingerprint(&again));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
        store.clear().unwrap();
        std::fs::write(dir.path().join(KEY_FILE_NAME), "not hex").unwrap();
        assert!(store.load().is_err());
    }

    /// Under test the OS-backed store refuses, so no test can create a real
    /// keychain item.
    #[test]
    fn the_os_key_store_refuses_under_test() {
        assert!(matches!(
            OsDigestKeyStore.load_or_create(),
            Err(DigestKeyError::Unavailable(_))
        ));
        assert!(matches!(
            OsDigestKeyStore.load(),
            Err(DigestKeyError::Unavailable(_))
        ));
    }
}
